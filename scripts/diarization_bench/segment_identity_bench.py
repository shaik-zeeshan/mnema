#!/usr/bin/env python3
"""Measure whether a speaker keeps their identity ACROSS Mnema's capture segments.

`run_der.py` answers "did we separate the voices inside this file". This answers
the different question behind the "merge with Unknown Speaker 2, 3, 5…" and
"I named someone and it didn't stick" reports: **did we recognise the same voice
again in the next segment, and in the next recording**. A run can score DER 0%
and still mint one Unknown Speaker per segment, because cross-segment identity is
decided by `resolve_stable_speaker_cluster` in `app-infra`, which DER never
touches.

Two stages, split because embedding audio is the expensive part and it does not
change when you tune a threshold:

  stage 1 (this script) — slice a VoxConverse clip into fixed-length segments,
      run the REAL speakrs provider on each via `diarize_to_rttm_speakrs
      --dump-clusters`, label every emitted cluster with the ground-truth speaker
      it mostly covers, and write one dump JSON per (clip, chunk size).

  stage 2 (`replay_speaker_identity`) — replay those centroids through the REAL
      shipped resolver and report clusters minted, merge suggestions raised,
      wrong auto-merges, and assignment stickiness. Free to re-run at any
      threshold; no CoreML involved.

So a chunk-size sweep costs one CoreML pass per size, and a threshold sweep costs
nothing.

Example:
    # dump embeddings for 4 three-speaker clips at each chunk size
    python segment_identity_bench.py --chunk-seconds 10,30,60,180,300 --clips 4

    # then sweep resolution rules for free
    python segment_identity_bench.py --replay-only --sweep

Prerequisites are `run_der.py`'s, plus the app-infra replay binary:
    cargo build -p app-infra --release --bin replay_speaker_identity
"""

from __future__ import annotations

import argparse
import io
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DATASET_ID = "diarizers-community/voxconverse"
# Same pinned revision as run_der.py, so clip indices mean the same thing in both
# harnesses and numbers stay comparable across runs.
DEFAULT_REVISION = "3acfa1b45ca4b7419aee999d67d94c617f9c9d47"
DEFAULT_CHUNK_SECONDS = "10,30,60,180,300"


def find_binary(name: str, explicit: str | None) -> Path:
    if explicit:
        path = Path(explicit)
        if not path.is_file():
            sys.exit(f"binary not found: {path}")
        return path
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / name
        if candidate.is_file():
            return candidate
    # The speakrs bin builds OpenBLAS from source, which needs the gcc lib dir on
    # LIBRARY_PATH or it dies at its own test link (AGENTS.md).
    prefix = ". scripts/openblas-build-env.sh && " if "diarize" in name else ""
    sys.exit(
        f"{name} not found. Build it first (from the repo root):\n"
        f"  {prefix}cargo build -p "
        f"{'speaker-analysis --features speakrs' if 'diarize' in name else 'app-infra'} "
        f"--release --bin {name}"
    )


def reference_turns(sample) -> list[tuple[float, float, str]]:
    """Ground-truth (start_s, end_s, speaker) triples for one clip."""
    return [
        (float(start), float(end), str(speaker))
        for start, end, speaker in zip(
            sample["timestamps_start"], sample["timestamps_end"], sample["speakers"]
        )
        if float(end) > float(start)
    ]


def dominant_true_speaker(
    turns: list[dict], offset_s: float, reference: list[tuple[float, float, str]]
) -> str | None:
    """The reference speaker a hypothesis cluster spends the most time inside.

    `turns` are the cluster's turns within one segment, in segment-relative ms;
    `offset_s` shifts them back onto the clip's timeline so they can be compared
    with the clip-level reference. A cluster that overlaps no reference speech at
    all (pure false alarm) gets None and is excluded from correctness scoring
    rather than silently counted as correct.
    """
    totals: dict[str, float] = {}
    for turn in turns:
        start = offset_s + turn["startMs"] / 1000.0
        end = offset_s + turn["endMs"] / 1000.0
        for ref_start, ref_end, speaker in reference:
            overlap = min(end, ref_end) - max(start, ref_start)
            if overlap > 0:
                totals[speaker] = totals.get(speaker, 0.0) + overlap
    if not totals:
        return None
    return max(totals.items(), key=lambda item: item[1])[0]


def dump_clip(
    binary: Path,
    models_dir: str | None,
    uri: str,
    data,
    sample_rate: int,
    reference: list[tuple[float, float, str]],
    chunk_seconds: float,
    work_dir: Path,
    out_path: Path,
) -> dict:
    """Slice one clip, diarize each slice, and write the labelled cluster dump."""
    import soundfile as sf

    chunk_samples = int(chunk_seconds * sample_rate)
    total_samples = len(data)
    segments = []

    for index, start in enumerate(range(0, total_samples, chunk_samples)):
        end = min(start + chunk_samples, total_samples)
        # A sliver of a trailing segment carries no usable speech and would just
        # add a spurious cluster; the app's real segments are whole too.
        if (end - start) < sample_rate:
            break

        slice_path = work_dir / f"{uri}-{chunk_seconds:g}s-{index:04d}.wav"
        dump_path = work_dir / f"{uri}-{chunk_seconds:g}s-{index:04d}.json"
        sf.write(slice_path, data[start:end], sample_rate)

        cmd = [
            str(binary),
            "--audio",
            str(slice_path),
            "--uri",
            f"{uri}-{index:04d}",
            "--dump-clusters",
            str(dump_path),
            "--out",
            str(work_dir / "discard.rttm"),
        ]
        if models_dir:
            cmd += ["--models-dir", models_dir]
        result = subprocess.run(cmd, capture_output=True, text=True)
        slice_path.unlink(missing_ok=True)
        if result.returncode != 0:
            tail = result.stderr.strip().splitlines()[-1:] or ["(no stderr)"]
            print(f"  segment {index}: FAILED {tail[0]}", file=sys.stderr)
            continue

        emitted = json.loads(dump_path.read_text())
        dump_path.unlink(missing_ok=True)
        offset_s = start / sample_rate

        clusters = []
        for cluster in emitted["clusters"]:
            cluster_turns = [
                turn
                for turn in emitted["turns"]
                if turn["providerClusterId"] == cluster["providerClusterId"]
            ]
            clusters.append(
                {
                    "embedding": cluster["embedding"],
                    "speechMs": cluster["speechMs"],
                    "trueSpeaker": dominant_true_speaker(
                        cluster_turns, offset_s, reference
                    ),
                }
            )

        segments.append({"index": index, "clusters": clusters})

    payload = {
        "clip": uri,
        "chunkSeconds": chunk_seconds,
        "trueSpeakers": len({speaker for _, _, speaker in reference}),
        "segments": segments,
    }
    out_path.write_text(json.dumps(payload))
    return payload


# Configurations the sweep replays. Each is (label, extra replay args). They
# isolate one change at a time so a win is attributable, then stack the winners.
SWEEP = [
    ("shipped", []),
    ("F4 reaverage", ["--centroid", "reaverage"]),
    ("F3 person-aware", ["--person-aware-ambiguity"]),
    ("F3+F4", ["--centroid", "reaverage", "--person-aware-ambiguity"]),
    (
        "F3+F4+steer",
        ["--centroid", "reaverage", "--person-aware-ambiguity", "--recognition-steers"],
    ),
    (
        "F3+F4+steer+F6@0.70",
        [
            "--centroid",
            "reaverage",
            "--person-aware-ambiguity",
            "--recognition-steers",
            "--auto-reuse",
            "0.70",
        ],
    ),
]


def replay(replay_binary: Path, dump: Path, mode: str, extra: list[str]) -> dict:
    out = dump.with_suffix(".metrics.json")
    cmd = [
        str(replay_binary),
        "--dump",
        str(dump),
        "--mode",
        mode,
        "--json-out",
        str(out),
        *extra,
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"replay failed: {result.stderr.strip()}")
    return json.loads(out.read_text())


def print_table(rows: list[tuple[str, str, dict]], mode: str) -> None:
    header = (
        f"{'config':<22} {'clip':<12} {'chunk':>6} {'segs':>5} {'minted':>7} {'over':>6} "
        f"{'auto':>6} {'WRONG':>6} {'clicks':>7}"
    )
    if mode == "multi-session":
        header += f" {'recog%':>7} {'notrec':>7} {'0-click':>8} {'sug-BAD':>8}"
    print(header)
    print("-" * len(header))
    for label, chunk, payload in rows:
        metrics = payload["metrics"]
        over = metrics["clustersMinted"] / max(payload["trueSpeakers"], 1)
        line = (
            f"{label:<22} {payload['clip']:<12} {chunk:>6} {payload['segments']:>5} "
            f"{metrics['clustersMinted']:>7} {over:>5.1f}x "
            f"{metrics['autoMerges']:>6} {metrics['wrongAutoMerges']:>6} "
            f"{metrics['suggestions']:>7}"
        )
        if mode == "multi-session":
            named = (
                metrics["stuckRecognized"]
                + metrics["stuckRecognizedWrongPerson"]
                + metrics["stuckUnrecognized"]
            )
            recognized_pct = 100.0 * metrics["stuckRecognized"] / max(named, 1)
            line += (
                f" {recognized_pct:>6.0f}% {metrics['stuckUnrecognized']:>7} "
                f"{metrics['stuckAutoLinked']:>8} "
                f"{metrics['stuckSuggestedWrongSpeaker']:>8}"
            )
        print(line)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--split", default="test", choices=["dev", "test"])
    parser.add_argument("--revision", default=DEFAULT_REVISION)
    parser.add_argument(
        "--speakers", type=int, default=3, help="only use clips with exactly N speakers"
    )
    parser.add_argument("--clips", type=int, default=3, help="how many clips to use")
    parser.add_argument(
        "--scan-limit",
        type=int,
        default=60,
        help="how many clips to stream while looking for --speakers matches",
    )
    parser.add_argument("--chunk-seconds", default=DEFAULT_CHUNK_SECONDS)
    parser.add_argument("--binary", help="path to diarize_to_rttm_speakrs")
    parser.add_argument("--replay-binary", help="path to replay_speaker_identity")
    parser.add_argument("--models-dir")
    parser.add_argument(
        "--work-dir",
        default=str(REPO_ROOT / "target" / "segment-identity-bench"),
        help="where dumps are written and reused",
    )
    parser.add_argument(
        "--replay-only",
        action="store_true",
        help="skip diarization and replay dumps already in --work-dir",
    )
    parser.add_argument(
        "--mode",
        default="single-session",
        choices=["single-session", "multi-session", "both"],
    )
    parser.add_argument(
        "--sessions-every",
        type=int,
        default=10,
        help="segments per session in multi-session mode",
    )
    parser.add_argument(
        "--sweep",
        action="store_true",
        help="replay every configuration in SWEEP, not just the shipped rules",
    )
    args = parser.parse_args()
    sys.stdout.reconfigure(line_buffering=True)

    work_dir = Path(args.work_dir)
    work_dir.mkdir(parents=True, exist_ok=True)
    chunk_sizes = [float(value) for value in args.chunk_seconds.split(",") if value]

    if not args.replay_only:
        try:
            import soundfile as sf
            from datasets import Audio, load_dataset
        except ImportError as exc:
            sys.exit(f"missing dependency: {exc}. Run: pip install -r requirements.txt")

        binary = find_binary("diarize_to_rttm_speakrs", args.binary)
        print(
            f"streaming {DATASET_ID} split={args.split} looking for "
            f"{args.clips} clips with exactly {args.speakers} speakers ...",
            file=sys.stderr,
        )
        ds = load_dataset(
            DATASET_ID, split=args.split, revision=args.revision, streaming=True
        )
        ds = ds.cast_column("audio", Audio(decode=False))

        found = 0
        for index, sample in enumerate(ds):
            if index >= args.scan_limit or found >= args.clips:
                break
            reference = reference_turns(sample)
            if len({speaker for _, _, speaker in reference}) != args.speakers:
                continue

            uri = f"{args.split}-{index:04d}"
            raw = sample["audio"].get("bytes")
            if raw is None and sample["audio"].get("path"):
                raw = Path(sample["audio"]["path"]).read_bytes()
            data, sample_rate = sf.read(io.BytesIO(raw))
            duration = len(data) / sample_rate
            print(f"{uri}: {duration:.0f}s, {args.speakers} speakers")

            for chunk_seconds in chunk_sizes:
                out_path = work_dir / f"{uri}-{chunk_seconds:g}s.json"
                payload = dump_clip(
                    binary,
                    args.models_dir,
                    uri,
                    data,
                    sample_rate,
                    reference,
                    chunk_seconds,
                    work_dir,
                    out_path,
                )
                print(
                    f"  chunk {chunk_seconds:g}s -> {len(payload['segments'])} segments, "
                    f"{sum(len(s['clusters']) for s in payload['segments'])} raw clusters"
                )
            found += 1

        if found == 0:
            sys.exit(
                f"no clips with exactly {args.speakers} speakers in the first "
                f"{args.scan_limit} of split={args.split}; raise --scan-limit"
            )

    replay_binary = find_binary("replay_speaker_identity", args.replay_binary)
    dumps = sorted(
        path
        for path in work_dir.glob("*.json")
        if not path.name.endswith(".metrics.json")
    )
    if not dumps:
        sys.exit(f"no dumps found in {work_dir}; run without --replay-only first")

    configs = SWEEP if args.sweep else [("shipped", [])]
    modes = (
        ["single-session", "multi-session"] if args.mode == "both" else [args.mode]
    )

    for mode in modes:
        print(f"\n=== {mode} ===")
        extra_mode = (
            ["--sessions-every", str(args.sessions_every)]
            if mode == "multi-session"
            else []
        )
        rows = []
        for label, extra in configs:
            for dump in dumps:
                payload = replay(replay_binary, dump, mode, extra + extra_mode)
                rows.append((label, f"{payload['chunkSeconds']:g}s", payload))
        print_table(rows, mode)


if __name__ == "__main__":
    main()
