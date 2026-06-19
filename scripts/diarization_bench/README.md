# Speaker diarization DER benchmark

Measures Mnema's speaker diarization accuracy as **Diarization Error Rate (DER)**
against the [VoxConverse](https://www.robots.ox.ac.uk/~vgg/data/voxconverse/)
dataset, so accuracy changes can be tracked and tuned instead of eyeballed.

```
DER = (false alarm + missed detection + speaker confusion) / total reference speech
```

- **missed detection / false alarm** → the pyannote *segmentation* stage (speech
  vs non-speech, boundaries, `min_duration_on/off`).
- **speaker confusion** → the *clustering / embedding* stage (`clustering_threshold`,
  `cross_chunk_threshold`, embedding model). This is the usual accuracy lever.

The harness scores with a 0.25s collar and reports DER both **including** and
**excluding** overlapped speech. Mnema emits one speaker per instant, so the
overlap-included number is penalized on overlapping speech — track both.

## How it works

1. `run_der.py` **streams** clips + ground-truth turns from the HuggingFace
   dataset `diarizers-community/voxconverse` (pre-split, timestamped, CC-BY-4.0).
   Streaming reads parquet shards lazily and stops after the requested clips, so
   it avoids the multi-GB Arrow cache that a full `load_dataset` writes to disk —
   important on a near-full disk. Splits are `dev` (216 clips) and `test` (232).
2. Each clip's audio is written to a temp WAV; the Rust `diarize_to_rttm` binary
   runs the **real** sherpa-onnx provider (`analyze_sherpa_request_blocking`) and
   prints a hypothesis RTTM.
3. `pyannote.metrics` scores the hypothesis against the reference.

## Prerequisites

You need the diarization models installed — the simplest path is to run the
desktop app once and let it download a preset, which lands them at
`~/Library/Application Support/com.shaikzeeshan.mnema/speaker-analysis-models`
(the binary's default `--models-dir`).

1. Build the Rust binary (macOS; needs the `sherpa-onnx` feature — no `mnema-cli`
   sidecar required since this targets the `speaker-analysis` crate, not the
   Tauri app):

   ```sh
   cargo build -p speaker-analysis --features sherpa-onnx --release --bin diarize_to_rttm
   ```

2. Set up Python deps (a virtualenv is recommended):

   ```sh
   cd scripts/diarization_bench
   python -m venv .venv && source .venv/bin/activate
   pip install -r requirements.txt
   ```

## Run

Fast loop (first 8 test clips) — use this while tuning:

```sh
python run_der.py --limit 8
```

Frozen subset (reproducible, committed in `voxconverse_subset.txt`):

```sh
python run_der.py --manifest voxconverse_subset.txt --json-out baseline.json
```

Full split for headline numbers:

```sh
python run_der.py --all --json-out voxconverse_test_full.json
```

### Tuning sweeps

The script forwards diarization knobs to the binary so you can compare configs
without rebuilding:

```sh
python run_der.py --manifest voxconverse_subset.txt --clustering-threshold 0.70
python run_der.py --manifest voxconverse_subset.txt --model-id reverb-v1-nemo-titanet-large
```

Save a `--json-out` baseline first, then re-run with a tweak and diff the
aggregate DER and its confusion / miss / FA split.

## Notes

- Clips are selected by **stream position** (0-based) within the split. That
  order is stable for a pinned dataset `--revision` (`DEFAULT_REVISION` in
  `run_der.py`), so `voxconverse_subset.txt` indices are reproducible; bump the
  revision deliberately and re-baseline if you change it.
- `--work-dir <path>` keeps the exported WAVs and hypothesis RTTMs for
  inspection instead of using a temp dir.
- VoxConverse is in-the-wild audio (debates, talk shows) and covers Mnema's
  "system audio / video playing" case. For the meeting / call case, the same
  harness works against `diarizers-community/ami` with a few field tweaks.

## NME-SC over-clustering experiment (prototype)

The production cross-chunk clustering is threshold-AHC (`cross_chunk_threshold=0.60`):
it has no global prior on speaker count and **over-splits** — on this 10-clip
subset it over-estimates the speaker count on 100% of clips (mean abs error
~17.9; e.g. 2 real speakers -> 24 predicted), even at DER ~9.7%.

`nme_sc.py` is a self-contained numpy/scipy prototype of **NME-SC** (Normalized
Maximum Eigengap Spectral Clustering, Park et al. 2019 — what NeMo uses), which
estimates the speaker count from the maximum eigengap of the normalized Laplacian
instead of a similarity threshold. `bench_nme_sc.py` measures it against the
baseline **apples-to-apples**: same subset, same reference, same `pyannote.metrics`
DER (0.25s collar) and same `SpeakerCountStats` (both imported from `run_der.py`).

This is additive/opt-in and does **not** touch the production Rust clustering:
`diarize_to_rttm --dump-embeddings <path>` (Rust flag) dumps the
pre-global-clustering local-cluster centroid embeddings + their pending turns;
NME-SC re-clusters those centroids; the RTTM is rebuilt from the turns + new
labels. Everything up to the global cluster-count step is identical to baseline.

Run it (after building the binary + installing deps as above):

```sh
cd scripts/diarization_bench
source .venv/bin/activate
# 1. Export the subset clips once (same pinned revision as run_der.py):
python export_clips.py --manifest voxconverse_subset.txt --out-dir work
# 2. Score baseline AHC vs NME-SC on all subset clips:
python bench_nme_sc.py --manifest voxconverse_subset.txt --work-dir work \
    --binary ../../target/release/diarize_to_rttm --json-out nme_sc_compare.json
```

`--max-speakers` (default 20) bounds the eigengap search; keep it generous so the
eigengap, not the cap, drives the count.
```
