# Platform Support

_Last reviewed: 2026-06-23_

This file tracks Mnema platform-specific implementation status. It is intentionally implementation-facing: it names the OS-owned capabilities that must exist behind Mnema's shared capture, processing, privacy, storage, and release seams.

## Legend

- `[x]` implemented / usable in the current app
- `[ ]` not implemented yet
- `[~]` partially implemented, stubbed, or supported only through a platform-specific fallback

## Current support summary

| Area | macOS | Windows | Linux | Notes |
| --- | --- | --- | --- | --- |
| Tauri desktop shell | [x] | [~] | [~] | Shell is mostly cross-platform, but window chrome/dock behavior has macOS-specific paths. |
| Native screen capture | [x] | [ ] | [ ] | macOS uses ScreenCaptureKit / AVFoundation fallback. |
| Native microphone capture | [x] | [ ] | [ ] | macOS uses AVFoundation. |
| Native system-audio capture | [x] | [ ] | [ ] | macOS uses ScreenCaptureKit and currently requires screen capture. |
| Capture segment lifecycle | [x] | [ ] | [ ] | Lifecycle is generic in shape but active runtime fields are macOS-gated. |
| Media writers/finalization | [x] | [ ] | [ ] | macOS uses AVAssetWriter, AVFoundation, `afconvert`, and some `ffmpeg` trim paths. |
| Screen frame export / frame index | [x] | [ ] | [ ] | Required for OCR, exact frame lookup, duplicate detection, and scrub previews. |
| Exact frame preview from video | [x] | [ ] | [ ] | macOS uses AVAssetImageGenerator. |
| Scrub preview generation | [x] | [ ] | [ ] | macOS-only today. |
| OCR: Apple Vision | [x] | [ ] | [ ] | Apple-only provider. |
| OCR: Tesseract/PaddleOCR | [x] | [~] | [~] | Cross-platform intent, but Windows/Linux packaging/runtime need verification. |
| Audio transcription: Apple Speech | [x] | [ ] | [ ] | Apple-only provider. |
| Audio transcription: Local Whisper/Parakeet | [x] | [~] | [~] | Models are cross-platform-ish, but audio decode is AVFoundation-only today. |
| Speaker analysis | [x] | [ ] | [ ] | On-device diarization runs through the `speakrs` provider (pure-Rust pyannote-community-1 segmentation + WeSpeaker embedding + VBx clustering on CoreML), which links system OpenBLAS. CoreML ties it to Apple Silicon macOS; audio decode is AVFoundation-only too. |
| Semantic Search (local embeddings + vec0) | [x] | [~] | [~] | Built and tested on macOS only. Embeddings run on-device via **candle** behind a pluggable **Semantic Search Backend**: the Apple GPU (Metal, F16) on macOS or **candle-CPU** (F32) elsewhere — the runtime tries Metal then falls back to CPU; CUDA is deferred (no v1 artifact). This replaces the earlier `fastembed`/ONNX (`ort`) path along with its per-thread QoS downclock and arena mitigations. `sqlite-vec` (`vec0`) is still statically linked into the SQLCipher amalgamation (unchanged), and model download is still a desktop-owned `reqwest` (now `rustls`) fetch from Hugging Face (now safetensors layout) — all cross-platform in principle and free of native capture/audio-decode dependencies. Note candle-metal is a **second native ML runtime** in the bundle (links Metal/MPS) alongside `ort`, which now remains only for transcription — flag for the release/notarization checklist. Windows/Linux are **unverified**: claiming support is gated on a **candle-CPU measurement** (CPU%, throughput, RSS) plus a real build/run of the candle-CPU + static `vec0` link on those platforms. See [ADR 0037](docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md). |
| Inactivity detection | [x] | [ ] | [ ] | macOS uses CoreGraphics input idle plus capture-sourced screen/audio activity. |
| Sleep/wake recovery | [x] | [ ] | [ ] | macOS uses AppKit/NSWorkspace + ScreenCaptureKit liveness. |
| Live app privacy exclusion | [x] | [ ] | [ ] | macOS uses ScreenCaptureKit app exclusion filters. Windows/Linux semantics need design. |
| Active app/window metadata | [x] | [ ] | [ ] | macOS uses NSWorkspace/CoreGraphics. |
| Browser URL metadata | [x] | [ ] | [ ] | macOS uses AppleScript for supported Chromium and WebKit browsers. Firefox-family (Gecko) browsers expose no scriptable URL and are not supported. |
| Recommended app exclusions | [x] | [ ] | [ ] | Current catalog uses macOS bundle IDs. |
| Quick Recall launcher panel | [x] | [~] | [~] | macOS summons a non-activating NSPanel (key without activating Mnema, like Spotlight/Raycast); non-macOS falls back to a plain shown/focused always-on-top window without non-activating semantics. |
| Ask AI (in-process Reasoning Engine) | [x] | [~] | [~] | Quick Recall + Insights Chat run in-process on the shared Reasoning Engine (`crates/ai-runtime` via `rig-core`) — no installed PI/Node runtime, no shim, no `node`/`pi`-on-PATH resolution. Brokered `search`/`timeline`/`show_text`/`recall_context` tools (plus presentation-only `reference_captures`) are injected from the Tauri layer under the All-Retained Ask AI broker scope. Cross-platform Rust like the engine itself; a cloud engine needs the bring-your-own-key in the Encrypted Capture Index key store (macOS Keychain only today, see that row), a local Ollama/Llamafile engine needs no key and is platform-agnostic. Windows/Linux are blocked only on the platform key store for cloud keys. |
| User Context / Reasoning Engine | [x] | [~] | [~] | Reasoning Engine + User Context derivation are cross-platform Rust via `rig-core` with no native capture dependency. A cloud engine (Anthropic/OpenAI bring-your-own-key) works on any platform with a key, but the key is stored in the **Encrypted Capture Index key store** (macOS Keychain only today, see that row). A local engine needs a running Ollama/Llamafile endpoint and no key, so it is platform-agnostic. Windows/Linux are blocked only on the platform key store for cloud keys; local-engine derivation should already work cross-platform. |
| Status bar / tray | [x] | [~] | [~] | Tauri tray exists cross-platform; current UX includes macOS-only Exclude Current App behavior. |
| Global shortcuts | [x] | [~] | [~] | Uses Tauri global shortcut plugin for background start/stop, pause/resume, and show/hide; platform behavior needs verification. |
| Encrypted Capture Index key store | [x] | [ ] | [ ] | macOS uses Keychain. Windows/Linux platform key stores are missing. |
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
- [x] Speaker analysis via the `speakrs` provider (CoreML + system OpenBLAS) using AVFoundation decode.
- [x] System-audio speech activity using AVFoundation-backed audio decode.
- [x] Semantic Search: on-device **candle** embeddings on the Apple GPU (Metal, F16) behind a pluggable **Semantic Search Backend** (the runtime tries Metal then falls back to candle-CPU; CUDA deferred), `vec0` vectors inside the SQLCipher-encrypted Capture Index, deferred-startup backfill sweep, and desktop-owned model download (now `rustls`, safetensors layout) from Hugging Face. This supersedes the `fastembed`/`ort` path: Metal frees the P-cores by construction, so the macOS-only per-thread QoS downclock and the ONNX arena mitigations are gone. candle-metal is a **second native ML runtime** in the bundle (links Metal/MPS) alongside `ort`, which now stays only for transcription — a release/notarization checklist item. No native capture or audio-decode dependency, so the runtime is platform-neutral in principle; only verified on macOS today. See [ADR 0037](docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md).

### Privacy, metadata, and UX

- [x] Screen and microphone permission checks/prompts.
- [x] Open macOS Privacy & Security panes for denied permissions.
- [x] Active app/window metadata from NSWorkspace/CoreGraphics.
- [x] Browser URL metadata for supported Chromium/WebKit browsers via AppleScript. Firefox-family (Gecko) browsers expose no scriptable URL and are not supported.
- [x] Live App Privacy Exclusion through ScreenCaptureKit app filters.
- [x] Exclude Current App tray action.
- [x] Recommended sensitive app exclusion catalog using macOS bundle IDs.
- [x] Native status-bar tray menu.
- [x] Quick Recall non-activating NSPanel launcher: reclassed `NSPanel` that becomes key without activating Mnema, floating window level, all-Spaces/full-screen-auxiliary collection behavior, `acceptsFirstMouse:` click pass-through, web-layer-owned Escape, and order-out/blur-grace dismissal.
- [x] Dock visibility and macOS terminate handling.
- [x] Global shortcuts through Tauri plugin for background start/stop recording, pause/resume recording, and show/hide Mnema.

### Storage, access, and release

- [x] Encrypted Capture Index key stored in macOS Keychain.
- [x] Broker Authorization Channel over per-user Unix socket.
- [x] Ask AI (Quick Recall + Insights Chat) brokered tool-calling session: runs in-process on the shared Reasoning Engine (`crates/ai-runtime` via `rig-core`, `run_agent_loop` in `agent_loop.rs`) — NOT the user's installed PI/Node runtime, which is removed (no shim, no `node`/`pi`-on-PATH resolution, no PI auth). It uses the same OS-Keychain provider key as User Context derivation and exposes only the `search`/`timeline`/`show_text`/`recall_context` brokered data tools (plus the presentation-only `reference_captures`), injected from the Tauri layer and enforced through the All-Retained Ask AI broker scope (`BrokeredCaptureAccess::execute_for_ask_ai`). Gating is two-layer: the engine-configured prerequisite plus the independent Ask AI Setting. See [ADR 0033](docs/adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md).
- [x] Ask AI background completion and live reattach (stateless-per-turn over the persistent conversation store): each turn loads the thread's persisted history, runs one agent loop, and persists the new turn — the resident-session/30-minute-unseen-expiry/resurrect-from-transcript machinery is deleted. A dismissed-but-streaming question finishes its task and writes the turn to the shared conversation store in the Encrypted Capture Index (origin `quick_recall`, governed by Retention Policy, cleared by Wipe User Context), and re-opening reads it back; a thread still generating supports live reattach (the in-flight task persists incremental partial progress, the reopened surface loads that partial then subscribes to ongoing `delta` events). A thread can be continued in the Insights Chat workspace ("Continue in Chat") under the same conversationId, and a Chat thread may pin a per-conversation engine identity. See [ADR 0033](docs/adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md) and [ADR 0031](docs/adr/0031-quick-recall-and-chat-share-one-persistent-conversation-store.md).
- [x] User Context derivation via the Reasoning Engine (`rig-core`): cloud (Anthropic/OpenAI bring-your-own-key, key in Keychain) or local (Ollama/Llamafile endpoint, no key); only redacted OCR/transcript text crosses the wire for a cloud engine, the dossier stays on-device, and the deterministic Confidence Policy / Sensitive Category Guardrail run with no model.
- [x] Deep-link app reopen fallback.
- [x] Open Captured URL in the default browser: the app-mediated action behind the broker `OpenCapturedUrl` request / CLI `open-url` subcommand and the desktop `open_captured_url` command resolves a result's raw captured `browser_url` locally and hands it to the OS opener. The desktop command uses the `@tauri-apps/plugin-opener` plugin; the broker/CLI path uses the native opener (`open`). The raw URL is local-only — it never enters a broker response, log, audit, or auth channel. Only the guarded host+path **Broker URL Context** crosses the broker boundary (see [ADR 0038](docs/adr/0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md)).
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

- [ ] Add Windows CI job for `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`, `cargo check --workspace --all-targets`, and `bun run check`.
- [ ] Audit all `cfg(target_os = "macos")` / non-mac stubs and ensure Windows builds cleanly with the desktop feature set.
- [ ] Decide whether capture output remains `.mov`/`.m4a` or becomes Windows-native formats with schema/runtime support for extensions.
- [ ] Remove user-facing “only macOS” errors once Windows adapters exist.
- [ ] Add Windows Tauri bundle config, installer target, signing plan, and updater artifacts.

### Runtime capture

- [ ] Implement a Windows screen capture backend behind `crates/capture-screen`.
  - Candidate APIs: Windows Graphics Capture or DXGI Desktop Duplication.
  - Must support frame timing, segment rotation, stop/error reporting, and output activity samples.
- [ ] Implement Windows microphone capture behind `crates/capture-microphone`.
  - Candidate APIs: WASAPI input, CPAL, or another native audio layer.
  - Must support device listing, default device tracking, selected-device reconnect policy, and VAD PCM feed.
- [ ] Implement Windows system-audio capture.
  - Candidate API: WASAPI loopback.
  - Decide whether system audio requires screen capture on Windows or can become an independent source.
- [ ] Implement Windows capture session IDs and source-session bookkeeping.
- [ ] Implement segment rotation without dropping OCR/frame-index invariants.
- [ ] Implement user pause/resume and inactivity pause/resume for each requested source family.
- [ ] Implement capture liveness/error propagation equivalent to ScreenCaptureKit delegate stop errors.
- [ ] Implement sleep/wake/session-lock recovery.

### Media writers and previews

- [ ] Implement cross-platform or Windows-specific audio/video writers.
  - Candidate APIs: Media Foundation, FFmpeg, GStreamer, or a Rust media pipeline.
- [ ] Implement finalized video validation equivalent to “openable `.mov` with moov”.
- [ ] Implement screen frame export to JPEG artifacts.
- [ ] Implement frame-index sidecar generation from finalized video timing.
- [ ] Implement exact frame preview extraction from video.
- [ ] Implement scrub preview batch generation.
- [ ] Implement audio trim/convert/finalization for microphone and system-audio outputs.
- [ ] Implement video-only screen output finalization when audio is muxed or recorded together.

### Processing

- [ ] Add Windows audio decode to mono PCM for Local Whisper.
- [ ] Add Windows audio decode to mono PCM for Parakeet.
- [ ] Add Windows audio decode to mono PCM for speaker analysis.
- [ ] Add Windows audio decode to mono PCM for system-audio speech activity.
- [ ] Verify packaged Tesseract/PaddleOCR runtimes and model installation on Windows.
- [ ] Choose Windows OCR default provider.
- [ ] Choose Windows transcription default provider.
- [ ] Hide/disable Apple Vision and Apple Speech providers on Windows.

### Permissions, privacy, and metadata

- [ ] Implement microphone permission/status UX and settings deep link, e.g. `ms-settings:privacy-microphone`.
- [ ] Define Windows screen-capture permission/support semantics.
- [ ] Define Windows system-audio permission/support semantics.
- [ ] Implement system idle detection, likely `GetLastInputInfo`.
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

- [ ] Implement Windows Capture Index Key Store using Credential Manager, DPAPI, or another platform-owned secret store.
- [ ] Implement Broker Authorization Channel for Windows.
  - Candidate transports: named pipes, localhost loopback, or another app-mediated IPC.
- [ ] Update `crates/cli` authorization request path for Windows.
- [ ] Verify deep links and app launch fallback on Windows.
- [ ] Verify Open Captured URL on Windows. The broker/CLI opener path is already coded as `cmd /C start "" <url>` and the desktop command uses the `@tauri-apps/plugin-opener` plugin; verify the captured URL opens in the default browser and that the raw URL never leaks into a broker response, log, or audit record.
- [~] Verify Ask AI (in-process Reasoning Engine) on Windows. There is no PI/Node runtime or shim to spawn — the agent loop runs in-process via `rig-core`. Verify brokered `search`/`timeline`/`show_text`/`recall_context` tool calls round-trip, the All-Retained broker scope/redaction holds, streaming/cancellation/background-completion work, and engine status reports correctly. Cloud-engine use is gated on the Windows Capture Index Key Store (above); a local Ollama/Llamafile engine needs no key.
- [~] Verify Quick Recall launcher behavior on Windows. The non-macOS fallback shows/focuses a plain always-on-top window (no non-activating panel), so verify summon-without-stealing-foreground, focus-into-search-field, and click-away/blur dismissal, or implement a Windows-native equivalent.
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
- [ ] Verify Open Captured URL across desktop environments. The broker/CLI opener path is already coded as `xdg-open <url>` and the desktop command uses the `@tauri-apps/plugin-opener` plugin; verify the captured URL opens in the default browser under Wayland/X11 portals and that the raw URL never leaks into a broker response, log, or audit record.
- [~] Verify Ask AI (in-process Reasoning Engine) on Linux. There is no PI/Node runtime or shim to spawn — the agent loop runs in-process via `rig-core`. Verify brokered `search`/`timeline`/`show_text`/`recall_context` tool calls round-trip under the All-Retained broker scope, streaming/cancellation/background-completion work, and engine status reports correctly across desktop environments. Cloud-engine use is gated on the Linux Capture Index Key Store; a local Ollama/Llamafile engine needs no key.
- [~] Verify Quick Recall launcher behavior across desktop environments/compositors. The non-macOS fallback shows/focuses a plain always-on-top window (no non-activating panel); verify summon focus, always-on-top/all-workspaces behavior, and click-away/blur dismissal under Wayland/X11, or implement a compositor-appropriate equivalent.
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
| `apps/desktop/src-tauri/src/app_infra/frame_preview.rs` | AVAssetImageGenerator exact/scrub previews | FFmpeg/GStreamer/Media Foundation extractor |
| `crates/audio-transcription/src/macos_audio_decode.rs` | AVFoundation audio decode for Local Whisper/Parakeet | Cross-platform audio decode module |
| `crates/speaker-analysis/src/macos_audio_decode.rs` | AVFoundation audio decode for diarization/recognition; the `speakrs` provider itself is CoreML + OpenBLAS (Apple Silicon only) | Cross-platform audio decode module **and** a non-CoreML diarization provider, since `speakrs` cannot run off Apple Silicon |
| `crates/ocr/src/lib.rs`, `crates/app-infra/src/processing/apple_vision.rs` | Apple Vision OCR | Disable on non-Apple; default to Tesseract/PaddleOCR |
| `crates/audio-transcription/src/providers/apple_speech.rs` | Apple Speech | Disable on non-Apple; default to local providers/cloud if introduced |
| `crates/app-infra/src/capture_index_key_store.rs` | macOS Keychain through `security` CLI | Windows Credential Manager/DPAPI; Linux Secret Service/KWallet |
| `apps/desktop/src-tauri/src/broker_authorization_channel.rs`, `crates/cli/src/main.rs` | Unix socket app-mediated authorization | Windows named pipe/TCP; Linux runtime-dir Unix socket validation |
| `apps/desktop/src-tauri/src/windows.rs` | macOS rounded content views, Dock visibility, terminate interception, Quick Recall non-activating NSPanel (reclass, style mask, floating level, first-mouse, key/first-responder, order-out) | Windows/Linux window behavior equivalents or no-ops; Quick Recall falls back to a plain shown/focused always-on-top window |
| `.github/workflows/*`, `docs/release-process.md`, `scripts/stage-macos-release-artifacts.sh` | macOS-only release pipeline | Windows/Linux release pipelines and docs |

## Cross-platform implementation principles

- Keep Tauri command handlers thin; native capture orchestration belongs behind the Recording Lifecycle seam in `apps/desktop/src-tauri/src/native_capture/lifecycle.rs` and platform capture crates.
- Keep shared serde/domain types in `crates/capture-types`.
- Keep capture primitives in `crates/capture-screen`, `crates/capture-microphone`, and `crates/capture-writers`; avoid putting OS APIs directly in Svelte or high-level Tauri commands.
- Preserve Capture Session / Capture Segment semantics across platforms, including date-organized output, source-session IDs, retention, OCR job enqueueing, and timeline events.
- Do not silently downgrade privacy. If an OS cannot support live app exclusion, expose an explicit unsupported/degraded capability and rely on App Privacy Exclusion disclosure, Pause Recording, and Delete Recent Capture recovery.
- Prefer capability-driven UI (`get_capture_support`, provider runtime availability, permission state) over direct platform checks in Svelte.
