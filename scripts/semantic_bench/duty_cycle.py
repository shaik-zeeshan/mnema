#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "torch>=2.5",
#   "transformers>=4.48,<5",
#   "huggingface_hub>=0.26",
#   "numpy>=1.26",
#   "einops>=0.8",
#   "psutil>=6",
# ]
# ///
"""Resource cost of one embedding model under the shipped backfill duty cycle.

Mirrors `apps/desktop/src-tauri/src/semantic_search_worker.rs`: embed
SWEEP_BATCH_SIZE anchors, then sleep
`clamp(BACKFILL_BATCH_COOLDOWN_MULTIPLIER * pass_elapsed, MIN, MAX)`, with the
embedder kept warm across passes. Samples process CPU / RSS / physical
footprint and system GPU utilization throughout, plus a pre-load idle baseline.

> CAVEAT: Python/torch-MPS, not the shipped candle-Metal path. Absolute CPU and
> GPU numbers do NOT transfer to the app; the RANKING between models does, and
> resident weights are within ~the file size either way.

    uv run scripts/semantic_bench/duty_cycle.py \
        --model granite-ml-97m --corpus "$SCRATCH/corpus.jsonl" \
        --passes 24 --out "$SCRATCH/duty_granite-ml-97m.json"
"""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from bench import MODELS, Embedder  # noqa: E402  (same-dir harness, reused wholesale)

# Mirrors semantic_search_worker.rs BACKFILL_BATCH_COOLDOWN_*. These pin the duty
# ratio (1/(1+multiplier)); a stale multiplier here reports a duty the app never runs.
SWEEP_BATCH_SIZE = 16
COOLDOWN_MULTIPLIER = 3.0
COOLDOWN_MIN_S = 0.150
COOLDOWN_MAX_S = 30.000
# Mirrors runtime.rs MAX_DOCUMENT_CHUNKS: a document is embedded from at most this
# many windows, so it is also the per-anchor forward-pass cost this sweep measures.
MAX_DOCUMENT_CHUNKS = 2

_FOOTPRINT_RE = re.compile(r"Physical footprint:\s+([0-9.]+)([KMG])")


def phys_footprint_mb(pid: int) -> float | None:
    """macOS physical footprint (the number that counts unified-memory GPU
    buffers), via vmmap. `ps` RSS misses them."""
    try:
        out = subprocess.run(
            ["/usr/bin/vmmap", "--summary", str(pid)],
            capture_output=True, text=True, timeout=30,
        ).stdout
    except Exception:
        return None
    m = _FOOTPRINT_RE.search(out)
    if not m:
        return None
    scale = {"K": 1e-3, "M": 1.0, "G": 1e3}[m.group(2)]
    return float(m.group(1)) * scale


def gpu_util() -> float | None:
    """System-wide GPU busy %, from the IOAccelerator perf counters. System
    wide, so anything else on the GPU (incl. a running Mnema) is in here —
    that's why the run records an idle baseline to compare against."""
    try:
        out = subprocess.run(
            ["ioreg", "-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"],
            capture_output=True, text=True, timeout=10,
        ).stdout
    except Exception:
        return None
    hits = re.findall(r'"Device Utilization %"=(\d+)', out)
    return max(int(h) for h in hits) if hits else None


class Sampler(threading.Thread):
    """Background CPU / RSS / GPU sampler. `phase` tags each sample so active
    passes can be told apart from the cooldown sleeps."""

    def __init__(self, pid: int, interval: float = 0.05):
        super().__init__(daemon=True)
        import psutil

        self.proc = psutil.Process(pid)
        self.proc.cpu_percent(None)  # prime the delta
        self.interval = interval
        self.samples: list[dict] = []
        self.phase = "idle"
        # NOT `_stop`: `threading.Thread._stop` is CPython's own internal method,
        # called from `Thread.join`. Shadowing it with an Event makes join() raise.
        self._done = threading.Event()

    def run(self):
        last_gpu = 0.0
        gpu = None
        while not self._done.is_set():
            now = time.perf_counter()
            if now - last_gpu >= 0.25:  # ioreg is a fork+exec; don't do it every tick
                gpu, last_gpu = gpu_util(), now
            try:
                cpu = self.proc.cpu_percent(None)
                rss = self.proc.memory_info().rss / 1e6
            except Exception:
                break
            self.samples.append({"t": now, "phase": self.phase, "cpu": cpu, "rss_mb": rss, "gpu": gpu})
            self._done.wait(self.interval)

    def stop(self):
        self._done.set()
        self.join(timeout=5)

    def agg(self, phase: str | None = None) -> dict:
        rows = [s for s in self.samples if phase is None or s["phase"] == phase]
        rows = rows[1:] or rows  # drop the primed first sample
        if not rows:
            return {}
        cpu = [r["cpu"] for r in rows]
        rss = [r["rss_mb"] for r in rows]
        gpu = [r["gpu"] for r in rows if r["gpu"] is not None]
        out = {
            "samples": len(rows),
            "cpu_mean": round(statistics.mean(cpu), 1),
            "cpu_max": round(max(cpu), 1),
            "rss_mean_mb": round(statistics.mean(rss), 1),
            "rss_max_mb": round(max(rss), 1),
        }
        if gpu:
            out |= {"gpu_mean": round(statistics.mean(gpu), 1), "gpu_max": max(gpu)}
        return out


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--model", required=True, help=f"one of {','.join(MODELS)}")
    p.add_argument("--corpus", required=True)
    p.add_argument("--out", required=True, help="results JSON (scratchpad, never the repo)")
    p.add_argument("--passes", type=int, default=24, help="measured sweep passes")
    p.add_argument("--warmup", type=int, default=2, help="unmeasured passes (Metal shader compile)")
    p.add_argument("--batch", type=int, default=SWEEP_BATCH_SIZE)
    p.add_argument("--baseline-seconds", type=float, default=5.0)
    p.add_argument("--window", type=int, default=256)
    p.add_argument("--max-doc-chunks", type=int, default=MAX_DOCUMENT_CHUNKS,
                   help="cap each document at N windows (runtime.rs MAX_DOCUMENT_CHUNKS); "
                        "0 = uncapped. This is the work lever the sweep exists to compare.")
    p.add_argument("--device", default=None)
    args = p.parse_args()

    if args.model not in MODELS:
        raise SystemExit(f"unknown model {args.model!r}; known: {', '.join(MODELS)}")

    import torch

    device = args.device or ("mps" if torch.backends.mps.is_available() else "cpu")
    corpus = [json.loads(l) for l in Path(args.corpus).read_text().splitlines() if l.strip()]
    texts = [d["text"] for d in corpus]
    # `<=`, not `<`: the pass cursor advances modulo `len(texts) - batch`, which is a
    # divide-by-zero at exactly one batch.
    if len(texts) <= args.batch:
        raise SystemExit("corpus must hold more than one batch")

    # Pay torch's Metal-context cost BEFORE the baseline, so the post-load
    # footprint delta is the model's weights + activations and not ~1.7 GB of
    # framework. (The framework floor is a Python-harness artefact; candle in
    # the app does not pay it — see the caveat.)
    if device == "mps":
        torch.zeros(8, 8, device="mps").sum().item()

    pid = os.getpid()
    sampler = Sampler(pid)
    sampler.start()

    time.sleep(args.baseline_seconds)  # pre-load idle baseline (this process + the machine)
    baseline = sampler.agg("idle")
    baseline["footprint_mb"] = phys_footprint_mb(pid)

    sampler.phase = "load"
    t0 = time.perf_counter()
    emb = Embedder(MODELS[args.model], device, args.window, args.max_doc_chunks)
    load_s = time.perf_counter() - t0
    loaded = {"footprint_mb": phys_footprint_mb(pid), "rss_mb": sampler.samples[-1]["rss_mb"]}

    # Warmup: first pass on Metal pays shader compilation, which is not the
    # steady state the worker lives in.
    sampler.phase = "warmup"
    cursor = 0
    for _ in range(args.warmup):
        emb.embed(texts[cursor : cursor + args.batch], "document")
        cursor = (cursor + args.batch) % (len(texts) - args.batch)

    passes, chunk_total = [], 0
    peak_footprint = loaded["footprint_mb"] or 0.0
    for i in range(args.passes):
        sampler.phase = "active"
        t = time.perf_counter()
        _, chunks = emb.embed(texts[cursor : cursor + args.batch], "document")
        elapsed = time.perf_counter() - t
        cursor = (cursor + args.batch) % (len(texts) - args.batch)
        chunk_total += chunks
        cooldown = min(max(COOLDOWN_MULTIPLIER * elapsed, COOLDOWN_MIN_S), COOLDOWN_MAX_S)
        passes.append({"pass": i, "seconds": elapsed, "chunks": chunks, "cooldown_s": cooldown})
        sampler.phase = "cooldown"
        time.sleep(cooldown)
        if i % 6 == 5:
            peak_footprint = max(peak_footprint, phys_footprint_mb(pid) or 0.0)
        print(
            f"  pass {i + 1}/{args.passes}: {elapsed:.2f}s "
            f"({chunks} chunks) cooldown {cooldown:.2f}s",
            file=sys.stderr,
        )

    sampler.phase = "done"
    peak_footprint = max(peak_footprint, phys_footprint_mb(pid) or 0.0)
    mps_mb = (
        torch.mps.driver_allocated_memory() / 1e6 if device == "mps" else None
    )
    sampler.stop()

    base_fp = baseline.pop("footprint_mb", None)
    secs = [q["seconds"] for q in passes]
    wall = sum(secs) + sum(q["cooldown_s"] for q in passes)
    report = {
        "model": args.model,
        "repo": MODELS[args.model]["repo"],
        "dim": MODELS[args.model]["dim"],
        "device": device,
        "window": args.window,
        "max_doc_chunks": args.max_doc_chunks,
        "cooldown_multiplier": COOLDOWN_MULTIPLIER,
        "snapshot_mb": round(emb.size_mb, 1),
        "batch": args.batch,
        "passes": args.passes,
        "load_seconds": round(load_s, 2),
        "pass_seconds": {
            "p50": round(statistics.median(secs), 3),
            "p95": round(sorted(secs)[int(0.95 * (len(secs) - 1))], 3),
            "mean": round(statistics.mean(secs), 3),
        },
        "chunks_per_pass": round(chunk_total / args.passes, 2),
        "anchors_per_sec_active": round(args.passes * args.batch / sum(secs), 1),
        "anchors_per_sec_duty": round(args.passes * args.batch / wall, 1),
        "duty_ratio": round(sum(secs) / wall, 3),
        "memory": {
            "baseline_footprint_mb": base_fp,
            "after_load_footprint_mb": loaded["footprint_mb"],
            "after_load_rss_mb": round(loaded["rss_mb"], 1),
            # RSS delta = the CPU-side weight copy; the MPS driver figure below is
            # torch's GPU pool (a caching allocator — it over-reserves, so it is a
            # torch number, not a model number). Read these two, not the footprint.
            "rss_delta_mb": round(loaded["rss_mb"] - (baseline.get("rss_mean_mb") or 0), 1),
            "peak_footprint_mb": round(peak_footprint, 1),
            # What the model itself costs on top of an already-warm runtime —
            # the only memory number here that means anything for the app.
            "model_peak_delta_mb": round(peak_footprint - (base_fp or 0), 1),
            "mps_driver_allocated_mb": round(mps_mb, 1) if mps_mb else None,
        },
        "baseline_idle": baseline,
        "active": sampler.agg("active"),
        "cooldown": sampler.agg("cooldown"),
        "whole_run": sampler.agg(None),
        "per_pass": passes,
    }
    Path(args.out).write_text(json.dumps(report, indent=2))

    m, a, c = report["memory"], report["active"], report["cooldown"]
    print(
        f"\n{args.model}: snapshot {report['snapshot_mb']:.0f} MB, load {load_s:.1f}s\n"
        f"  pass p50 {report['pass_seconds']['p50']:.2f}s for {args.batch} anchors "
        f"({report['chunks_per_pass']:.1f} chunks) → {report['anchors_per_sec_duty']:.1f} anchors/s "
        f"at duty {report['duty_ratio']:.0%}\n"
        f"  CPU active {a.get('cpu_mean')}% (max {a.get('cpu_max')}%), cooldown {c.get('cpu_mean')}%\n"
        f"  GPU active {a.get('gpu_mean')}% vs idle baseline {baseline.get('gpu_mean')}%\n"
        f"  footprint {m['baseline_footprint_mb']:.0f} → {m['peak_footprint_mb']:.0f} MB peak "
        f"(model costs +{m['model_peak_delta_mb']:.0f} MB over a warm runtime)"
    )
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
