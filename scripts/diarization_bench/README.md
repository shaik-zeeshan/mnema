# Speaker diarization DER benchmark

> **Shipped provider:** Mnema ships **`speakrs`** as the sole on-device diarization
> provider ([ADR 0003](../../crates/speaker-analysis/docs/adr/0003-remove-sherpa-make-speakrs-sole-diarization-provider.md)).
> Sherpa ONNX — its `sherpa-onnx` Cargo feature and its `diarize_to_rttm` bench
> binary — has been removed from this branch. The historical Sherpa-vs-speakrs
> comparison (that comparison is *why* speakrs won and sherpa was dropped) lives in
> ADR 0003 as the benchmark record; re-running it requires a checkout from before
> that PR, since neither the feature nor the bin exists here. Score `speakrs` via
> the `diarize_to_rttm_speakrs` binary for current numbers.

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
2. Each clip's audio is written to a temp WAV; the Rust `diarize_to_rttm_speakrs`
   binary runs the **real** shipped speakrs provider
   (`analyze_speakrs_request_blocking`) and prints a hypothesis RTTM. It speaks the
   `--binary` CLI/RTTM contract `run_der.py` expects, so DER scoring against the
   reference is apples-to-apples (and stays comparable to the historical sherpa
   numbers, which used the same contract).
3. `pyannote.metrics` scores the hypothesis against the reference.

## Prerequisites

You need the diarization models installed — the simplest path is to run the
desktop app once and let it download a preset, which lands them at
`~/Library/Application Support/day.mnema/speaker-analysis-models`
(the binary's default `--models-dir`).

1. Build the Rust binary (macOS; no `mnema-cli` sidecar required since this targets
   the `speaker-analysis` crate, not the Tauri app).

   Build the shipped **speakrs** provider's bench bin (needs the `speakrs` feature;
   OpenBLAS must be installed first — `brew install openblas pkgconf` and
   `export PKG_CONFIG_PATH=$(brew --prefix openblas)/lib/pkgconfig`):

   ```sh
   cargo build -p speaker-analysis --features speakrs --release --bin diarize_to_rttm_speakrs
   ```

   (The removed `sherpa-onnx` feature and its `diarize_to_rttm` bin are gone on this
   branch; re-running the historical sherpa comparison needs a checkout from before
   the PR that dropped them.)

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
python run_der.py --manifest voxconverse_subset.txt --model-id pyannote-community-1-wespeaker
```

> **Most tuning flags are inert on speakrs.** `--clustering-threshold`,
> `--cross-chunk-threshold`, and the other sherpa-era knobs are *accepted and
> ignored* by the `diarize_to_rttm_speakrs` bin (it prints a stderr note and
> uses speakrs's single fixed pipeline). Only the removed sherpa bin honored
> them, so sweeping them here yields a *flat* DER — that's the flag being
> ignored, not the parameter having no effect. `--model-id` is the one knob the
> speakrs bin still honors (it selects the preset).

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

## Cross-segment identity: `segment_identity_bench.py`

DER answers **"did we separate the voices inside this file"**. It cannot see the
bug behind *"merge with Unknown Speaker 2, 3, 5…"* and *"I named someone and it
didn't stick"*, because that decision is made by
`resolve_stable_speaker_cluster` in `app-infra` — after diarization, comparing
each segment's clusters against the ones already stored **for the same session**.
A run can score DER 0% and still mint one Unknown Speaker per capture segment.

`segment_identity_bench.py` measures that second thing, in two stages:

1. **Dump** — slice a clip into fixed-length segments, run the real speakrs
   provider on each (`diarize_to_rttm_speakrs --dump-clusters`), and label every
   emitted cluster with the ground-truth speaker it mostly covers.
2. **Replay** — feed those centroids through the real shipped resolver
   (`replay_speaker_identity`, which calls
   `processing::speaker_resolution::resolve_stable_speaker_cluster_from_candidates`
   directly, so the harness can never drift from production rules).

The split matters: embedding audio is the expensive step and it does **not**
change when you tune a threshold. One CoreML pass per chunk size, then unlimited
free threshold sweeps.

```sh
cargo build -p speaker-analysis --features speakrs --release --bin diarize_to_rttm_speakrs
cargo build -p app-infra --release --bin replay_speaker_identity

# stage 1 + 2: dump 3 three-speaker clips at each chunk size, replay shipped rules
python segment_identity_bench.py --chunk-seconds 10,30,60,180,300 --clips 3

# stage 2 only: sweep candidate fixes over the dumps already on disk (no CoreML)
python segment_identity_bench.py --replay-only --sweep --mode both
```

### What the columns mean

| Column | Reads as |
|---|---|
| `minted` / `over` | Clusters created vs. real speakers. `3.0x` means three "Unknown Speaker" rows per actual person. |
| `auto` | Segments that silently reused an existing speaker — the invisible good path. |
| `WRONG` | Auto-merges that fused **two different people**. Must stay 0; a config that wins on clicks but scores here is a regression, not a win. |
| `clicks` | Merge suggestions raised — literally what the user is complaining about. |
| `recog%` | `--mode multi-session` only. Of the named person's clusters in *later* sessions, how many the enrolled voiceprint matched. This is "did naming stick" — 0% means enrolling bought nothing. |
| `notrec` | Later clusters of that person the voiceprint missed entirely: they surface as plain "Unknown Speaker". |
| `0-click` | Later clusters that auto-linked to a cluster already carrying the person — the only outcome that costs the user nothing. Recognition alone never produces this today: a match is a *suggestion*, so a recognized voice still costs one confirm per cluster. |
| `sug-BAD` | Recognition identified the person, yet resolution suggested merging with a **different** speaker's cluster. The veto-not-steer defect. |

`--mode multi-session` resets the candidate pool every `--sessions-every`
segments, reproducing what `store.rs` does across recordings (candidates are
filtered `WHERE session_id = ?1`; enrolled voiceprints are the only bridge). That
is where "naming someone doesn't stick" lives — measure it there, not in
single-session.

### Configurations in the sweep

`--sweep` replays each dump under the `SWEEP` list in the script, isolating one
change before stacking them:

- `shipped` — today's rules; the baseline every other row is judged against.
- `F4 reaverage` — auto-merge folds the incoming centroid into a
  duration-weighted mean instead of the shipped upsert, which **overwrites** the
  survivor's embedding with the newest segment's (`store.rs` `ON CONFLICT …
  embedding = excluded.embedding`). The anchor does not merely fail to improve;
  it drifts to whatever was seen last.
- `F3 person-aware` — a near-tie blocks auto-reuse only between *different*
  identities. Several unnamed fragments of one voice scoring alike currently make
  the system *more* reluctant to merge them, the more of them there are.
- `+steer` — a recognition match picks the right cluster instead of only
  vetoing. Today a confirmed "this is Alice" blocks the auto-merge and then
  suggests merging with whichever cluster scored highest — **even when that
  cluster is Bob**.
- `F6@0.70` — lower auto-reuse threshold, applied last, since the rows above
  shift the score distribution and must be re-measured before tuning it.

## NME-SC over-clustering experiment (prototype)

This experiment targeted the **removed sherpa** cross-chunk clustering, which
was threshold-AHC (`cross_chunk_threshold=0.60`): it had no global prior on
speaker count and **over-split** — on this 10-clip subset it over-estimated the
speaker count on 100% of clips (mean abs error ~17.9; e.g. 2 real speakers -> 24
predicted), even at DER ~9.7%. (The shipped speakrs provider does not use this
pipeline: it clusters with VBx plus a 0.6 centroid stitch.)

`nme_sc.py` is a self-contained numpy/scipy prototype of **NME-SC** (Normalized
Maximum Eigengap Spectral Clustering, Park et al. 2019 — what NeMo uses), which
estimates the speaker count from the maximum eigengap of the normalized Laplacian
instead of a similarity threshold. `bench_nme_sc.py` measures it against the
baseline **apples-to-apples**: same subset, same reference, same `pyannote.metrics`
DER (0.25s collar) and same `SpeakerCountStats` (both imported from `run_der.py`).

> **Pre-PR experiment.** This prototype was built against the removed **sherpa**
> bench bin: it relies on `diarize_to_rttm --dump-embeddings <path>`, a flag that
> only existed on the now-deleted `diarize_to_rttm` binary. Both that bin and the
> `sherpa-onnx` feature are gone on this branch, so reproducing the run below needs
> a checkout from before the PR that dropped them. The notes are kept as the record
> of the over-clustering investigation.

This was additive/opt-in and did **not** touch the production Rust clustering:
`diarize_to_rttm --dump-embeddings <path>` (Rust flag) dumped the
pre-global-clustering local-cluster centroid embeddings + their pending turns;
NME-SC re-clustered those centroids; the RTTM was rebuilt from the turns + new
labels. Everything up to the global cluster-count step was identical to baseline.

How it was run (on a pre-PR checkout, after building that binary + installing deps
as above):

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
