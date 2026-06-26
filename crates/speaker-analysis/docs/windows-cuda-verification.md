# Windows CUDA Diarization — On-Device Verification Matrix

Operator checklist for the **#137 CUDA GPU Diarization Execution Backend** (Windows).
This is the on-device pass that **cannot be CI'd** (needs a real NVIDIA GPU + the NVIDIA
redist pack). Pure-logic and provenance assembly are already unit-tested
(`cargo test -p speaker-analysis`); this doc covers the live hardware matrix only.

Decision record: ADR 0005 (`crates/speaker-analysis/docs/adr/0005-windows-cuda-backend-in-app-provisioning-and-dynamic-ort.md`).
Backend stays orthogonal to identity throughout: one `model_id`, one Voiceprint Space —
the backend is observable **only** in provenance (`executionMode`), never a model choice.

## Fixture / dev box

- **RTX 5070 Ti dev box**: NVIDIA GPU + current driver + `nvml.dll` present, but **no
  system CUDA 12 / cuDNN 9** by default. This is the *native fallback fixture* — rows 2
  and 4 exercise paths that exist precisely because a GPU box has no CUDA toolkit until
  the pack is installed.

## Pre-req / setup

1. **Build the Windows app with the CUDA-enabled ONNX Runtime 1.24.x DLLs staged.**
   - `onnxruntime.dll` (ONNX Runtime 1.24.4, MIT) is committed and staged by default.
   - For the CUDA provider DLL (`onnxruntime_providers_cuda.dll` +
     `onnxruntime_providers_shared.dll`), point the staging script at a GPU redist zip:
     set `MNEMA_ORT_REDIST_ZIP` to a Microsoft `onnxruntime-win-x64-gpu-1.24.x.zip`
     (the `-gpu-` variant carries the CUDA provider DLLs) before the build. The script
     `scripts/prepare-ort-dylibs.mjs` extracts whichever of the three DLLs it contains
     into `apps/desktop/src-tauri/resources/ort/`, which `tauri.windows.conf.json` bundles
     flat next to the exe. Local build helper: `scripts/build-windows-local.ps1`.
   - Confirm after build: `onnxruntime.dll`, `onnxruntime_providers_shared.dll`,
     `onnxruntime_providers_cuda.dll` all sit next to the packaged `mnema.exe`.
2. **Install the GPU pack via Settings** (for the rows that need it): Settings → the
   Intelligence panel → **GPU acceleration** → accept the NVIDIA CUDA + cuDNN license →
   **Enable GPU acceleration**. The pack (CUDA 12.9.1 + cuDNN 9.10.2 redist) lands flat in
   `<app_data_dir>/gpu-acceleration-pack/` with a `.installed.json` marker. That marker is
   the only gate the helper checks (`gpu_pack_present`).
3. **Toggle storage:** the default-on "Use GPU acceleration" override persists to
   `<app_data_dir>/gpu-acceleration.json`; it is read **live** at each job spawn.

## How to read provenance (each row asserts against one of these)

- **Settings (live, last run):** Settings → Intelligence → GPU acceleration. Installed
  state shows **"Last run: GPU (CUDA)"** or **"Last run: CPU"**; a CUDA-init fallback adds
  the warn notice **"GPU initialization failed — ran on CPU: <reason>"**. Backed by
  `GpuAccelerationState.record_execution_outcome`, exposed via the
  `get_gpu_acceleration_state` Tauri command (`apps/desktop/src-tauri/src/gpu_acceleration.rs`).
- **Result payload / DB:** the Speaker Analysis Job's structured-payload JSON (serialized
  `SpeakerAnalysisOutput`) carries `metadata.provenance.executionMode` and, on a
  CUDA-init fallback only, `executionModeRequested` + `cudaFallbackReason`. Stored as the
  processing result's structured payload (`crates/app-infra/src/processing/speaker_analysis.rs`).
- **Helper stderr log:** the speaker-analysis helper subprocess logs
  `speaker-analysis helper completed: elapsed_ms=… timeout_seconds=… stdout_bytes=…`
  (`speaker_analysis_runtime.rs`) — this confirms the helper ran; the execution mode
  itself is in the returned JSON payload / Settings above, not in that line.

## Matrix

Each row: **action → expected provenance + Settings state.** Trigger a Speaker Analysis
Job by recording (or re-processing) a short multi-speaker audio segment after each setup
change.

| # | Setup | Action | Expected `executionMode` | Extra provenance keys | Settings state |
|---|-------|--------|--------------------------|-----------------------|----------------|
| 1 | Pack installed, GPU healthy, toggle **ON** | run a job | `"cuda"` | none | "installed", **"Last run: GPU (CUDA)"** |
| 2 | Pack installed but **cuDNN withheld/removed** (delete `cudnn64_9.dll` from the pack dir, or otherwise break CUDA init), toggle ON | run a job | `"cpu"` (job still **SUCCEEDS**) | `executionModeRequested="cuda"` + `cudaFallbackReason=<error>` | warn notice **"GPU initialization failed — ran on CPU: …"** surfaced |
| 3 | Pack installed, toggle **OFF** (Force-CPU) | run a job | `"cpu"` | **none** (no `executionModeRequested`/`cudaFallbackReason` noise) | "installed", **"Last run: CPU"**, toggle off |
| 4 | **No pack installed** (GPU present) | run a job | `"cpu"` | **none** (CUDA never attempted — "not provisioned" is not a failure) | the pack **OFFER** ("gpu acceleration available" / "not installed" / Enable button) |

Notes:
- Row 2 vs Row 4 is the key distinction: a `cudaFallbackReason` appears **only** when the
  pack is present and CUDA still fails to initialize. No pack ⇒ plain CPU, zero diagnostics.
- A CUDA failure *after* successful init (not an init failure) is a **job failure**, not a
  silent CPU re-run — out of scope for this happy-path matrix but do not mistake it for row 2.

## Regression (AC of Slice 1 — dynamic ORT must not break the CPU paths)

- [ ] **Parakeet transcription** still works on Windows under dynamic ORT: a normal
  recording yields a transcript (the int8 `parakeet-tdt-0.6b-v3-onnx-int8` model, CPU,
  ONNX Runtime loaded from the bundled `onnxruntime.dll` via `ORT_DYLIB_PATH`).
- [ ] **CPU speakrs diarization** still works on Windows under dynamic ORT: the same
  recording yields speaker turns/clusters with `executionMode="cpu"` (rows 3/4 already
  cover this; confirm a real multi-speaker clip clusters sensibly).
- The **23 known pre-existing Windows test failures** (Unix-path assumptions, tracked for
  #77) are out-of-scope noise — do not block this pass or blame the CUDA changes on them.
