# Platform Support

_Last reviewed: 2026-06-08_

This file tracks Mnema platform-specific implementation status. It is intentionally implementation-facing: it names the OS-owned capabilities that must exist behind Mnema's shared capture, processing, privacy, storage, and release seams.

## Legend

- `[x]` implemented / usable in the current app
- `[ ]` not implemented yet
- `[~]` partially implemented, stubbed, or supported only through a platform-specific fallback

## Current support summary

| Area | macOS | Windows | Linux | Notes |
| --- | --- | --- | --- | --- |
| Tauri desktop shell | [x] | [~] | [~] | Shell is mostly cross-platform, but window chrome/dock behavior has macOS-specific paths. |
| Native screen capture | [x] | [~] | [ ] | macOS uses ScreenCaptureKit / AVFoundation fallback. Windows uses WGC for primary-monitor screen capture. |
| Native microphone capture | [x] | [~] | [ ] | macOS uses AVFoundation. Windows captures selected/default WASAPI endpoints, tracks device/default changes, encodes AAC `.m4a` via Media Foundation (capture-and-store; no audio processing yet), and gracefully surfaces a blocked microphone with a `ms-settings:privacy-microphone` deep link. |
| Native system-audio capture | [x] | [~] | [ ] | macOS uses ScreenCaptureKit and currently requires screen capture. Windows uses independent WASAPI loopback when a default render endpoint is available. |
| Capture segment lifecycle | [x] | [~] | [ ] | Lifecycle is generic in shape; Windows now drives screen, microphone, and independent system-audio segment rotation off the shared `CaptureClock`/`SegmentSchedule`. |
| Media writers/finalization | [x] | [~] | [ ] | macOS uses AVAssetWriter, AVFoundation, `afconvert`, and some `ffmpeg` trim paths. Windows uses Media Foundation for H.264 `.mp4` screen output and AAC `.m4a` microphone/system-audio output, with an MF Source Reader positive-duration `.m4a` validator. |
| Screen frame export / frame index | [x] | [x] | [ ] | Windows writes ~1 fps JPEG frame artifacts and, at segment finalization, a binary frame-index sidecar in the same on-disk format macOS emits (offsets pushed live from capture-thread tick offsets; macOS-shared monotonicity check). An index-less segment degrades to exact-preview-only, never scrub-eligible. The MF decode-based rebuild-from-video recovery path stays unported. |
| Exact frame preview from video | [x] | [x] | [ ] | macOS uses AVAssetImageGenerator. Windows extracts the exact frame through the `crates/media-decode` MF Source Reader video seam (ADR 0024: seek-to-keyframe + decode-forward to the target offset), then JPEG-encodes it (JPEG-only on Windows v1, no WebP); the `image/jpeg` MIME flows through the preview result so consumers never assume a format. On-device extraction from a real captured segment is deferred to an operator. |
| Scrub preview generation | [x] | [x] | [ ] | macOS uses AVAssetImageGenerator. Windows generates the same JPEG q72 / 360px scrub rendition through the `crates/media-decode` MF video seam (frame-index sidecar + seek/decode-forward), reusing the shared eligibility, cache keying, queue, and coalesced cache-change events with no platform fork beyond the extraction backend. On-device per-second scrub verification is deferred to an operator. |
| OCR: Apple Vision | [x] | [ ] | [ ] | Apple-only provider. |
| OCR: Tesseract/PaddleOCR | [x] | [~] | [~] | Cross-platform intent, but Windows/Linux packaging/runtime need verification. |
| Audio transcription: Apple Speech | [x] | [ ] | [ ] | Apple-only provider. |
| Audio transcription: Local Whisper | [x] | [x] | [~] | macOS decodes via AVFoundation; Windows decodes a captured `.m4a` through the `media-decode` MF Source Reader seam (ADR 0024), resamples to 16 kHz mono in-crate, and runs whisper.cpp CPU-only. No-speech segments complete as empty (successful) transcriptions. On-device decode of real captured audio is deferred to an operator. |
| Audio transcription: Parakeet | [x] | [x] | [~] | macOS offers both the full and int8 ONNX variants; Windows offers only the int8 `parakeet-tdt-0.6b-v3-onnx-int8` model (the full variant is hidden at the manifest source of truth), decodes a captured `.m4a` through the `media-decode` MF Source Reader seam (ADR 0024), resamples to 16 kHz mono in-crate, and runs the `ort` ONNX adapter CPU-only (ORT statically linked, no `onnxruntime.dll` runtime dependency; DirectML/GPU deferred). A selected-but-unavailable model behaves like a missing model. On-device decode of real captured audio is deferred to an operator. |
| Speaker analysis | [x] | [x] | [~] | macOS decodes via AVFoundation; Windows decodes a captured `.m4a` through the `media-decode` MF Source Reader seam (ADR 0024), resamples to 16 kHz mono in-crate, and runs the Sherpa ONNX diarization/embedding runtime CPU-only in the isolation subprocess helper. `sherpa-onnx` links via ORT's `shared` feature and ships its own `onnxruntime.dll`. On-device diarization of a real multi-speaker recording is deferred to an operator. |
| Audio decode to mono PCM (`media-decode` seam) | [x] | [~] | [ ] | macOS keeps its AVFoundation decoders in-crate. Windows decodes `.m4a`/`.mp4` audio to native-rate mono `f32` via the `crates/media-decode` MF Source Reader seam (ADR 0024). Transcription/speaker-analysis still resample in-crate; only system-audio speech activity is wired through the seam so far. |
| System-audio speech activity | [x] | [~] | [ ] | macOS decodes via AVFoundation; Windows decodes through the `media-decode` MF Source Reader seam and runs the same VAD runtime. On-device decode of a captured Windows `.m4a` is deferred to an operator. |
| Inactivity detection | [x] | [~] | [ ] | macOS uses CoreGraphics input idle plus capture-sourced screen/audio activity. Windows uses system-input snapshots plus capture-sourced screen/microphone/system-audio activity for inactivity pause/resume. |
| Sleep/wake/session-lock recovery | [x] | [~] | [ ] | macOS uses AppKit/NSWorkspace + ScreenCaptureKit liveness. Windows transient-liveness recovery currently covers display-unavailable and session-lock screen suspension while audio continues. |
| Live app privacy exclusion | [x] | [ ] | [ ] | macOS uses ScreenCaptureKit app exclusion filters. Windows/Linux semantics need design. |
| Active app/window metadata | [x] | [ ] | [ ] | macOS uses NSWorkspace/CoreGraphics. |
| Browser URL metadata | [x] | [ ] | [ ] | macOS uses AppleScript for supported browsers. |
| Recommended app exclusions | [x] | [ ] | [ ] | Current catalog uses macOS bundle IDs. |
| Status bar / tray | [x] | [~] | [~] | Tauri tray exists cross-platform; current UX includes macOS-only Exclude Current App behavior. |
| Global shortcuts | [x] | [~] | [~] | Uses Tauri global shortcut plugin for background start/stop, pause/resume, and show/hide; platform behavior needs verification. |
| Encrypted Capture Index key store | [x] | [x] | [ ] | macOS uses Keychain. Windows uses Credential Manager. Linux platform key store is missing. |
| Broker Authorization Channel | [x] | [ ] | [~] | Unix socket implementation works for macOS/Linux shape; Windows needs named pipe/TCP/etc. |
| CLI sidecar build | [x] | [~] | [~] | Script has target-aware `.exe` handling, but packaging/release not verified. |
| Release/updater pipeline | [x] | [ ] | [ ] | Current release workflow ships Apple Silicon macOS only. |

## macOS checklist

### Runtime capture

- [x] Screen capture via ScreenCaptureKit on macOS 15+.
- [x] AVFoundation screen fallback for older macOS backend constraints.
- [x] Microphone capture via AVFoundation.
- [x] System-audio capture through ScreenCaptureKit.
- [x] Segment rotation and finalization for screen, microphone, and system audio.
- [x] User capture pause/resume.
- [x] Inactivity pause/resume for screen, microphone, and system audio families.
- [x] Sleep/wake recovery for screen/system-audio while preserving microphone continuation.
- [x] Screen/system-audio liveness reconciliation from AppKit wake notifications and ScreenCaptureKit stream delegate failures.
- [x] Display-unavailable recovery: a display sleep/lock/lid-close/disconnect surfaced as a `DisplayUnavailable` privacy-filter apply error suspends screen/system-audio (preserving microphone continuation) and auto-resumes when a display returns, instead of failing the session.
- [x] Screen frame export, captured-frame equivalence, OCR batching, and frame-index sidecars.

### Media and processing

- [x] `.mov` screen segment output.
- [x] `.m4a` microphone/system-audio output.
- [x] Video-only screen finalization when system audio was captured.
- [x] Exact frame preview extraction from finalized video.
- [x] Scrub preview generation from finalized indexed segments.
- [x] Apple Vision OCR provider.
- [x] Tesseract/PaddleOCR provider integration.
- [x] Apple Speech provider.
- [x] Local Whisper/Parakeet provider integration using AVFoundation decode.
- [x] Speaker analysis using AVFoundation decode.
- [x] System-audio speech activity using AVFoundation-backed audio decode.

### Privacy, metadata, and UX

- [x] Screen and microphone permission checks/prompts.
- [x] Open macOS Privacy & Security panes for denied permissions.
- [x] Active app/window metadata from NSWorkspace/CoreGraphics.
- [x] Browser URL metadata for supported browsers via AppleScript.
- [x] Live App Privacy Exclusion through ScreenCaptureKit app filters.
- [x] Exclude Current App tray action.
- [x] Recommended sensitive app exclusion catalog using macOS bundle IDs.
- [x] Native status-bar tray menu.
- [x] Dock visibility and macOS terminate handling.
- [x] Global shortcuts through Tauri plugin for background start/stop recording, pause/resume recording, and show/hide Mnema.

### Storage, access, and release

- [x] Encrypted Capture Index key stored in macOS Keychain.
- [x] Broker Authorization Channel over per-user Unix socket.
- [x] Deep-link app reopen fallback.
- [x] Mnema CLI sidecar for Apple targets.
- [x] macOS release workflow for Apple Silicon.
- [ ] Developer ID signing and notarization.
- [ ] Universal or Intel macOS release, if needed.

## Windows checklist

Research notes:

- `docs/windows/runtime-capture-research.md` tracks recommended Windows capture APIs and crate options.
- `docs/windows/media-processing-research.md` tracks OCR/transcription/speaker/media-processing provider choices and alternatives.
- `docs/windows/permissions-privacy-metadata-research.md` tracks Windows permission, privacy, app identity, and metadata alternatives.
- `docs/windows/storage-access-release-research.md` tracks Windows 11 storage paths, key-store/CLI access choices, and release/updater options.

### Bring-up / compilation

- [~] Add Windows CI job for `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`, `cargo check --workspace --all-targets`, and `bun run check`. The `windows-check` job in `.github/workflows/type-check.yml` runs `cargo check --workspace --all-targets` plus `cargo test` of the platform-neutral crates; the manifest-path check and `bun run check` are still outstanding.
- [ ] Audit all `cfg(target_os = "macos")` / non-mac stubs and ensure Windows builds cleanly with the desktop feature set.
- [ ] Decide whether capture output remains `.mov`/`.m4a` or becomes Windows-native formats with schema/runtime support for extensions.
- [ ] Remove user-facing “only macOS” errors once Windows adapters exist.
- [ ] Add Windows Tauri bundle config, installer target, signing plan, and updater artifacts.

### Runtime capture

- [~] Implement a Windows screen capture backend behind `crates/capture-screen`.
  - Candidate APIs: Windows Graphics Capture or DXGI Desktop Duplication.
  - WGC primary-monitor screen capture exists with frame timing, segment rotation, stop/error reporting, frame export, resolution scaling, in-session frame-pool recreation for resolution/DPI/display-mode changes, H.264 bitrate control, and screen activity samples for inactivity decisions; broader source support remains outstanding.
- [~] Implement Windows microphone capture behind `crates/capture-microphone`.
  - Active WASAPI capture endpoint enumeration, selected/default endpoint capture, default-device tracking, `IMMNotificationClient` device-change notifications, and `FallbackToDefault` / `WaitForSameDevice` reconnect policy are implemented.
  - The microphone and system-audio support/permission gates re-probe the default WASAPI endpoint on every call (no process-lifetime latch), so an endpoint hot-plugged after launch becomes recordable — and the tray/settings source gating refreshes — without an app restart (`smoke_support_hotplug` example covers the HITL check).
  - Best-effort permission UX is implemented: a blocked microphone (Windows privacy denial surfacing as `E_ACCESSDENIED` at WASAPI `IAudioClient` activation) is mapped to the recoverable `microphone_access_denied` error at capture start, which raises an app notification deep-linking to `ms-settings:privacy-microphone`. The `microphone` permission reports as `Unknown` (best-effort) since per-app privacy cannot be queried synchronously.
  - The microphone capture callback emits a VAD PCM feed and peak-since-last-poll Audio Activity Samples (debug-visible raw samples via the `get_idle_debug` surface); those samples feed Windows per-family inactivity pause/resume decisions.
  - Still outstanding: audio decode/processing and broader audio device-disconnect smoke coverage.
- [~] Implement Windows system-audio capture.
  - WASAPI loopback captures the default render endpoint as an independent source when the support probe succeeds, tracks default render-endpoint changes via `IMMNotificationClient`, re-attaches loopback mid-recording when the default changes, and emits audio activity samples for inactivity decisions.
  - System audio is modeled as an independent source on Windows (ADR 0022); backend source validation allows system-audio-only sessions instead of requiring screen capture.
  - `get_capture_support.systemAudioRequiresScreen` is false on Windows and true on macOS; settings, onboarding, and the native tray use that capability so Windows users can keep/select system audio when screen is unchecked.
  - On-device default-render switch smoke coverage was verified on Windows 11 by switching from LG ULTRAGEAR to Realtek Digital Output and back during loopback capture; both rotated `.m4a` segments remained openable with positive duration.
- [~] Implement Windows capture session IDs and source-session bookkeeping. Screen and microphone `SourceSessions` metadata are populated; Windows system audio uses `sysaudio_session`-prefixed source ids when loopback capture is active.
- [~] Implement segment rotation without dropping OCR/frame-index invariants. Screen, microphone, and independent system audio rotate on the shared 5-minute boundary; an audio-only session rotates without a screen planner/session.
- [x] Implement inactivity pause/resume for each requested source family. Windows pauses screen, microphone, and independent system-audio families after the inactivity threshold and resumes only the paused families when system input, screen, microphone, or system-audio activity returns; whole-runtime inactivity pause/resume uses the same segment-start path so resumed segments keep shared `CaptureClock` / `SegmentSchedule` boundaries and segment naming.
- [~] Implement capture liveness/error propagation equivalent to ScreenCaptureKit delegate stop errors. Windows now treats primary-monitor display-unavailable transitions as transient liveness instead of fatal failure: `DisplayUnavailable` from monitor/display loss (#62) and `SessionLock` from workstation lock (#63) suspend screen capture only, keep the native session running, and auto-resume screen through the shared transient-liveness/start-segment path when the display returns or the session unlocks.
- [~] Implement sleep/wake/session-lock recovery. Session lock on Windows is supported as `TransientLiveness { SessionLock }`: screen is paused, microphone remains active/not paused, independent system audio remains active/not paused when requested, audio source-session ids are preserved, and unlock resumes screen automatically. Broader OS sleep policy is still outstanding.

### Media writers and previews

- [~] Implement cross-platform or Windows-specific audio/video writers.
  - Candidate APIs: Media Foundation, FFmpeg, GStreamer, or a Rust media pipeline.
- [x] Honor Windows screen resolution presets/custom dimensions by scaling WGC frames during CPU BGRA -> NV12 conversion and JPEG frame export.
- [x] Honor Windows screen bitrate presets/custom bitrate through Media Foundation H.264 `MF_MT_AVG_BITRATE`.
- [x] Implement finalized video validation equivalent to “openable `.mov` with moov”. On a user stop, the finalized Windows `.mp4` screen segment is inspected through the `media-decode` MF Source Reader video seam (`inspect_video`): it must open, expose a decodable video stream, and report positive duration/dimensions — the MF analogue of the macOS `AVAssetReader` timing check. An invalid artifact is removed (mirroring the macOS validate-and-remove). This is stronger than the byte-level `moov` openability probe the preview layer uses.
- [x] Implement screen frame export to JPEG artifacts.
- [x] Implement frame-index sidecar generation. Index entries are accumulated live on the capture thread (each exported frame's segment-relative tick offset, which the encoder rebases to zero) and serialized to a binary sidecar at segment finalization in the same on-disk format macOS emits; monotonicity is enforced via the macOS-shared check. The decode-based rebuild-from-finalized-video recovery path stays unported, so an index-less segment is an exact-preview-only, never-scrub-eligible degradation.
- [x] Implement exact frame preview extraction from video. The `media-decode` MF Source Reader video seam negotiates the first video stream to CPU-readable `RGB32`, seeks to (or before) the frame-index/estimated target offset, decodes forward to the target timestamp (MF seeks land on keyframes), and returns straight RGBA pixels; the exact-preview fallback JPEG-encodes them (JPEG-only on Windows v1) and flows the `image/jpeg` MIME. On-device extraction from a real captured segment is deferred to an operator.
- [ ] Implement scrub preview batch generation.
- [~] Implement audio trim/convert/finalization for microphone and system-audio outputs. Windows microphone and system-audio `.m4a` outputs are finalized and validated via the MF Source Reader positive-duration probe. Inactivity-tail trim is implemented writer-side: both sessions hold back the last N seconds of PCM ahead of the AAC sink writer and discard that tail on an inactivity stop (flush it on a normal stop or rotation), with the trim boundary refined by peak level or VAD speech, mirroring the macOS asset-writer hold-back; general re-encode trim/convert remains outstanding.
- [ ] Implement video-only screen output finalization when audio is muxed or recorded together.

### Processing

- [x] Add the `crates/media-decode` MF Source Reader seam decoding `.m4a`/`.mp4` audio to native-rate mono `f32` (ADR 0024); processing crates depend on it, no capture crate grows a decoder. On-device decode of a captured Windows `.m4a` is deferred to an operator.
- [x] Wire Local Whisper through the `media-decode` seam on Windows: a captured `.m4a` decodes via the MF Source Reader seam, resamples to 16 kHz mono in-crate, and transcribes with whisper.cpp CPU-only (`base` default). No-speech segments complete as successful empty transcriptions. On-device decode of real captured audio is deferred to an operator.
- [x] Wire Parakeet through the `media-decode` seam on Windows. Windows offers only the int8 `parakeet-tdt-0.6b-v3-onnx-int8` model (the full variant is platform-gated out of `builtin_model_manifest`, so the backend status and the capability-driven UI picker agree, and a setting that still names the full model behaves like a missing model — segment stays eligible, no job until backfill — not an error). A captured `.m4a` decodes via the MF Source Reader seam, resamples to 16 kHz mono in-crate, and transcribes with the `ort` ONNX Runtime adapter CPU-only. ONNX Runtime is statically linked by `ort` (`download-binaries`), so the packaged Windows app carries the runtime inside `mnema.exe` and needs no separate `onnxruntime.dll`; DirectML/GPU stays deferred. On-device decode of real captured audio is deferred to an operator.
- [x] Wire speaker analysis through the `media-decode` seam on Windows. A captured `.m4a` decodes via the MF Source Reader seam, resamples to 16 kHz mono in-crate, and runs the Sherpa ONNX diarization/embedding runtime CPU-only inside the `MNEMA_SPEAKER_ANALYSIS_HELPER` isolation subprocess (the helper resolves `current_exe`, round-trips request/response JSON as raw bytes over stdin/stdout, and enforces a timeout with `kill_on_drop`). `sherpa-onnx` links via ORT's `shared` feature and ships its own `onnxruntime.dll` (separate from the statically-linked Parakeet `ort` path; see #80). DirectML/GPU stays deferred. On-device diarization of a real multi-speaker recording is deferred to an operator.
- [x] Wire system-audio speech activity through the `media-decode` seam on Windows; decoded mono PCM runs the same VAD runtime as macOS. On-device verification deferred to an operator.
- [ ] Verify packaged Tesseract/PaddleOCR runtimes and model installation on Windows.
- [ ] Choose Windows OCR default provider.
- [ ] Choose Windows transcription default provider.
- [ ] Hide/disable Apple Vision and Apple Speech providers on Windows.

### Permissions, privacy, and metadata

- [x] Implement microphone permission/status UX and settings deep link, e.g. `ms-settings:privacy-microphone`. Best-effort: capture-start access denial (`E_ACCESSDENIED`) maps to the recoverable `microphone_access_denied` error and an app notification that deep-links to `ms-settings:privacy-microphone`; `microphone` permission reports as `Unknown`. No full permission state machine (per decision #6).
- [ ] Define Windows screen-capture permission/support semantics.
- [~] Define Windows system-audio permission/support semantics. Best-effort support is based on a non-prompting default render endpoint probe; per-app loopback privacy/denial still surfaces at capture start if Windows rejects activation.
- [x] Implement system idle detection via Windows activity snapshots backed by `GetLastInputInfo`.
- [ ] Implement active app/window metadata.
  - Candidate APIs: foreground window handle, process ID, process executable path, window title.
- [ ] Define Windows app identity model for privacy rules.
  - Candidate identifiers: executable path, process name, AppUserModelID, package family name.
- [ ] Implement app candidate discovery and icon materialization.
- [ ] Design live app privacy exclusion semantics.
  - If per-app exclusion is not possible, expose clear unsupported/degraded UX and rely on pause/delete-recent/app disclosure.
- [ ] Implement “Exclude Current App” using Windows active-window identity, or hide/disable it.
- [ ] Add Windows sensitive app recommendation catalog.
- [ ] Add Windows known-browser catalog and browser capture disclosure.
- [ ] Decide whether browser URL metadata is supported on Windows; do not add browser extension plumbing without an ADR.

### Storage, access, and release

- [x] Implement Windows Capture Index Key Store using Credential Manager, DPAPI, or another platform-owned secret store.
- [ ] Implement Broker Authorization Channel for Windows.
  - Candidate transports: named pipes, localhost loopback, or another app-mediated IPC.
- [ ] Update `crates/cli` authorization request path for Windows.
- [ ] Verify deep links and app launch fallback on Windows.
- [ ] Verify tray/menu behavior on Windows.
- [ ] Verify global shortcut registration and default shortcut labels on Windows.
- [ ] Add Windows release workflow and update manifest generation.
- [ ] Add Windows install/update signing process.

## Linux checklist

Linux support is not the immediate target, but these are the likely seams if/when Mnema expands beyond macOS/Windows.

### Bring-up / compilation

- [ ] Add Linux CI job for Rust and frontend checks.
- [ ] Audit macOS-only stubs for acceptable Linux unsupported behavior.
- [ ] Decide supported desktop environments/compositors: Wayland-first, X11 fallback, or both.
- [ ] Add Linux Tauri bundle config and release artifacts, e.g. AppImage/deb/rpm.

### Runtime capture

- [ ] Implement screen capture backend.
  - Candidate APIs: PipeWire + xdg-desktop-portal for Wayland, X11 capture fallback if supported.
- [ ] Implement microphone capture.
  - Candidate APIs: PipeWire/PulseAudio/ALSA via CPAL or native bindings.
- [ ] Implement system-audio/loopback capture.
  - Candidate APIs: PipeWire monitor streams or PulseAudio monitor sources.
- [ ] Implement segment rotation, liveness, sleep/wake/session-lock recovery, and source pause/resume.
- [ ] Implement screen activity samples without excessive compositor/GPU cost.

### Media writers and previews

- [ ] Choose Linux media writer/extractor stack.
  - Candidate APIs: FFmpeg or GStreamer.
- [ ] Implement screen frame export and frame-index sidecars.
- [ ] Implement exact video-backed frame previews.
- [ ] Implement scrub preview generation.
- [ ] Implement audio decode/trim/convert for processing providers.

### Processing

- [ ] Verify Tesseract/PaddleOCR packaging on Linux.
- [ ] Add Linux audio decode for Local Whisper/Parakeet.
- [ ] Add Linux audio decode for speaker analysis.
- [ ] Add Linux audio decode for system-audio speech activity.
- [ ] Hide/disable Apple-only providers.

### Permissions, privacy, and metadata

- [ ] Define screen/microphone/system-audio permission UX for portals and desktop environments.
- [ ] Implement system idle detection.
  - Candidate APIs: xdg idle portal, compositor-specific APIs, or X11 screensaver extension.
- [ ] Implement active app/window metadata where available.
- [ ] Define Linux app identity model for privacy rules.
  - Candidate identifiers: desktop file ID, app ID, executable path, process name.
- [ ] Implement app candidate discovery and icons from desktop entries.
- [ ] Design live app privacy exclusion semantics for PipeWire/portal capabilities.
- [ ] Add Linux sensitive app and known-browser catalogs if metadata/disclosure is supported.

### Storage, access, and release

- [ ] Implement Linux Capture Index Key Store using Secret Service/libsecret/KWallet or a clear unsupported flow.
- [~] Broker Authorization Channel can likely reuse Unix socket shape, but needs Linux config-dir/runtime-dir review.
- [ ] Verify CLI sidecar packaging and app-mediated authorization flow on Linux.
- [ ] Verify tray/menu behavior across desktop environments.
- [ ] Verify global shortcuts; Linux support may depend on desktop environment/portal support.
- [ ] Add Linux release workflow and updater artifacts if supported.

## Current macOS-only source map

Use this map when turning checklist items into implementation slices.

| Source area | macOS-only implementation today | Windows/Linux replacement needed |
| --- | --- | --- |
| `crates/capture-screen/src/lib.rs` | ScreenCaptureKit/AVFoundation screen capture, system audio, permissions, stream liveness, frame export, video stripping, frame-index rebuild | Platform capture backend, permission semantics, writers, frame export/index, liveness/recovery |
| `crates/capture-microphone/src/lib.rs` | AVFoundation microphone capture, device list/change notifications, permission prompt, VAD PCM feed | WASAPI/CPAL/PipeWire/etc. microphone adapter, device policy, permission UX |
| `crates/capture-writers/src/lib.rs` | AVAssetWriter/AVAudioFile, `afconvert`, AVFoundation duration/decode helpers | Cross-platform writer, duration validation, decode/trim/convert |
| `apps/desktop/src-tauri/src/native_capture/*` | Runtime active sessions and lifecycle operations are mostly macOS-gated | Promote runtime fields/adapters to platform-neutral traits or add Windows/Linux gated implementations |
| `apps/desktop/src-tauri/src/native_capture_metadata.rs` | NSWorkspace, CoreGraphics window list, AppleScript browser URL probe | Foreground-window/app metadata and optional browser metadata per OS |
| `apps/desktop/src-tauri/src/native_capture/privacy.rs` | ScreenCaptureKit app exclusion filters | OS-specific live exclusion or explicit unsupported/degraded behavior |
| `apps/desktop/src-tauri/src/native_capture_system_idle.rs` | CoreGraphics idle time | `GetLastInputInfo` on Windows; portal/X11/compositor path on Linux |
| `apps/desktop/src-tauri/src/app_infra/frame_preview.rs` | AVAssetImageGenerator exact/scrub previews | Windows exact preview wired through the `media-decode` MF video seam (JPEG); scrub preview (#83) still needed on Windows/Linux |
| `crates/media-decode/src/lib.rs` | Windows MF Source Reader audio decode to native-rate mono `f32` plus video frame extraction (RGBA + JPEG) and finalized-video timing inspection (ADR 0024); shared processing seam | Cross-platform decode/extract seam; macOS decoders/AVAssetImageGenerator stay in their existing crates (out of scope here) |
| `crates/audio-transcription/src/macos_audio_decode.rs` | AVFoundation audio decode for Local Whisper/Parakeet | Migrate Windows decode to the `media-decode` seam; keep macOS in-crate |
| `crates/speaker-analysis/src/macos_audio_decode.rs` | AVFoundation audio decode for diarization/recognition | Migrate Windows decode to the `media-decode` seam; keep macOS in-crate |
| `crates/ocr/src/lib.rs`, `crates/app-infra/src/processing/apple_vision.rs` | Apple Vision OCR | Disable on non-Apple; default to Tesseract/PaddleOCR |
| `crates/audio-transcription/src/providers/apple_speech.rs` | Apple Speech | Disable on non-Apple; default to local providers/cloud if introduced |
| `crates/app-infra/src/capture_index_key_store.rs` | macOS Keychain through `security` CLI | Windows Credential Manager/DPAPI; Linux Secret Service/KWallet |
| `apps/desktop/src-tauri/src/broker_authorization_channel.rs`, `crates/cli/src/main.rs` | Unix socket app-mediated authorization | Windows named pipe/TCP; Linux runtime-dir Unix socket validation |
| `apps/desktop/src-tauri/src/windows.rs` | macOS rounded content views, Dock visibility, terminate interception | Windows/Linux window behavior equivalents or no-ops |
| `.github/workflows/*`, `docs/release-process.md`, `scripts/stage-macos-release-artifacts.sh` | macOS-only release pipeline | Windows/Linux release pipelines and docs |

## Cross-platform implementation principles

- Keep Tauri command handlers thin; native capture orchestration belongs behind the Recording Lifecycle seam in `apps/desktop/src-tauri/src/native_capture/lifecycle.rs` and platform capture crates.
- Keep shared serde/domain types in `crates/capture-types`.
- Keep capture primitives in `crates/capture-screen`, `crates/capture-microphone`, and `crates/capture-writers`; avoid putting OS APIs directly in Svelte or high-level Tauri commands.
- Preserve Capture Session / Capture Segment semantics across platforms, including date-organized output, source-session IDs, retention, OCR job enqueueing, and timeline events.
- Do not silently downgrade privacy. If an OS cannot support live app exclusion, expose an explicit unsupported/degraded capability and rely on App Privacy Exclusion disclosure, Pause Recording, and Delete Recent Capture recovery.
- Prefer capability-driven UI (`get_capture_support`, provider runtime availability, permission state) over direct platform checks in Svelte.
