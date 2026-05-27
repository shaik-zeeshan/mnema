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
```
