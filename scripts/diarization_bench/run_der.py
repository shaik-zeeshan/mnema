#!/usr/bin/env python3
"""Benchmark Mnema's speaker diarization against VoxConverse using DER.

Pipeline per clip:
  1. Pull the clip + ground-truth speaker turns from the HuggingFace dataset
     `diarizers-community/voxconverse` (already split, timestamped).
  2. Write the audio to a temp WAV and build the reference annotation.
  3. Run the Rust `diarize_to_rttm` binary (real sherpa-onnx provider) to get a
     hypothesis RTTM, then parse it back.
  4. Score with pyannote.metrics DER, with a 0.25s collar, reported both
     including and excluding overlapped speech.

DER = (false alarm + missed detection + speaker confusion) / total reference
speech. The miss/false-alarm vs confusion split tells you whether to look at the
pyannote *segmentation* stage (speech detection / boundaries) or at the
*clustering/embedding* stage (who-is-who).

See README.md for setup. Example:
    python run_der.py --limit 8                # fast loop on first 8 test clips
    python run_der.py --manifest voxconverse_subset.txt
    python run_der.py --all --json-out baseline.json
"""

from __future__ import annotations

import argparse
import io
import json
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DATASET_ID = "diarizers-community/voxconverse"
# Pin to an immutable dataset commit for reproducibility; override with
# --revision for ad-hoc runs. "main" is a moving branch and would let DER
# baselines drift as the upstream dataset changes. Sample order (and therefore
# the indices in voxconverse_subset.txt) is stable for a given revision. This
# SHA is `main` as of 2024-05-31, the dataset's last upstream change.
DEFAULT_REVISION = "3acfa1b45ca4b7419aee999d67d94c617f9c9d47"


def find_binary(explicit: str | None) -> Path:
    if explicit:
        p = Path(explicit)
        if not p.is_file():
            sys.exit(f"--binary not found: {p}")
        return p
    for profile in ("release", "debug"):
        cand = REPO_ROOT / "target" / profile / "diarize_to_rttm"
        if cand.is_file():
            return cand
    sys.exit(
        "diarize_to_rttm binary not found. Build it first:\n"
        "  cargo build -p speaker-analysis --features sherpa-onnx --release "
        "--bin diarize_to_rttm\n"
        "or pass --binary <path>."
    )


def wanted_indices(args: argparse.Namespace) -> "set[int] | None":
    """Stream positions to evaluate; None means every clip in the split."""
    if args.manifest:
        indices: set[int] = set()
        for raw in Path(args.manifest).read_text().splitlines():
            line = raw.split("#", 1)[0].strip()
            if line:
                indices.add(int(line))
        return indices
    if args.all or args.limit <= 0:
        return None  # None => every clip in the (streamed) split
    return set(range(args.limit))


@dataclass
class Components:
    confusion: float = 0.0
    missed: float = 0.0
    false_alarm: float = 0.0
    total: float = 0.0

    def add(self, detailed: dict) -> None:
        self.confusion += detailed["confusion"]
        self.missed += detailed["missed detection"]
        self.false_alarm += detailed["false alarm"]
        self.total += detailed["total"]

    def der(self) -> float:
        if self.total <= 0:
            return 0.0
        return (self.confusion + self.missed + self.false_alarm) / self.total

    def as_dict(self) -> dict:
        return {
            "der": self.der(),
            "confusion": self.confusion,
            "missed_detection": self.missed,
            "false_alarm": self.false_alarm,
            "total_speech": self.total,
        }


@dataclass
class SpeakerCountStats:
    """Aggregates speaker-count error across clips.

    DER alone hides over-/under-clustering: a run can land a low DER while
    predicting the wrong number of speakers (e.g. splitting one speaker into
    several near-identical clusters). This tracks, per clip, the signed count
    error (predicted - reference) and rolls it up so experiments can report
    whether a change pushes the pipeline toward over- or under-clustering.

    Reusable by later experiments (e.g. NME-SC): feed each clip's
    (reference, hypothesis) speaker counts via `add` and read `as_dict`.
    """

    errors: list[int] = field(default_factory=list)  # signed: pred - ref, per clip

    def add(self, reference_speakers: int, hypothesis_speakers: int) -> int:
        """Record one clip; returns the signed count error (pred - ref)."""
        error = hypothesis_speakers - reference_speakers
        self.errors.append(error)
        return error

    def as_dict(self) -> dict:
        n = len(self.errors)
        if n == 0:
            return {
                "clips": 0,
                "mean_signed_error": 0.0,
                "mean_abs_error": 0.0,
                "over_count": 0,
                "under_count": 0,
                "exact_count": 0,
                "pct_over_estimate": 0.0,
                "pct_under_estimate": 0.0,
                "pct_exact": 0.0,
            }
        over = sum(1 for e in self.errors if e > 0)
        under = sum(1 for e in self.errors if e < 0)
        exact = sum(1 for e in self.errors if e == 0)
        return {
            "clips": n,
            # mean signed error: sign tells direction (positive => over-clusters).
            "mean_signed_error": sum(self.errors) / n,
            # mean absolute error: magnitude of miscounting regardless of sign.
            "mean_abs_error": sum(abs(e) for e in self.errors) / n,
            "over_count": over,
            "under_count": under,
            "exact_count": exact,
            "pct_over_estimate": over / n * 100.0,
            "pct_under_estimate": under / n * 100.0,
            "pct_exact": exact / n * 100.0,
        }


def build_reference(sample, uri: str):
    from pyannote.core import Annotation, Segment

    ann = Annotation(uri=uri)
    starts = sample["timestamps_start"]
    ends = sample["timestamps_end"]
    speakers = sample["speakers"]
    for i, (start, end, spk) in enumerate(zip(starts, ends, speakers)):
        if end > start:
            ann[Segment(float(start), float(end)), i] = str(spk)
    return ann


def parse_rttm(text: str, uri: str):
    from pyannote.core import Annotation, Segment

    ann = Annotation(uri=uri)
    for i, line in enumerate(text.splitlines()):
        parts = line.split()
        if len(parts) < 8 or parts[0] != "SPEAKER":
            continue
        onset = float(parts[3])
        duration = float(parts[4])
        speaker = parts[7]
        if duration > 0:
            ann[Segment(onset, onset + duration), i] = speaker
    return ann


def diarizer_extra_args(args: argparse.Namespace) -> list[str]:
    extra: list[str] = []
    if args.model_id:
        extra += ["--model-id", args.model_id]
    if args.clustering_threshold is not None:
        extra += ["--clustering-threshold", str(args.clustering_threshold)]
    if args.cross_chunk_threshold is not None:
        extra += ["--cross-chunk-threshold", str(args.cross_chunk_threshold)]
    if args.num_clusters is not None:
        extra += ["--num-clusters", str(args.num_clusters)]
    if args.min_duration_on is not None:
        extra += ["--min-duration-on", str(args.min_duration_on)]
    if args.min_duration_off is not None:
        extra += ["--min-duration-off", str(args.min_duration_off)]
    if args.safe_chunk_ms is not None:
        extra += ["--safe-chunk-ms", str(args.safe_chunk_ms)]
    return extra


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--split", default="test", help="dataset split (default: test)")
    parser.add_argument("--revision", default=DEFAULT_REVISION, help="dataset revision to pin")
    parser.add_argument("--limit", type=int, default=8, help="first N clips (default: 8; <=0 means all)")
    parser.add_argument("--all", action="store_true", help="evaluate the whole split")
    parser.add_argument("--manifest", help="file of clip indices (one per line, # comments ok)")
    parser.add_argument("--binary", help="path to diarize_to_rttm (default: target/{release,debug})")
    parser.add_argument("--collar", type=float, default=0.25, help="DER forgiveness collar in seconds")
    parser.add_argument("--models-dir", help="speaker-analysis model store (passed to the binary)")
    parser.add_argument("--model-id", help="preset id (passed to the binary)")
    parser.add_argument("--clustering-threshold", type=float)
    parser.add_argument("--cross-chunk-threshold", type=float)
    parser.add_argument("--num-clusters", type=int)
    parser.add_argument("--min-duration-on", type=float)
    parser.add_argument("--min-duration-off", type=float)
    parser.add_argument(
        "--safe-chunk-ms",
        type=int,
        help="safe-chunk diarization window override in ms (default 60000, clamped <=60000)",
    )
    parser.add_argument("--work-dir", help="keep exported WAV/RTTM here instead of a temp dir")
    parser.add_argument("--json-out", help="write aggregate + per-file results as JSON")
    args = parser.parse_args()
    sys.stdout.reconfigure(line_buffering=True)  # flush per-clip rows on long runs

    try:
        import soundfile as sf
        from datasets import Audio, load_dataset
        from pyannote.metrics.diarization import DiarizationErrorRate
    except ImportError as exc:
        sys.exit(f"missing dependency: {exc}. Run: pip install -r requirements.txt")

    binary = find_binary(args.binary)

    # Stream the split: read parquet shards lazily, no multi-GB Arrow cache on
    # disk, and stop once the requested clips are processed.
    print(f"streaming {DATASET_ID} split={args.split} revision={args.revision} ...", file=sys.stderr)
    ds = load_dataset(DATASET_ID, split=args.split, revision=args.revision, streaming=True)
    # Keep the raw encoded audio bytes instead of letting `datasets` decode them
    # (newer versions require the heavy `torchcodec` for that). We decode the
    # bytes ourselves with soundfile below.
    ds = ds.cast_column("audio", Audio(decode=False))
    wanted = wanted_indices(args)
    max_idx = max(wanted) if wanted else None
    print(
        f"evaluating {'all' if wanted is None else len(wanted)} clips"
        f"{'' if wanted is None else f' (indices up to {max_idx})'}",
        file=sys.stderr,
    )

    metric_overlap = DiarizationErrorRate(collar=args.collar, skip_overlap=False)
    metric_no_overlap = DiarizationErrorRate(collar=args.collar, skip_overlap=True)
    agg_overlap = Components()
    agg_no_overlap = Components()
    count_stats = SpeakerCountStats()
    per_file: list[dict] = []

    work_ctx = (
        tempfile.TemporaryDirectory()
        if not args.work_dir
        else _DirHolder(Path(args.work_dir))
    )
    with work_ctx as work_dir:
        work_dir = Path(work_dir)
        extra = diarizer_extra_args(args)
        header = (
            f"{'uri':<28} {'ref':>3} {'hyp':>3} {'cnt±':>5} "
            f"{'DER%':>7} {'conf%':>7} {'miss%':>7} {'FA%':>7}"
        )
        print(header)
        print("-" * len(header))

        for idx, sample in enumerate(ds):
            if wanted is not None and idx not in wanted:
                if max_idx is not None and idx > max_idx:
                    break
                continue
            uri = f"{args.split}-{idx:04d}"
            wav_path = work_dir / f"{uri}.wav"
            audio = sample["audio"]
            raw = audio.get("bytes")
            if raw is None and audio.get("path"):
                raw = Path(audio["path"]).read_bytes()
            data, sr = sf.read(io.BytesIO(raw))
            sf.write(wav_path, data, sr)

            reference = build_reference(sample, uri)
            n_speakers = len(set(reference.labels()))

            cmd = [str(binary), "--audio", str(wav_path), "--uri", uri]
            if args.models_dir:
                cmd += ["--models-dir", args.models_dir]
            cmd += extra
            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode != 0:
                print(f"{uri:<28} FAILED: {result.stderr.strip().splitlines()[-1:]}", file=sys.stderr)
                continue
            if args.work_dir:
                (work_dir / f"{uri}.hyp.rttm").write_text(result.stdout)

            if not args.work_dir:
                wav_path.unlink(missing_ok=True)  # bound peak disk use

            hypothesis = parse_rttm(result.stdout, uri)
            hyp_speakers = len(set(hypothesis.labels()))
            count_error = count_stats.add(n_speakers, hyp_speakers)  # signed: hyp - ref

            det_o = metric_overlap(reference, hypothesis, detailed=True)
            det_n = metric_no_overlap(reference, hypothesis, detailed=True)
            agg_overlap.add(det_o)
            agg_no_overlap.add(det_n)

            total_o = det_o["total"] or 1.0
            der_o = (det_o["confusion"] + det_o["missed detection"] + det_o["false alarm"]) / total_o
            conf = det_o["confusion"] / total_o
            miss = det_o["missed detection"] / total_o
            fa = det_o["false alarm"] / total_o
            print(
                f"{uri:<28} {n_speakers:>3} {hyp_speakers:>3} {count_error:>+5d} "
                f"{der_o * 100:>7.2f} {conf * 100:>7.2f} {miss * 100:>7.2f} {fa * 100:>7.2f}"
            )
            per_file.append(
                {
                    "uri": uri,
                    "index": idx,
                    "reference_speakers": n_speakers,
                    "hypothesis_speakers": hyp_speakers,
                    "speaker_count_error": count_error,  # signed: hyp - ref
                    "with_overlap": {
                        "der": der_o,
                        "confusion": conf,
                        "missed_detection": miss,
                        "false_alarm": fa,
                    },
                }
            )

    print("-" * len(header))
    if not per_file:
        sys.exit("no clips were successfully evaluated (see warnings above)")
    o = agg_overlap.as_dict()
    n = agg_no_overlap.as_dict()
    print(
        f"AGGREGATE (collar={args.collar}s, incl. overlap): "
        f"DER={o['der'] * 100:.2f}%  conf={o['confusion'] / max(o['total_speech'], 1) * 100:.2f}%  "
        f"miss={o['missed_detection'] / max(o['total_speech'], 1) * 100:.2f}%  "
        f"FA={o['false_alarm'] / max(o['total_speech'], 1) * 100:.2f}%"
    )
    print(f"AGGREGATE (collar={args.collar}s, excl. overlap): DER={n['der'] * 100:.2f}%")

    c = count_stats.as_dict()
    print(
        "SPEAKER COUNT: "
        f"mean_abs_err={c['mean_abs_error']:.2f}  "
        f"mean_signed_err={c['mean_signed_error']:+.2f}  "
        f"over={c['pct_over_estimate']:.0f}% ({c['over_count']}/{c['clips']})  "
        f"under={c['pct_under_estimate']:.0f}% ({c['under_count']}/{c['clips']})  "
        f"exact={c['pct_exact']:.0f}% ({c['exact_count']}/{c['clips']})"
    )

    if args.json_out:
        payload = {
            "dataset": DATASET_ID,
            "split": args.split,
            "revision": args.revision,
            "collar": args.collar,
            "clips": len(per_file),
            "config": {
                "model_id": args.model_id,
                "clustering_threshold": args.clustering_threshold,
                "cross_chunk_threshold": args.cross_chunk_threshold,
                "num_clusters": args.num_clusters,
                "min_duration_on": args.min_duration_on,
                "min_duration_off": args.min_duration_off,
                "safe_chunk_ms": args.safe_chunk_ms,
            },
            "aggregate": {"with_overlap": o, "without_overlap": n},
            "speaker_count": c,
            "per_file": per_file,
        }
        Path(args.json_out).write_text(json.dumps(payload, indent=2))
        print(f"wrote {args.json_out}", file=sys.stderr)

    return 0


class _DirHolder:
    """Context-manager shim so --work-dir behaves like TemporaryDirectory."""

    def __init__(self, path: Path):
        self.path = path

    def __enter__(self) -> str:
        self.path.mkdir(parents=True, exist_ok=True)
        return str(self.path)

    def __exit__(self, *exc) -> None:
        return None


if __name__ == "__main__":
    raise SystemExit(main())
