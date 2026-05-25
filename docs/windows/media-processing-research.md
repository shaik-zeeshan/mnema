# Windows Media Processing Research

_Last researched: 2026-05-26_

This note records what Mnema currently uses for OCR, transcription, speaker analysis/diarization, VAD, and media file processing, plus the likely Windows 11 alternatives. It complements `docs/windows/runtime-capture-research.md`, which covers capture APIs, and `docs/windows/media-writers-preview-research.md`, which focuses on writer/preview implementation details.

## Short recommendation

For a Windows 11 first port, keep Mnema's local-processing architecture and replace only the macOS media plumbing:

- **OCR default:** use **PaddleOCR via `ocr-rs`/MNN** as the first Windows default candidate; keep **Tesseract via `tesseract-rs`** as a fallback/advanced provider. Hide **Apple Vision** on Windows.
- **Transcription default:** use **Local Whisper via `whisper-rs`** for the lower-memory default path. Keep **Parakeet ONNX via `ort`** as a high-quality/local option, likely with the int8 model as the practical Windows bundle. Hide **Apple Speech** on Windows.
- **Speaker diarization/recognition:** keep **Sherpa ONNX**. The crate and upstream toolkit support Windows; Mnema mainly lacks Windows audio decode and verification.
- **VAD / system-audio speech gate:** keep **Silero VAD** and **WebRTC VAD**. They are already cross-platform-ish; the missing piece is decoding captured audio files to mono PCM on Windows.
- **Media processing seam:** implement a Windows media backend around **Media Foundation Source Reader/Sink Writer** for decode, encode, validation, frame extraction, and trim/convert. Consider **Symphonia** for audio-only decode if we want pure Rust and accept format coverage/licensing. Keep **FFmpeg** as a fallback, not the default, because packaging/licensing is heavier.
- **Build target:** start with Windows 11 x64 MSVC. Expect Visual Studio Build Tools, CMake, and LLVM/libclang to be needed for current native crates (`whisper-rs`, `tesseract-rs`, `ocr-rs` bindgen). Later decide whether to prebuild/cache native libraries for release builds.

## What Mnema currently enables in the desktop build

`apps/desktop/src-tauri/Cargo.toml` currently enables these processing features:

| Area | Current feature/dependency | Main files | Windows status |
| --- | --- | --- | --- |
| OCR | `ocr` with `paddle-rs`, `tesseract-embedded` | `crates/ocr/*` | Engines are plausible on Windows, but packaging/build must be verified. Apple Vision is macOS-only. |
| Transcription | `audio-transcription` with `local-whisper`, `parakeet-onnx` | `crates/audio-transcription/*` | Model runtimes are plausible on Windows; audio decode is macOS/AVFoundation-only today. |
| Speaker analysis | `speaker-analysis` with `sherpa-onnx` | `crates/speaker-analysis/*`, `apps/desktop/src-tauri/src/speaker_analysis_runtime.rs` | Sherpa supports Windows; audio decode is macOS/AVFoundation-only today. |
| System-audio speech detection | `capture-vad` default `silero` plus `webrtc-vad` | `crates/capture-vad/*`, `crates/app-infra/src/processing/system_audio_speech_activity.rs` | VAD is plausible; audio-file decode is macOS-only today. |
| Media writers/readers | `capture-writers`, AVFoundation/ImageIO/AVAssetImageGenerator, `/usr/bin/afconvert`, `ffmpeg` for one trim path | `crates/capture-writers/src/lib.rs`, `apps/desktop/src-tauri/src/app_infra/frame_preview.rs`, `crates/capture-screen/src/lib.rs` | Needs Windows replacement. |

## Processor-by-processor Windows notes

### OCR

#### Apple Vision (`apple_vision`)

- Current use: OS-managed Vision.framework through `cidre` in `crates/ocr/src/lib.rs`.
- Windows 11: not available.
- Action: hide/disable provider and remove from Windows default/provider picker.

#### Windows OCR API (`Windows.Media.Ocr`) as possible OS-managed alternative

- Microsoft exposes `Windows.Media.Ocr.OcrEngine`, with available recognizer languages, max image dimension, and `RecognizeAsync(SoftwareBitmap)`.
- Important blocker: Microsoft docs say `Windows.Media.Ocr` APIs are **only supported for desktop apps with package identity** (MSIX/package identity). Mnema's likely Tauri Windows installers are MSI/NSIS, so this should not be the default unless we deliberately ship MSIX or otherwise add package identity.
- Action: keep as a later optional OS-managed provider, not the first Windows OCR path.

#### Tesseract (`tesseract`)

Current Mnema details:

- Crate: `tesseract-rs = 0.2.0` with default `build-tesseract`.
- Runtime layout expected by Mnema: `tessdata/eng.traineddata`, `tessdata/osd.traineddata`, `tessdata/snum.traineddata` under `ocr-models/tesseract/tesseract-5.5.2/`.
- The provider preprocesses frames itself: decode with `image`, grayscale, optional threshold/upscale, then calls `TesseractAPI` on raw pixels.

Windows research:

- `tesseract-rs` README claims Linux/macOS/Windows support and uses CMake plus C++ compiler, with `%APPDATA%/tesseract-rs` cache for built libraries/data.
- Current `tesseract-rs` build script downloads and builds Tesseract/Leptonica from source and has Windows MSVC/NMake handling.

Windows recommendation:

- Keep as fallback/advanced OCR because it is mature and local, but verify build times, cache behavior, static/dynamic CRT, and release packaging.
- If `tesseract-rs` becomes too brittle, alternatives are shipping a prebuilt Tesseract distribution or switching fallback OCR to a pure-Rust/ONNX path such as `ocrs`/RTen.

#### PaddleOCR (`paddle_ocr`)

Current Mnema details:

- Crate: `ocr-rs = 2.2.2`, backed by MNN.
- Runtime model bundle: `det/model.mnn`, `rec/model.mnn`, `rec/charset.txt` under `ocr-models/paddle_ocr/en-ppocrv5-mobile/`.
- Mnema uses PP-OCRv5 English mobile detector/recognizer artifacts pinned from `zibo-chen/rust-paddle-ocr`.

Windows research:

- `ocr-rs` README says prebuilt MNN static libraries are downloaded by default and lists Windows x86_64/i686 as supported prebuilt targets.
- Build still compiles a small C++ wrapper and runs bindgen, so Windows build machines need MSVC and libclang/LLVM.
- GPU features exist (`opencl`, `vulkan`, `cuda`) but Mnema should start CPU-only.

Windows recommendation:

- Best first OCR default for Windows 11 because it avoids OS package identity and is already app-managed.
- Add Windows CI/build verification for `ocr` with `paddle-rs` and check whether the prebuilt MNN CRT mode conflicts with the Tauri/Rust MSVC build.

### Audio transcription

All local transcription providers currently need the same missing Windows primitive: **decode an audio segment file to mono `f32` PCM at 16 kHz**. The current implementation uses AVFoundation and returns a hard error on non-macOS.

#### Apple Speech (`apple_speech_on_device`)

- Current use: Speech.framework on macOS, on-device mode, permission-gated.
- Windows 11: not available.
- Windows built-in `Windows.Media.SpeechRecognition` exists, but it is geared toward app speech input/dictation/grammar constraints and microphone permission UX, not Mnema's offline file-transcription processor. Docs also note only custom grammar constraints are performed on-device.
- Action: hide/disable Apple Speech on Windows; do not choose Windows SpeechRecognition as a default file-transcription replacement without a separate spike.

#### Local Whisper (`local_whisper`)

Current Mnema details:

- Crate: `whisper-rs = 0.16` over whisper.cpp.
- App-managed GGML models from `ggerganov/whisper.cpp`: tiny/base/small/medium.
- macOS build uses Metal; non-macOS build uses `whisper-rs` without Metal.
- Provider options include model path, language, and Whisper's token timestamp settings.

Windows research:

- `whisper-rs` docs include Windows build instructions for MSYS2/MinGW and Visual Studio C++ builds. CMake is required; LLVM/clang may be needed.
- `whisper-rs` has optional CUDA/Vulkan features for later, but CPU should be first.

Windows recommendation:

- Use Local Whisper as the first Windows transcription default because model sizes are predictable and smaller than Parakeet. Start with `base` or `small`; allow `tiny` for low-resource machines.
- Implement a Windows audio decoder, then verify sample-rate conversion, no-speech behavior, model cache/unload, and long-segment runtime.

#### Parakeet ONNX (`parakeet`)

Current Mnema details:

- Crate: `ort = 2.0.0-rc.12` over ONNX Runtime.
- Model options:
  - full `parakeet-tdt-0.6b-v3-onnx`: ~2.55 GB of model files,
  - int8 `parakeet-tdt-0.6b-v3-onnx-int8`: ~670 MB.
- Current code runs CPU ONNX sessions and has memory modes: performance, balanced idle-unload, low-memory.

Windows research:

- `ort` supports downloading ONNX Runtime binaries for Windows x86_64/aarch64 MSVC and can copy runtime DLLs.
- ONNX Runtime has a DirectML execution provider on Windows 10 1903+ and Windows 11, but Mnema's current Parakeet code uses CPU only.

Windows recommendation:

- Keep as an optional higher-quality local transcription provider, with **int8** as the practical Windows default model if offered.
- Do not enable DirectML in v1; first verify CPU correctness and package `onnxruntime.dll` behavior. Then benchmark DirectML separately.

### Speaker diarization / recognition

#### Sherpa ONNX (`sherpa_onnx`)

Current Mnema details:

- Crate: `sherpa-onnx = 1.13.1` with default static prebuilt libraries.
- Model bundle:
  - `pyannote-segmentation-3.0/model.onnx`,
  - `nemo_en_titanet_small.onnx`.
- Provider performs diarization, speaker embeddings, clustering, cross-chunk merge, and optional person recognition suggestions.
- Desktop uses a subprocess helper (`MNEMA_SPEAKER_ANALYSIS_HELPER=1`) to isolate heavy Sherpa work.

Windows research:

- `sherpa-onnx` upstream README lists Windows x64/x86/arm64 support and speaker diarization support.
- `sherpa-onnx-sys` build script downloads prebuilt Windows x64 static or shared release archives.

Windows recommendation:

- Keep Sherpa ONNX for Windows speaker analysis.
- Implement shared Windows audio decode to 16 kHz mono `f32` and test the subprocess helper on Windows (`current_exe`, env var, stdin/stdout JSON, timeout/kill-on-drop).
- Keep CPU provider initially; GPU/DirectML is not required for the current Sherpa path.

### VAD and system-audio speech activity

Current Mnema details:

- `capture-vad` has Silero and WebRTC adapters plus peak-level fallback.
- `SystemAudioSpeechActivityProcessorBackend` decodes an audio file, then runs `AudioSpeechDetectorRuntime` over mono PCM.
- Silero uses the `silero` crate, which itself uses `ort`; WebRTC uses `webrtc-vad`.

Windows research:

- `silero` exposes DirectML/CUDA/OpenVINO/etc. features through `ort`, but default bundled CPU should be enough.
- `webrtc-vad` is a small native VAD module and supports the standard 8/16/32/48 kHz, 10/20/30 ms PCM frame contract.

Windows recommendation:

- Keep both detectors and the current fallback behavior.
- The only blocking item is shared audio-file decode to mono PCM.

## Media file processing requirements for Windows

These are the concrete media primitives Windows must provide before the processors are useful.

### 1. Audio file decode to mono PCM

Needed by:

- Local Whisper,
- Parakeet,
- Sherpa speaker analysis,
- system-audio speech activity.

Current macOS behavior:

- `crates/audio-transcription/src/macos_audio_decode.rs` and `crates/speaker-analysis/src/macos_audio_decode.rs` decode with `AVAudioFile`, falling back through `AVAssetReader` + temporary WAV.
- `crates/capture-writers/src/lib.rs::decode_audio_file_to_mono_pcm` does similar for system-audio speech detection.

Windows alternatives:

1. **Media Foundation Source Reader** (recommended first): native Windows, decodes common captured `.m4a`/`.mp4`/AAC/WAV formats to PCM, aligns with Media Foundation writer/preview work.
2. **Symphonia**: pure Rust audio demux/decode. Use features `isomp4`, `aac`, `wav`, maybe `mp3` if imports are later supported. Good for processor-only decode; not a video preview solution.
3. **FFmpeg**: broadest coverage and easiest trim/extract, but heavier distribution/legal/update surface.

Recommended API seam:

```text
AudioDecodeBackend::decode_to_mono_f32(path) -> { samples: Vec<f32>, sample_rate_hz: u32 }
```

Then reuse the existing in-crate linear resamplers or move resampling to a shared helper (possibly `rubato` later if quality/perf matters).

### 2. Audio trim/convert/finalization

Needed by:

- user pause/inactivity tail trimming,
- microphone/system-audio finalization,
- retention of valid `.m4a` artifacts.

Current macOS behavior:

- AVAssetWriter writes audio.
- `/usr/bin/afconvert` converts recording audio to `.m4a` in one path.
- `ffmpeg` trims audio in `trim_audio_file_to_m4a`.

Windows alternatives:

1. **Media Foundation Sink Writer** with AAC encoder and MPEG-4 file sink (`.m4a`/`.mp4`) for native output.
2. For early prototypes, write `.wav` PCM and convert later, but this increases disk usage and changes output assumptions.
3. FFmpeg fallback only if Media Foundation trimming is too expensive.

### 3. Video decode and frame extraction

Needed by:

- exact frame preview fallback,
- scrub preview generation,
- finalized video validation,
- frame-index verification/extraction if frame timestamps need reconciliation after finalization.

Current macOS behavior:

- `AVAssetImageGenerator` extracts exact and scrub previews.
- `AVAssetReader` is used to inspect finalized video timing and build the binary frame-index sidecar.
- Preview images are encoded with ImageIO, preferring WebP for exact preview and JPEG for scrub previews.

Windows alternatives:

1. **Media Foundation Source Reader** for video decode/seeking to NV12/BGRA frames.
2. Encode preview images with the existing Rust `image` crate once frames are in CPU memory, or use Windows Imaging Component (WIC) if native image encoding is preferred.
3. FFmpeg fallback if exact seeking/frame extraction via Media Foundation is too slow to implement.

### 4. Screen frame artifact export for OCR

Needed by OCR, duplicate detection, frame batches, and exact captured-frame inspection.

Current macOS behavior:

- ScreenCaptureKit frame callbacks export JPEG artifacts to the hidden segment workspace.
- `crates/capture-screen` computes captured-frame equivalence directly from frame pixels when possible.

Windows recommendation:

- From Windows Graphics Capture/D3D frames, copy/convert to CPU BGRA/RGBA only at the configured OCR export interval.
- Use `image` crate JPEG encode first unless performance says WIC is needed.
- Preserve current artifact naming and frame identity parsing so OCR/model cleanup/search code stays stable.

## Model and packaging implications

| Component | Runtime native bits | Model/data bits | Windows packaging concern |
| --- | --- | --- | --- |
| PaddleOCR | MNN static prebuilt + C++ wrapper | 3 small app-managed files, ~8.7 MB | Needs MSVC + libclang at build; verify CRT/static library compatibility. |
| Tesseract | Tesseract + Leptonica from `tesseract-rs` build | tessdata files, ~23 MB in Mnema manifest | Build from source may be slow/brittle; verify CMake/NMake and cache location. |
| Local Whisper | whisper.cpp static build | GGML model single file, 78 MB-1.5 GB | Needs CMake/C++; choose default model carefully. |
| Parakeet | ONNX Runtime DLLs via `ort` | int8 ~670 MB or full ~2.55 GB | Package/copy `onnxruntime.dll`; memory-mode defaults matter. |
| Sherpa speaker | prebuilt sherpa static/shared libs | segmentation + embedding ~47 MB | Verify Windows static archive download/link; subprocess helper behavior. |
| Silero VAD | ONNX Runtime via `ort` | bundled Silero model in crate | Usually hidden by `silero` crate; verify packaged ORT DLL if shared. |
| WebRTC VAD | native C VAD from crate | none | Verify MSVC build. |
| Media Foundation backend | OS APIs/DLLs | none | Native OS dependency, no model distribution. |
| FFmpeg fallback | external binary or FFmpeg DLLs | none | LGPL/GPL compliance, updater/security surface, binary size. |

## Suggested Windows tracer bullets for processing

1. **Provider gating:** hide Apple Vision and Apple Speech on Windows; surface Paddle/Tesseract/Whisper/Parakeet/Sherpa availability accurately.
2. **Audio decode seam:** implement Windows `decode_audio_file_to_mono_pcm` once and wire it into Whisper, Parakeet, Sherpa, and system-audio speech activity.
3. **PaddleOCR smoke test:** run Windows x64 `cargo check`/packaged app with `ocr-rs` and app-managed PP-OCRv5 bundle; OCR one exported JPEG.
4. **Whisper smoke test:** decode captured `.m4a` -> 16 kHz mono -> Whisper base model -> transcript segments.
5. **Sherpa smoke test:** decode captured `.m4a` -> Sherpa helper subprocess -> speaker turns/clusters.
6. **Media Foundation preview spike:** seek an `.mp4`/`.mov` frame by timestamp, emit JPEG, compare against current `get_frame_preview`/scrub expectations.
7. **Tesseract fallback smoke test:** verify `tesseract-rs` build/package and tessdata install path on Windows.
8. **CI:** add Windows x64 MSVC checks with CMake + LLVM installed; cache native downloads if build times are high.

## Main risks / open decisions

- **Output container choice:** Windows runtime-capture research recommends `.mp4` for screen and `.m4a` for audio. Existing code assumes `.mov` in several places; if Windows uses `.mp4`, ensure DB/schema/UI paths do not hard-code `.mov` semantics beyond macOS.
- **Package identity:** Windows.Media.Ocr requires package identity, so it does not fit a normal Tauri NSIS/MSI default.
- **Native build toolchain:** `ocr-rs`, `tesseract-rs`, and `whisper-rs` all have native build requirements. Release should probably prebuild/cache artifacts rather than making every developer build from scratch.
- **Large models:** Parakeet full is too large for a default first-run path. Prefer Whisper base/small or Parakeet int8 as opt-in.
- **ORT DLL placement:** `ort` can copy DLLs for development, but Tauri packaging needs explicit verification.
- **FFmpeg legal surface:** FFmpeg is LGPL by default but can become GPL depending on enabled components. If we ship it, treat it as a product/legal decision rather than a hidden helper.
- **Current non-mac stubs:** audio decode and video preview functions return explicit macOS-only errors today. Windows processors will not be useful until those seams are implemented.

## Sources checked

Project sources:

- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/app_infra.rs`
- `apps/desktop/src-tauri/src/app_infra/frame_preview.rs`
- `apps/desktop/src-tauri/src/speaker_analysis_runtime.rs`
- `crates/ocr/*`
- `crates/audio-transcription/*`
- `crates/speaker-analysis/*`
- `crates/capture-vad/*`
- `crates/capture-writers/src/lib.rs`
- `crates/app-infra/src/processing/system_audio_speech_activity.rs`

Online / crate docs:

- Microsoft Learn: `Windows.Media.Ocr` namespace and `OcrEngine` — https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr
- Microsoft Learn: `Windows.Media.SpeechRecognition` namespace and Speech recognition UX — https://learn.microsoft.com/en-us/uwp/api/windows.media.speechrecognition and https://learn.microsoft.com/en-us/windows/apps/design/input/speech-recognition
- Microsoft Learn: Media Foundation Source Reader and audio decode tutorial — https://learn.microsoft.com/en-us/windows/win32/medfound/source-reader and https://learn.microsoft.com/en-us/windows/win32/medfound/tutorial--decoding-audio
- Microsoft Learn: Media Foundation Sink Writer, MPEG-4 file sink, H.264 encoder, AAC encoder, supported formats — https://learn.microsoft.com/en-us/windows/win32/medfound/sink-writer, https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink, https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder, https://learn.microsoft.com/en-us/windows/win32/medfound/aac-encoder, https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation
- ONNX Runtime DirectML Execution Provider — https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- FFmpeg legal notes — https://ffmpeg.org/legal.html
- crates.io/docs.rs / crate READMEs: `tesseract-rs`, `ocr-rs`, `whisper-rs`, `sherpa-onnx`, `ort`, `silero`, `webrtc-vad`, `symphonia`, `rubato`, `ocrs`, `ffmpeg-next`, `ffmpeg-sidecar`
