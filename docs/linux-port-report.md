# Mnema on Linux: porting report

_Written 2026-11-01 against `main` @ `85dcb993`. Research only; no code was changed. Build checks ran on an Arch Linux machine with no display, so nothing here was run on a real Linux desktop._

**How to read this.** Section 1 is the answer. Sections 5–10 are the plan. Sections 2–4 are the evidence: every claim about the code cites `file:line`, and every claim about Linux, macOS, crates or benchmarks links a source. Uncertain or version-dependent items are marked **(unverified)** or **[U]**. Section 4 has one part per subsystem, each in the same shape: (a) what macOS does today, (b) how it performs, (c) the recommendation, (d) expected Linux performance and the gap, (e) fallbacks, (f) risks, (g) effort, (h) parity, (i) files, (j) degradation, (k) tests, (l) open questions.

**Corrections to the brief.** The brief's stack description is out of date in five places. The report follows the code:

| Brief said | The code actually has |
|---|---|
| CAM++ speaker diarization | **speakrs** 0.4 (pyannote segmentation + WeSpeaker embeddings + VBx clustering) on **CoreML**, statically linked OpenBLAS, run in a helper process (`crates/speaker-analysis/Cargo.toml:38`, `apps/desktop/src-tauri/src/speaker_analysis_runtime.rs:37`). |
| Apple Foundation Models with an embedded llama.cpp fallback behind `InsightProvider` | None of these exist. The AI layer is `crates/ai-runtime` on rig-core, calling cloud providers or a **user-installed** Ollama/Llamafile over HTTP (`crates/ai-runtime/src/lib.rs:95-139`). It has no OS-specific code. |
| ONNX Runtime (`ort`, `load-dynamic`) for embeddings | Embeddings run on **candle** (Metal on macOS, CPU elsewhere, `crates/semantic-search/src/backend/candle.rs:609-624`). `ort` is used only by Parakeet transcription, the Silero voice detector and speakrs. It is **statically linked** from pyke's prebuilt libraries; `load-dynamic` is off. |
| Browser URL via the AX tree plus a WebExtension | No extension exists. It was built and removed (ADR 0013, `docs/adr/0013-browser-integrated-sensitive-capture-v2.md:8-20`). URLs come from AppleScript (Chromium/WebKit) and the Accessibility API (Firefox/Zen only, ADR 0039). |
| Polar.sh licensing | Licensing runs on the `licensegate` crate with offline Ed25519 checks. Polar is only the merchant of record (ADRs 0045, 0054). |

## 1. Executive summary

**Mnema can be ported to Linux. Most of the hard work is replacing macOS media and capture frameworks, and three privacy promises can't fully carry over.** Every Apple framework the app leans on has a Linux replacement, and much of the app is already portable: storage, search, secret redaction, the AI engine, the OCR budget and the inactivity rules. The real cost is in three places:
- **Capture and media:** ScreenCaptureKit, AVFoundation writers and decoders, and Core Audio taps.
- **Desktop fragmentation:** there is no single Linux API for "which app is in front", global shortcuts or tray icons.
- **CPU-only machine learning:** Apple's Neural Engine and Metal paths fall back to the CPU.

**Rough size: about 8 engineer-months for full parity.** A Phase 1 build that records the screen and makes it searchable is about 3 of those months. These are estimates summed from the subsystem sections; nothing was prototyped.

### Biggest blockers, most serious first

1. **Per-app privacy exclusion can't match macOS.**
   - **Screen:** on macOS, excluded apps never reach the recording (ScreenCaptureKit filters them, ADR 0006). The Linux screen-sharing portal has no exclusion API, so on GNOME hiding an app at capture time is impossible. KDE 6.6+ and Hyprland may allow per-window hiding, which needs testing. Everywhere else, GNOME included once the Mnema extension is on, the fallback is "pause while an excluded app is in front", which still records an excluded window visible in the background. **Decided:** ship that, with plain per-desktop wording (§10 decision 3, ADR 0060).
   - **Audio:** system-audio exclusion has to be rebuilt by hand from PipeWire links, and audio-effect apps like EasyEffects can leak excluded audio.
   - Settled in §10 decisions 3 and 4 (ADR 0060) (§4.1, §4.5, §7).
2. **Consent and indicator UX for an always-on recorder.**
   - Wayland capture goes through a desktop dialog. The "remember" token is single-use and must be saved again after every start.
   - It can be lost when monitors change, and an unattended login can end up stuck on an unanswered dialog.
   - GNOME and KDE show a screen-sharing indicator, with a one-click Stop, the entire time Mnema records (§4.1, §4.10).
3. **Every AVFoundation media path has to be replaced:** the video writer, frame extraction, scrub previews, audio writers, the trim step and four audio-decode call sites. This brings two new questions:
   - **Video:** the H.264 patent licence. Shipping our own encoder means signing the free Via LA licence, or using AV1 instead.
   - **Audio:** the format. Opus on Linux versus AAC/m4a everywhere (§4.1, §4.5).
4. **No portable "active app" API on Wayland.** KDE, wlroots and X11 are fine. GNOME needs a Shell extension that Mnema ships itself, plus a log-out to enable it, plus an update every GNOME release (§4.2).
5. **Weaker secret isolation.** On Linux, any program running as the user can read the database key from the keyring. Today on macOS only the app and its CLI can (§4.8).
6. **CPU-only machine learning.**
   - OCR is roughly several times slower per frame. The existing OCR budget turns that into indexing delay rather than extra watts.
   - Embeddings: candle on the CPU may need 35–60 minutes to index 1,000 items, against about 7 minutes on Metal (estimate).
   - Speaker detection goes from about 529× real time on CoreML to an unmeasured CPU speed (§4.4, §4.6).

### Already broken on non-macOS, found during this audit

These are worth fixing whether or not the port goes ahead:
- **Licensing is open off macOS.** Non-macOS builds have no machine fingerprint, so no trial is issued and status is forced to "Activated". It's deliberate, but it means any Linux build captures for free (`apps/desktop/src-tauri/src/licensing/adapter.rs:175`, `licensing/trial.rs:168`; checked by hand). The fix is to hash `/etc/machine-id` through licensegate's existing salted hash (§4.12).
- **The release workflow overwrites every platform's updater signature** with the macOS one (`.github/workflows/macos-release.yml:304`). A Linux entry in the same `latest.json` would fail to update (§4.13).
- **Local Ollama gets no context-window size** (`num_ctx` is never sent). Ollama's default is 4k tokens on machines with under 24 GiB of video memory, so long prompts may already be cut silently on 16 GB Macs **(unverified in practice)** (§4.7).
- **Whisper is tuned to the build machine's CPU.** whisper.cpp defaults to `GGML_NATIVE=ON`, confirmed in `whisper-rs-sys-0.15.0/whisper.cpp/ggml/CMakeLists.txt:102-106`. CI releases build on `macos-15` runners and are fine, but a release built locally on a newer Mac could crash on older ones. Linux CI must set `GGML_NATIVE=OFF`.

### Recommended Linux v1 target

- **Desktops:** **Wayland only, on GNOME 46+ and KDE Plasma 6.4+.** GNOME 50 and Plasma 6.8 have dropped their X11 sessions ([linuxiac](https://linuxiac.com/gnome-50-ends-the-x11-era-after-decades/), [Phoronix](https://www.phoronix.com/news/KDE-Plasma-68-Wayland-Exclusive)), so X11 is a shrinking target.
- **KDE Plasma is the reference desktop:** it has the best parity (KWin script for app tracking, possible per-window hiding, portal shortcuts, input idle).
- **GNOME is supported with the bundled Shell extension.**
- **COSMIC is supported** (the default desktop of Pop!_OS 24.04) with its own front-app backend (§10 decision 12).
- **X11 and wlroots (Sway, Hyprland, niri):** best-effort and community-tested. They are cheap to keep working but not release-blocking.
- **Distros:** Ubuntu 24.04 LTS and later, Linux Mint 22+, Pop!_OS 24.04+, Fedora 42+, Debian 13+, Arch (via AUR).
- **Architecture:** x86_64 only for v1; aarch64 later.
- **Packages:** `.deb` + `.rpm` updated by the in-app updater, plus an AUR package.
- **Not in v1:** Flatpak and Snap.

**Build floor: Ubuntu 24.04 (glibc 2.39, PipeWire 1.0)**, decided in §10 decision 1. 22.04 ships PipeWire 0.3.48 and GNOME 42, both too old to rely on restore tokens and portal maturity for an always-on recorder. §4.12 and §4.13 now build on 24.04.

### Phase 0 status, measured on this machine

The compile check ran on Linux (§2):
- **12 of 16 crates already build as committed**, including `semantic-search` on candle's CPU path.
- **Two one-line `cfg` mistakes break the rest:**
  - `crates/capture-screen/src/lib.rs:20` gates a module that `:61` uses on every platform.
  - `crates/ocr/src/lib.rs:730` gates a helper that the Paddle and Tesseract providers use.
- **With those two lines ungated:**
  - `capture-screen`, `ocr`, `cli` and **`app-infra` build**.
  - SQLCipher, sqlite-vec and sqlx build together on one `libsqlite3-sys`, linking the system OpenSSL. The storage stack is not a Linux blocker.
- **Still failing:**
  - **speakrs** stops on `compile_error!("the coreml feature is only supported on macOS")`.
  - **Whisper, Parakeet and the OCR engines** need `cmake` and `clang` installed.
  - **The desktop crate** needs `webkit2gtk-4.1`, then stops at **at least 14 errors**, all macOS-only gating inside `native_capture/*`, `app_infra.rs` and `windows.rs:501`.

Phase 0 (§8) is therefore days of work, not weeks.

### Subsystem scorecard

| # | Subsystem | Recommendation in one line | Effort |
|---|---|---|---|
| 4.1 | Screen capture + video | Portal + PipeWire (`ashpd`, `pipewire`), shared-memory frames, OpenH264 + `mp4-atom` writer and preview decoder; X11 via `xcap` | **L** (7–10 wk) |
| 4.2 | Active window | One interface, four backends: KWin script, Mnema GNOME extension, wlroots window-list protocol + IPC, `x11rb` | **L** (3–4 wk) |
| 4.3 | Browser URL | AT-SPI2 via `atspi` for every browser; no extension | **M** |
| 4.4 | OCR | Make the existing PaddleOCR (MNN) provider the Linux default; Tesseract as fallback | **M** |
| 4.5 | Audio + diarization | cpal for the mic; PipeWire link graph for system audio; Opus/Ogg on Linux; speakrs CPU mode | **L** |
| 4.6 | ML runtimes | CPU-only v1 with three build fixes; gate semantic search on a benchmark | **M** |
| 4.7 | Local LLM | Keep "bring your own Ollama"; send `num_ctx` | **S** |
| 4.8 | Secrets | Secret Service through a pure-Rust zbus client; explicit file opt-in when no keyring | **M** |
| 4.9 | Lifecycle | One zbus module: logind sleep/lock, per-desktop idle readers, GDK hotplug, notify-rust | **M** |
| 4.10 | Permissions UX | One real consent (the portal dialog); mic and system audio need none; no relaunch | **M** |
| 4.11 | Tauri shell | Stock Tauri v2 on WebKitGTK 4.1; GlobalShortcuts portal; menu-only tray; Quick Recall as a normal window | **M** (~3 wk) |
| 4.12 | Packaging | `.deb` + `.rpm` with the in-app updater; AUR; AppImage fallback; Linux machine id for licensing | **M** |
| 4.13 | CI | Container-based `linux-build.yml`; allowlist of Linux-clean crates now, widen as the port lands | **M** |

## 2. Inventory of platform dependencies

_Each row is rated **blocker**, **needs reimplementation**, **config-only** or **trivial**. Row counts by area come first, then the full table, per-crate counts and the Linux compile check._

Repo: the repo at `85dcb993` (main, clean). All paths relative to repo root. Read-only; the compile probes ran against a scratch copy (`.git`, `target` and `node_modules` excluded) so no build script or lockfile touched the repo.

Severity key:
- **blocker** — no Linux code path exists and the feature is core (capture, storage key, decode); without it the app records/processes nothing.
- **needs reimplementation** — a non-macOS stub already exists or the feature is secondary; a Linux backend must be written behind the existing seam.
- **config-only** — a path, URL, flag, manifest or build setting; no new subsystem.
- **trivial** — copy, label, or a one-line `cfg` fix.

---

### Summary of severities by area (from the table)

| area | blocker | needs reimplementation | config-only | trivial | already-scaffolded rows |
|---|---:|---:|---:|---:|---:|
| Screen capture | 7 | 5 | 0 | 2 | 3 |
| Audio capture (mic, system audio, writers) | 7 | 6 | 1 | 1 | 2 |
| OCR | 0 | 3 | 1 | 1 | 0 |
| ML (transcription / diarization / embeddings) | 3 | 5 | 1 | 1 | 2 |
| Metadata / AX / browser URL | 0 | 8 | 2 | 0 | 0 |
| Secrets / identity / licensing | 3 | 2 | 1 | 0 | 5 |
| Lifecycle / shell | 0 | 1 | 3 | 10 | 4 |
| Frontend | 0 | 1 | 0 | 5 | 1 |
| Build / CI / packaging | 0 | 0 | 9 | 0 | 0 |

---

### Inventory table

#### Screen capture (`crates/capture-screen`, desktop adapters)

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `crates/capture-screen/Cargo.toml:13-17` | target deps | cidre (`sc`,`av`,`vt`,`cg`,`iio`,`cm`,`dispatch`), objc2, objc2-av-foundation, objc2-foundation | — | blocker |
| `crates/capture-screen/src/lib.rs` (254 macOS cfg lines; API hits at 184, 503-516, 585-1152, 1544-2066, 2255-2790, 3026-3838, 4393-4470) | `start_capture_session`, `rotate_…`, `stop_…`, stream output/stream delegates (`mod stream_output_delegate` :1846, `mod stream_delegate` :1979) | ScreenCaptureKit stream → H.264 via AVAssetWriter; per-frame JPEG export for OCR (`save_screen_sample_as_jpeg` :1656, ImageIO); screen-activity fingerprint from `CMSampleBuffer`/`CVPixelBuffer` (:716-1107); privacy content filter (app-window exclusion) | ScreenCaptureKit, AVFoundation (AVCaptureScreenInput fallback), AVAssetWriter, VideoToolbox, CoreGraphics (`CGDisplayCreateImage` :503, `CGGetActiveDisplayList` :498), ImageIO, CoreVideo | blocker |
| `crates/capture-screen/src/lib.rs:3815-3833` | `screen_permission_state` | TCC screen-recording preflight/request | `CGPreflightScreenCaptureAccess`, `CGRequestScreenCaptureAccess` | blocker (→ xdg-desktop-portal ScreenCast) |
| `crates/capture-screen/src/lib.rs:1141,1167,1179,1528,3708,3855-4042` | **non-macOS stubs already exist** | `poll_screen_activity`→false, `screen_display_available`, `rebuild_screen_segment_frame_index_from_video`, `supports_screen_capture_kit_backend`/`supports_frame_export`→false, stub `ActiveCaptureSession`/`StartedCaptureSession`, `new_session_id`/`start_capture_session` → `unsupported_platform` | — | scaffolded |
| `crates/capture-screen/src/lib.rs:20-21` vs `:61` | **cfg drift (compile error on Linux)** | `mod equivalence` gated to macOS but `pub use equivalence::{…}` is unconditional; `equivalence.rs` is pure Rust (0 cidre/objc refs, 410 lines) | — | trivial (un-gate) |
| `apps/desktop/src-tauri/src/native_capture.rs:1302-1531` | `start_system_wake_notifier`, wake recovery | sleep/wake → suspend/recover screen | `NSWorkspaceWillSleep/DidWake` via cidre `ns::NotificationGuard` | needs reimplementation (stub at :1531; logind `PrepareForSleep` on D-Bus) |
| `apps/desktop/src-tauri/src/native_capture.rs:1541-1624` | `start_display_reconfiguration_notifier` | display attach/detach/wake re-arm | `CGDisplayRegisterReconfigurationCallback` | needs reimplementation |
| `apps/desktop/src-tauri/src/native_capture.rs:84-125` | `SystemWakeNotifierState`, `DisplayReconfigurationNotifierState`, `MetadataNotifierState` | **non-macOS stub types exist** | — | scaffolded |
| `apps/desktop/src-tauri/src/native_capture/runtime.rs:22-889` (38 cfg lines; fields :128-178) | `NativeCaptureRuntime` | every active-session field (`active_screen_session`, `active_microphone_session`, `active_system_audio_session`, `capture_suspension`, `free_space_probe`, recording files) is macOS-only, so the generic lifecycle holds nothing on Linux | — | blocker (un-gate fields when Linux backends land) |
| `apps/desktop/src-tauri/src/native_capture/segments.rs:52-649` (115 cfg lines) | segment create/rotate/finalize, low-disk + trial/revoked stop notifications | whole segment pipeline gated macOS | — | blocker (logic is mostly generic; gated because session types are) |
| `apps/desktop/src-tauri/src/native_capture/lifecycle.rs:40-975` (18 cfg lines) | `tick_inactivity`, `tick_rotation`, sleep, system-audio retry, out-of-band mic | owning lifecycle seam; ticks only exist on macOS | — | blocker (same reason) |
| `apps/desktop/src-tauri/src/native_capture/privacy.rs:18-637` (37 cfg lines; stub :104) | privacy filter collection/apply | builds SCK content filter + tap exclude list from privacy list | ScreenCaptureKit filter | needs reimplementation (no Linux equivalent to per-app window exclusion under Wayland; X11 only by masking) |
| `apps/desktop/src-tauri/src/native_capture_output.rs:63-1275` (44 cfg lines) | `finalize_capture_outputs`, output validation | finalize / validate m4a+mp4 outputs | via capture-writers (AVFoundation) | blocker |
| `apps/desktop/src-tauri/src/app_infra/frame_preview.rs:1387-1899` | `extract_preview_image_from_video`, scrub previews | exact frame + scrub thumbnails from mp4 | AVAssetImageGenerator via cidre, CGImage | needs reimplementation (stubs at :1889, :1899) |
| `apps/desktop/src-tauri/src/native_capture/activity.rs:61-66,129,182` | `current_system_idle_ms`, screen-activity poll | inactivity detection | via `native_capture_system_idle` | needs reimplementation (stub :66) |
| `apps/desktop/src-tauri/src/native_capture/capture_health.rs:195,401-414` | dev health DTO | tap reasons + disk thresholds | — | trivial (stub :414) |
| `apps/desktop/src-tauri/src/native_capture/tests.rs` (189 cfg lines, `mod low_disk` :9764) | tests | macOS-only test bodies | — | n/a |

#### Audio capture (microphone, system audio, writers)

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `crates/capture-microphone/Cargo.toml:11-12` | cidre (`av`,`cat`,`cm`,`dispatch`) | — | — | blocker |
| `crates/capture-microphone/src/lib.rs` (155 cfg lines; `mod microphone_delegate` :1651; PCM parsing :330-1299) | `AvFoundationMicrophoneCaptureSession` | AVCaptureSession mic → AAC m4a, device list, device-change, VAD-fed activity | AVFoundation `AVCaptureDevice`/`AVCaptureAudioDataOutput`, CoreAudioTypes | blocker |
| `crates/capture-microphone/src/lib.rs:2791-2822` | mic permission state/request | TCC microphone | `AVCaptureDevice authorizationStatus/requestAccess` | needs reimplementation (Linux: no TCC; PipeWire portal optional) |
| `crates/capture-microphone/src/lib.rs:203-425,2380,2600-2604,2844-2849` | **non-macOS stubs exist** (18) | — | — | scaffolded |
| `apps/desktop/src-tauri/src/native_capture/microphone.rs:102-252` | `start_microphone_device_change_notifier`, reconnect | device hot-plug | AVCaptureDevice notifications | needs reimplementation (stub :133) |
| `apps/desktop/src-tauri/src/voice_enrollment.rs:143-166,238` | bounded enrollment clip | fixed-length mic take | via capture-microphone | needs reimplementation |
| `crates/capture-system-audio/Cargo.toml:11-15` | cidre `core_audio` | — | — | blocker |
| `crates/capture-system-audio/src/lib.rs:10-20` | `mod probe/process/rebuild/tap/writer` all macOS-only | Core Audio process tap + aggregate device + per-segment writer | `CATapDescription`, `AudioHardwareCreateProcessTap`, aggregate devices | blocker (→ PipeWire monitor/loopback) |
| `crates/capture-system-audio/src/lib.rs:32-62` | re-exports + `supports_system_audio_capture` | stub returns `false` at :59 | `cidre::api::version!(macos = 15.0)` | scaffolded |
| `crates/capture-system-audio/src/exclude.rs:9-772` (27 cfg lines) | `read_audio_processes`, `SystemAudioExcludeWatcher` | enumerates audio processes by bundle id / responsible bundle for privacy exclusion | Core Audio process objects, `NSRunningApplication` | needs reimplementation (Linux: PipeWire node `application.process.binary`/`.id`) |
| `crates/capture-system-audio/src/permission.rs:3-70` | inferred tri-state permission | evidence-row inference (no TCC query) | — | config-only (Linux has no TCC; can report "granted") |
| `apps/desktop/src-tauri/src/native_capture.rs:23-24` | `mod system_audio` (adapter) | macOS-only adapter | — | blocker |
| `apps/desktop/src-tauri/src/lib.rs:500-532` | `reap_stale_system_audio_aggregate_devices`, `hydrate_system_audio_permission_evidence` | startup heal | Core Audio | trivial (stubs at :512, :532) |
| `crates/capture-writers/Cargo.toml:9-10` | cidre (`av`,`cat`,`cm`) | — | — | blocker |
| `crates/capture-writers/src/lib.rs:4-1535` (71 cfg lines, **0 non-macOS stubs** — the whole public API vanishes on Linux) | `AudioAssetWriterState`, `VideoAssetWriterState`, `create_*_asset_writer*`, `finish_*`, `trim_audio_file_to_m4a` (:26), `decode_audio_file_to_mono_pcm` (:165) | AAC/H.264 writers, trim, decode | AVAssetWriter, AVAudioFile, AudioToolbox | blocker |
| `crates/capture-writers/src/lib.rs:1535-1541` | `convert_recording_audio_to_m4a` | shells to `/usr/bin/afconvert -f m4af -d aac` | `afconvert` | needs reimplementation (ffmpeg/GStreamer) |
| `crates/app-infra/src/processing/system_audio_speech_activity.rs:143-157` | `detect_system_audio_speech` | decode m4a → Silero VAD | via capture-writers decode | needs reimplementation (stub :157 errors) |

#### OCR

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `crates/ocr/Cargo.toml:24-25` | cidre `vn` | — | — | needs reimplementation |
| `crates/ocr/src/lib.rs:14-20,109,409-740` | Apple Vision provider | `VNRecognizeTextRequest` OCR | Vision | needs reimplementation (Tesseract/Paddle providers exist; stub :749 errors) |
| `crates/ocr/src/lib.rs:730` vs `paddle.rs:6`, `tesseract.rs:10` | **cfg drift (compile error on Linux)** | `normalize_candidate_text` gated `any(macos, test)` but used by always-compiled `paddle`/`tesseract` modules | — | trivial (un-gate) |
| `crates/ocr/Cargo.toml:8-19` | `paddle-rs` (`ocr-rs` 2.2.2: bindgen + cmake, MNN), `tesseract-embedded` (`tesseract-rs` 0.2: builds Tesseract/Leptonica via cmake) | cross-platform OCR backends, desktop enables both (`apps/desktop/src-tauri/Cargo.toml:69`) | — | config-only (needs cmake + clang/libclang on Linux) |
| `crates/app-infra/src/processing/apple_vision.rs:4-393` (42 cfg lines) | Apple Vision structured observations | app-infra side of Vision OCR | Vision via cidre | needs reimplementation (test-only shapes compile on Linux) |
| `crates/ocr/examples/apple_vision_benchmark.rs`, `examples/common/mod.rs` | benchmarks | — | Vision | n/a |

#### ML / transcription / diarization / embeddings

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `crates/audio-transcription/Cargo.toml:29-37` | block, block2, objc, objc2, objc2-foundation, objc2-speech, cidre, `whisper-rs` with `metal` | — | Speech, Metal | — |
| `crates/audio-transcription/Cargo.toml:39-40` | **non-macOS whisper-rs without metal already declared** | — | — | scaffolded |
| `crates/audio-transcription/src/providers/apple_speech.rs:12-581` (28 cfg lines; auth :332-412) | Apple Speech provider | `SFSpeechURLRecognitionRequest` on-device transcription + TCC auth | Speech framework | needs reimplementation (drop on Linux; Whisper/Parakeet/Deepgram remain) |
| `crates/audio-transcription/src/macos_audio_decode.rs:4-782` | AVFoundation decode | m4a → PCM for Whisper/Parakeet | AVAssetReader/AVAudioFile | blocker for local transcription (Linux needs symphonia/ffmpeg decode) |
| `crates/audio-transcription/src/providers/local_whisper.rs:9-378` | Whisper | metal provenance flag (:232) + AVFoundation decode | Metal | needs reimplementation (decode only) |
| `crates/audio-transcription/src/providers/parakeet.rs:1267-1290` | Parakeet ONNX (`ort =2.0.0-rc.12`) | decode stub on non-macOS errors (:1278) | AVFoundation | needs reimplementation (decode only) |
| `crates/speaker-analysis/Cargo.toml:38` | `speakrs` features `coreml`,`openblas-static` (unconditional) | — | CoreML | blocker |
| `crates/speaker-analysis/src/providers/speakrs.rs:189-192` | `ExecutionMode::CoreMl` hard-coded | diarization pipeline | CoreML | needs reimplementation (switch to speakrs CPU/ort mode on Linux) |
| `crates/speaker-analysis/src/macos_audio_decode.rs:1-425`, `providers/shared.rs:287-301` | AVFoundation decode | m4a → 16 kHz mono; **non-macOS stub exists** (:301) | AVAssetReader | blocker for diarization (same decode gap) |
| `crates/speaker-analysis/src/enrollment.rs:129,176` | test helpers | `~/Library/Application Support/day.mnema` + `/usr/bin/say` | `say` | trivial (test-only) |
| `apps/desktop/src-tauri/build.rs:42-44` | static gfortran link | gated to `target_os = macos`; Linux openblas-static would still need gfortran at build time | — | config-only |
| `crates/semantic-search/Cargo.toml:15-18`; `apps/desktop/src-tauri/Cargo.toml:120-124` | candle `metal` feature | enabled only via macOS target dep; Linux gets candle-CPU | Metal/MPS | scaffolded (config already correct) |
| `apps/desktop/src-tauri/src/semantic_search_worker.rs` | App Nap note | comment only, no NSProcessInfo call found | — | n/a |

#### Metadata / Accessibility / browser URL

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `apps/desktop/src-tauri/src/native_capture_metadata.rs:219-613` | `start_metadata_notifier`, `collect_native_active_window_snapshot` (:427), `frontmost_app_is_lock_screen` (:444), `copy_on_screen_window_infos` (:472), `active_browser_url` (:587) | frontmost app/window title/bounds, lock-screen detection, browser URL | NSWorkspace activation notifications, `CGWindowListCopyWindowInfo`, CoreFoundation dicts | needs reimplementation (stub :271; X11 `_NET_ACTIVE_WINDOW`; Wayland has no portable API) |
| `apps/desktop/src-tauri/src/native_capture_metadata.rs:601-614` | `active_browser_url_applescript`, `run_osascript` | Chromium/WebKit tab URL | `/usr/bin/osascript` (AppleScript) | needs reimplementation (no Linux equivalent; extension or AT-SPI) |
| `apps/desktop/src-tauri/src/native_capture_browser_url_ax.rs` (mod gated at `native_capture.rs:3-5`) | Gecko URL via AX | `AXURL` attribute walk, `AXUIElementSetMessagingTimeout` | Accessibility (AXUIElement), `AXIsProcessTrusted` | needs reimplementation (AT-SPI2) |
| `apps/desktop/src-tauri/src/native_capture.rs:2644-2660` | `open_browser_url_accessibility_settings` | opens Accessibility pane | `x-apple.systempreferences:` URL | config-only |
| `crates/capture-metadata/src/lib.rs:18-994` (no cfg; 45 AppleScript refs) | `BrowserUrlStrategy`, `browser_url_applescript`, browser catalog by bundle id (:311-317), `LOCK_SCREEN_BUNDLE_IDS = ["com.apple.loginwindow"]` (:104) | platform-neutral logic but data keyed by macOS bundle ids and AppleScript dialects | — | needs reimplementation (catalog keyed by bundle id; Linux keys are WM_CLASS / .desktop id) |
| `apps/desktop/src-tauri/src/sensitive_capture_recommendations.rs:45+` (25 bundle-id rows) | recommended exclusions | catalog uses `com.apple.Passwords`, `com.1password.1password`… | — | config-only (Linux id catalog) |
| `apps/desktop/src-tauri/src/native_capture.rs:161-704` | privacy app picker + icons | enumerates `/Applications`, `/System/Applications` (:484-485) and running apps; renders app icons | NSWorkspace `iconForFile`, bundle parsing | needs reimplementation (.desktop files + icon theme) |
| `apps/desktop/src-tauri/src/native_capture_system_idle.rs:5-46` | `current_system_idle_ms` | HID idle seconds | `CGEventSourceSecondsSinceLastEventType` | needs reimplementation (stub :46; logind IdleHint / XScreenSaver / ext-idle-notify) |
| `apps/desktop/src-tauri/src/native_capture/not_recording_reminder.rs:15,98,187,193-200` | reminder + `user_away` | posts OS notification; idle via CG | UNUserNotificationCenter, CoreGraphics | needs reimplementation (stub :200) |
| `apps/desktop/src-tauri/src/status_bar.rs:641-685` | tray "Exclude Current App" | uses frontmost snapshot | via metadata | needs reimplementation (non-mac branch :685) |

#### Secrets / identity / licensing storage

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `crates/app-infra/Cargo.toml:48-54` | security-framework, core-foundation, cidre, libc | — | — | — |
| `crates/app-infra/src/secret_vault.rs:19-83` | `KeychainMasterKeySource` (`day.mnema.vault`) | vault master key in login keychain | Security.framework `get/set_generic_password` | blocker (**non-mac stub :70-83 returns error** → vault never unlocks; Linux: Secret Service / `keyring` crate) |
| `crates/app-infra/src/secret_vault.rs:86-138` | `MNEMA_DEV_MASTER_KEY_FILE` file source | debug-only file key (has `#[cfg(unix)]` 0600 perms) | — | scaffolded (debug only) |
| `crates/app-infra/src/capture_index_key_store.rs:9-16,251-463` | SQLCipher DB key | shared-access-group DP keychain (`shared_group` mod :17) + `/usr/bin/security` fallback (:369, :411, :432) | Security.framework, `/usr/bin/security` CLI | blocker (non-mac `load/store/delete_platform_key` stubs :449-463 all error; `MNEMA_CAPTURE_INDEX_KEY_DIR` file knob :242 is cross-platform) |
| `crates/app-infra/src/capture_index_key_store/shared_group.rs` | team access group `RJYMY4RR97.day.mnema.capture-index` | entitlement-validated keychain group | Keychain access groups | needs reimplementation (no Linux analogue) |
| `crates/app-infra/src/license_token_store.rs:190-242` | `day.mnema.licensing` token store | license token in keychain | Security.framework | blocker for licensing (non-mac stubs :230-242 error) |
| `crates/app-infra/src/machine_id.rs:14-127` | `hardware_uuid`, `hardware_model` | licensing machine binding | `gethostuuid`, sysctl `hw.model` | needs reimplementation (stubs :48, :104 error; Linux `/etc/machine-id`, DMI) |
| `crates/app-infra/src/brokered_access.rs:1296-1310` | `default_app_config_dir_for_identifier` | `~/Library/Application Support/<id>`; **non-mac → `dirs::config_dir()`** | — | scaffolded |
| `crates/app-infra/src/brokered_access.rs:3538-3566` | `open_external_url` | `open` / `cmd start` / **`xdg-open`** | — | scaffolded |
| `crates/app-infra/src/brokered_access.rs:1520-1530` | `write_owner_only` | 0600 via `#[cfg(unix)]` | — | scaffolded |
| `apps/desktop/src-tauri/src/broker_authorization_channel.rs:1-1031` | per-user Unix socket broker | `#[cfg(unix)]` throughout; config dir mac `~/Library/…` vs `XDG_CONFIG_HOME` (:1018-1036) | — | scaffolded |
| `crates/cli/src/main.rs:1-35,1180-1220` | mnema-cli | `#[cfg(unix)]` socket client; `open`/`xdg-open` launch (:1189-1199); mac-only branch :1212 | — | scaffolded |
| `apps/desktop/src-tauri/Info.mnema-cli.plist`, `Entitlements.mnema-cli.plist` | CLI as bundle main exe (`Contents/Helpers/mnema-cli.app`) for keychain group | — | code signing | config-only (n/a on Linux) |

#### Lifecycle / app shell (Rust side)

| file:line | symbol / feature | what it does | macOS API | severity |
|---|---|---|---|---|
| `apps/desktop/src-tauri/Cargo.toml:36` + `tauri.conf.json:13` | `macos-private-api` feature, `macOSPrivateApi: true` | transparent windows on macOS | private WKWebView API | config-only (harmless elsewhere) |
| `apps/desktop/src-tauri/Cargo.toml:110-124` | block, cocoa, core-graphics, core-foundation(-sys), cidre, objc | all target-gated | — | — |
| `apps/desktop/src-tauri/src/windows.rs:495-502` | `title_bar_style(TitleBarStyle::Overlay)`, `hidden_title(true)` — **not cfg-gated** | overlay traffic-light titlebar | NSWindow | trivial (Tauri builder methods are macOS-only → expected compile error; gate it) |
| `apps/desktop/src-tauri/src/windows.rs:205-324,507-508,656` | `macos_corner_radius`, `apply_macos_rounded_content_view` | rounded content views | NSView layer | trivial |
| `apps/desktop/src-tauri/src/windows.rs:696-978,1444` | Quick Recall `NSPanel` subclass, key/first-responder, order-out | Spotlight-style non-activating panel | NSPanel via objc runtime | needs reimplementation (plain always-on-top fallback at :969/:978/:1446 exists) |
| `apps/desktop/src-tauri/src/windows.rs:910-928` | `set_all_webview_views_hidden` | render-idle gate | WKWebView `setHidden:` | trivial (stub :928) |
| `apps/desktop/src-tauri/src/windows.rs:1068-1136` | dock icon show/refresh, `install_macos_terminate_handler` | Dock visibility; swizzles `applicationShouldTerminate:` | NSApplication activation policy, objc runtime | trivial (stubs :1075, :1088, :1135) |
| `apps/desktop/src-tauri/src/windows.rs:1230-1245` | `macos_immediate_process_exit` | fast exit | — | trivial (non-mac branch :1238) |
| `apps/desktop/src-tauri/src/windows/quick_recall_frame.rs:47-93` | anchor Quick Recall | NSScreen `visibleFrame` | AppKit | trivial (non-mac uses `monitor.work_area()` :93) |
| `apps/desktop/src-tauri/src/windows/permission_helper.rs:75-122` | permission helper float | `orderFrontRegardless` | AppKit | trivial (stub :122); content is macOS System Settings–specific |
| `apps/desktop/src-tauri/src/webview_cache.rs:42-54` | memory-cache purge on blur | `WKWebsiteDataStore removeDataOfTypes` | WebKit | trivial (no-op on Linux :47) |
| `apps/desktop/src-tauri/src/clock_format.rs:10-82` | `system_uses_24_hour_time` | locale hour cycle | `NSDateFormatter dateFormatFromTemplate:"j"` | trivial (returns `None` :29) |
| `apps/desktop/src-tauri/src/lib.rs:997-998` | `RunEvent::Reopen` | Dock-click reopen | — | trivial (already gated) |
| `apps/desktop/src-tauri/src/native_capture.rs:2548-2563`; `audio_transcription_models.rs:343-353` | `open_capture_privacy_settings`, `open_apple_speech_recognition_privacy_settings` | `x-apple.systempreferences:` URLs, **not cfg-gated** (compile fine, fail at runtime) | System Settings | config-only |
| `apps/desktop/src-tauri/src/app_infra.rs:4280-4315` | sidecar triple candidates | has macOS, Windows **and Linux** (:4306) branches | — | scaffolded |
| `apps/desktop/src-tauri/src/app_infra.rs:4384`, `ask_ai/mcp/transport.rs:141` | login-shell PATH | `$SHELL -l -c`, `/bin/zsh` fallback | — | trivial (use `/bin/sh` on Linux) |
| `apps/desktop/src-tauri/src/ask_ai/mcp/transport.rs:117-123,405` | MCP process-group kill | `process-wrap` `ProcessGroup::leader()`, `kill` | Unix | scaffolded |
| `apps/desktop/src-tauri/src/app_infra.rs:4248-4559` | Windows-specific helpers | `#[cfg(windows)]` + `#[cfg(unix)]` (:4497) | — | scaffolded |
| `apps/desktop/src-tauri/capabilities/default.json:5` | window labels | platform-neutral | — | none |
| Tauri plugins (`Cargo.toml:37-42,88-90`): global-shortcut, updater, deep-link, single-instance, tray | cross-platform crates; Linux needs `libayatana-appindicator3` (tray), `.desktop` for `mnema://` deep link, updater needs AppImage | — | config-only |

#### Frontend (`apps/desktop/src`)

| file:line | symbol / feature | what it does | severity |
|---|---|---|---|
| `lib/keyboard.ts:96-117` | `keyboardPlatformFromUserAgent` → `macos`/`windows`/`other` | platform detection already exists; `other` (Linux) accepts Meta XOR Ctrl as primary (:116) | scaffolded (Linux should map to Ctrl like Windows) |
| `lib/keyboard-bindings.svelte.ts:19-27`, `lib/keyboard-binding-utils.ts:7,168-170` | `CommandOrControl+…` accelerators | cross-platform Tauri accelerators | none |
| 21 files with literal `⌘` labels (e.g. `lib/shell/Titlebar.svelte`, `lib/quick-recall/*.svelte`, `lib/chat/Chat.svelte`, `routes/+layout.svelte`, `lib/insights/*.svelte`) | hard-coded ⌘ glyphs | shortcut hints | trivial (render from platform) |
| 18 files using `metaKey` (e.g. `lib/quick-recall/search-keys.ts`, `routes/+page.svelte`, `lib/components/DateInput.svelte`) | raw `metaKey` checks outside `matchShortcut` | some bypass the platform helper | trivial |
| `lib/styles/shell.css:10-17`, `routes/+layout.svelte:56,384,928,958`, `lib/settings/SettingsRail.svelte:4` | 78px traffic-light inset, `app-shell--macos` | already conditional on `windowPlatform === "macos"` in layout | trivial (Linux needs its own window controls since `decorations:false` windows lose them) |
| `lib/shell/Titlebar.svelte:121-167` (+4 other files) | `data-tauri-drag-region` | custom drag | none (works on Linux) |
| 20 files, 61 `-webkit-*` hits (13× `-webkit-backdrop-filter`, scrollbar, line-clamp, mask) + 30 `backdrop-filter` | CSS | WebKitGTK is WebKit; renders, but backdrop-filter is slow on WebKitGTK | trivial |
| 24 files mention "System Settings"/"Privacy & Security"/Dock/Finder (e.g. `lib/settings/panels/capture/PermissionRows.svelte`, `lib/shell/permission-helper.ts`, `routes/onboarding/onboarding-permissions.svelte.ts:94-96`, `routes/onboarding/screens/FinaleScreen.svelte:226`) | macOS permission copy + flows | onboarding permission screens assume TCC | needs reimplementation (Linux has portals, not TCC) |
| `lib/settings/state/model-status.svelte.ts:199-211`, `lib/settings/state/gecko-url-access.svelte.ts:50` | invokes `request_apple_speech…`, `open_apple_speech_recognition_privacy_settings`, `open_browser_url_accessibility_settings` | macOS-only commands called from UI | trivial (hide on Linux) |

#### Build / CI / packaging

| file:line | item | severity |
|---|---|---|
| `apps/desktop/src-tauri/tauri.conf.json:39` | `externalBin: binaries/mnema-cli` — sidecar must exist for any `cargo check` of the desktop crate | config-only |
| `apps/desktop/src-tauri/tauri.conf.json:43-46` | `bundle.macOS` entitlements/Info.plist; no `linux` bundle section (deb/rpm/AppImage deps, `.desktop`, deep-link) | config-only |
| `apps/desktop/src-tauri/tauri.conf.json:27-32` | updater endpoint `release.mnema.day/stable/latest.json` (macOS artifacts only) | config-only |
| `apps/desktop/src-tauri/Info.plist:5-11`, `Entitlements.plist:5,12`, `mnema.provisionprofile` | TCC usage strings, audio-input, app-groups | n/a on Linux |
| `apps/desktop/src-tauri/build.rs:42-44` | Fortran static link only on macOS | config-only |
| `crates/app-infra/build.rs`, `crates/cli/build.rs` | read identifier from `tauri.conf.json` | none |
| `scripts/prepare-mnema-cli-sidecar.sh:73-93` | universal-apple-darwin via `lipo`; otherwise builds host triple (works on Linux) | config-only |
| `scripts/openblas-build-env.sh`, `scripts/ci-install-fortran.sh` | `brew install gcc` / gfortran path discovery | config-only |
| `scripts/build-macos-local-sign.sh`, `dev-app-signed.sh`, `entitle-macos-app.sh`, `stage-macos-release-artifacts.sh`, `bench-app.sh`, `release.sh` | codesign, notarytool, `.app`, `LSEnvironment`, DMG | n/a (macOS only); Linux needs new packaging script |
| `.github/workflows/macos-release.yml:24,85,191,215-243`, `macos-release-promote.yml` | `runs-on: macos-15`, `aarch64-apple-darwin`, codesign + notarize | config-only (add Linux job) |
| `.github/workflows/type-check.yml:16` | `runs-on: macos-15` — no Linux CI at all | config-only |
| `turbo.json` `passThroughEnv` | `MNEMA_*` env knobs | none |

---

### Counts

Lines mentioning `target_os = "macos"` (includes `not(...)`), per crate. "Stub lines" = lines with `not(target_os = "macos")`, i.e. Linux-facing scaffolding.

| crate | macOS cfg lines | of which `not(macos)` stubs | files with cfg | other cfg (`unix`/`windows`/`linux`) | Rust LOC |
|---|---:|---:|---:|---:|---:|
| apps/desktop/src-tauri (`mnema`) | 660 (189 in tests.rs) | 52 | 24 | 34 | 100,719 |
| capture-screen | 254 | 26 | 1 | 0 | 5,894 |
| capture-microphone | 155 | 18 | 1 | 0 | 2,852 |
| app-infra | 81 | 14 | 7 | 12 | 84,234 |
| capture-writers | 71 | **0** | 1 | 0 | 2,146 |
| audio-transcription | 56 | 5 | 4 | 0 | 5,678 |
| capture-system-audio | 41 | 1 | 2 (+5 whole modules gated) | 0 | 3,238 |
| ocr | 32 | 3 | 3 | 3 | 3,498 |
| speaker-analysis | 17 | 1 | 2 | 0 | 3,966 |
| cli | 4 | 2 | 1 | 29 | 4,327 |
| **total** | **1,371** | **122** | | | |

Desktop hot spots: `native_capture/tests.rs` 189, `native_capture/segments.rs` 115, `native_capture.rs` 83, `native_capture_output.rs` 44, `native_capture/runtime.rs` 38, `native_capture/privacy.rs` 37, `windows.rs` 28, `native_capture_metadata.rs` 20, `app_infra/frame_preview.rs` 19, `native_capture/lifecycle.rs` 18.

Whole modules gated to macOS (invisible to per-line counts): `capture-system-audio::{probe,process,rebuild,tap,writer}`, `capture-screen::equivalence` (wrongly), `capture-screen::{stream_output_delegate,stream_delegate}`, `capture-microphone::microphone_delegate`, `app-infra::capture_index_key_store::shared_group`, desktop `native_capture::{system_audio,browser_url_ax}`, `not_recording_reminder::macos_notification`, `webview_cache::macos`.

**Already platform-neutral crates (0 macOS cfg):** `ai-runtime`, `capture-types`, `capture-runtime`, `capture-vad`, `capture-metadata` (code neutral, data is macOS bundle ids + AppleScript), `secret-redaction`, `semantic-search` (Metal is an opt-in feature).

---

### Linux compile probe

Machine: Arch Linux, kernel 6.18, cargo 1.99.1, headless. a separate `CARGO_TARGET_DIR`, `timeout 900` each, `--offline` (the registry cache already held every crate, so no network was needed).

System packages present: gtk+-3.0 3.24.52, libsoup-3.0 3.8.1, openssl 3.6.5, alsa, libpipewire-0.3 1.6.10, libpulse, dbus-1, librsvg-2.0, gcc, ffmpeg. **Missing:** `webkit2gtk-4.1`, `javascriptcoregtk-4.1`, `ayatana-appindicator3-0.1`, `tesseract`, `lept`, `xdo`, plus the tools `cmake`, `clang`/libclang, `gfortran`.

#### Pass 1: repo exactly as committed

| crate | result | first real error / missing package |
|---|---|---|
| capture-types | **pass** | — |
| capture-runtime | **pass** | — |
| secret-redaction | **pass** | — |
| capture-vad (default `silero`) | **pass** | — |
| capture-metadata | **pass** | — |
| capture-writers | **pass** (empty API on Linux) | — |
| capture-microphone | **pass** | — |
| capture-system-audio | **pass** | — |
| ai-runtime | **pass** | — |
| audio-transcription (default, no features) | **pass** | — |
| semantic-search (candle-CPU) | **pass** | — |
| speaker-analysis (default, no `speakrs`) | **pass** | — |
| capture-screen | **FAIL** | `crates/capture-screen/src/lib.rs:61:9: error[E0432]: unresolved import 'equivalence'` (cfg drift; module gated at :20) |
| ocr | **FAIL** | `crates/ocr/src/paddle.rs:6:5` / `tesseract.rs:10:5: error[E0432]: unresolved import 'crate::normalize_candidate_text'` (cfg drift; fn gated at `lib.rs:730`) |
| app-infra | **FAIL** (only through its deps capture-screen + ocr) | same two errors |
| cli | **FAIL** (only through deps) | same two errors |
| desktop `mnema` | **FAIL** | `javascriptcore-rs-sys v1.1.1: The system library 'javascriptcoregtk-4.1' required by crate 'javascriptcore-rs-sys' was not found` → needs `webkit2gtk-4.1` (Arch: `webkit2gtk-4.1`). On a clean checkout the `mnema-cli` sidecar (`binaries/mnema-cli-x86_64-unknown-linux-gnu`) and the generated `apps/desktop/THIRD_PARTY_LICENSES.md` (`tauri.conf.json:41`, gitignored) are also missing; `tauri_build` fails on the latter with ``resource path `../THIRD_PARTY_LICENSES.md` doesn't exist`` |

#### Pass 2: scratch copy with the two cfg-drift lines un-gated

The fix was deleting `#[cfg(target_os = "macos")]` above `mod equivalence;` (capture-screen `lib.rs:20`) and the `#[cfg(any(target_os = "macos", test))]` on `normalize_candidate_text` (ocr `lib.rs:730`).

| check | result | detail |
|---|---|---|
| `-p capture-screen` | **pass** | 14 dead-code warnings |
| `-p ocr` | **pass** | 26 warnings |
| `-p app-infra` | **pass** | **libsqlite3-sys 0.30.1 `bundled-sqlcipher` + sqlite-vec 0.1.9 + sqlx 0.8 built on one libsqlite3-sys**; links against system openssl through openssl-sys. Answers the Phase 0 question: the storage stack is not a Linux blocker |
| `-p cli` | **pass** | — |
| `-p semantic-search` | **pass** | candle 0.10.2 CPU |
| `-p ocr --features paddle-rs,tesseract-embedded` | **FAIL** | `tesseract-rs v0.2.0: failed to execute command … is 'cmake' not installed?`; `ocr-rs v2.2.2: Unable to find libclang` → needs `cmake`, `clang` |
| `-p audio-transcription --features local-whisper,parakeet-onnx` | **FAIL** | `whisper-rs-sys v0.15.0: Unable to find libclang` → needs `clang` (and `cmake` for whisper.cpp) |
| `-p speaker-analysis --features speakrs` | **FAIL — code blocker, not a missing package** | `speakrs-0.4.2/src/lib.rs:203: error: the 'coreml' feature is only supported on macOS`, then `unresolved import 'objc2_core_ml'`. `crates/speaker-analysis/Cargo.toml:38` turns on `coreml` for every target; Linux needs a target-split dep with speakrs's ort/CPU path (speakrs depends on `ort`; it has `cuda = ["ort/cuda"]`) plus `ExecutionMode` ≠ `CoreMl` (`providers/speakrs.rs:191`). openblas-src started compiling; whether it would finish without `gfortran` is **not proven** because the rustc errors stopped the build first |

#### Pass 3: type-check of the desktop crate itself

This pass stubs out the native libraries, so it only type-checks; it does not prove the app links. Setup:
- fake `.pc` files for `webkit2gtk-4.1`, `javascriptcoregtk-4.1`, `ayatana-appindicator3-0.1` (allowed because `cargo check` never links);
- an empty sidecar file and an empty licenses file;
- `--no-default-features` (drops speakrs);
- the copy's `Cargo.toml` stripped of the `local-whisper`, `parakeet-onnx`, `paddle-rs` and `tesseract-embedded` features, because those need cmake and libclang.

Result: **the Tauri/wry/gtk stack type-checks, and the `mnema` crate stops at 14 errors, all of them macOS cfg drift in the desktop crate:**

```
native_capture/lifecycle.rs:3   E0432 super::output::{append_committed_segment_output_files, cleanup_unusable_segment_artifacts}
native_capture/segments.rs:2    E0432 (same items)
native_capture/lifecycle.rs:11  E0432 super::runtime::{system_audio_stops_with_suspension, CaptureSuspensionKind}
native_capture/lifecycle.rs:19  E0432 super::segments::{apply_microphone_output_finalization, cleanup_failed_segment_dirs, …}
native_capture/segments.rs:34   E0432 super::runtime::{privacy_suspended_sources_for_runtime_state, CaptureSuspensionKind, …}
native_capture.rs:3153-3160     E0425/E0433 privacy::{request_privacy_filter_refresh, PrivacyRefreshReason}
native_capture/lifecycle.rs:185 E0425 segments::graceful_stop_for_license_block
native_capture/runtime.rs:568   E0425 output::is_missing_requested_screen_output_failure_detail
app_infra.rs:3017,3019          E0609 no field recording_file / current_segment_output_files on NativeCaptureRuntime
windows.rs:501                  E0599 no method title_bar_style on WebviewWindowBuilder (Tauri gates it #[cfg(target_os="macos")], tauri-2.10.3 webview_window.rs:828)
```

Caveat: rustc reports name-resolution errors (E0432/E0425) before type errors, so more errors will probably surface once these are fixed. Treat 14 as a lower bound.

#### Packages Phase 0 must install (Arch names)

`webkit2gtk-4.1` (gives javascriptcoregtk-4.1), `libayatana-appindicator` (tray), `cmake`, `clang` (libclang for bindgen: whisper-rs-sys, ocr-rs), `gcc-fortran` (OpenBLAS LAPACK for speakrs `openblas-static`), and optionally `tesseract`/`leptonica` if the crate stops building them from source. Already present: gtk3, libsoup3, openssl, alsa, pipewire, ffmpeg.

---

## 3. macOS baseline: how each feature works and performs today

_Subsystem-specific detail is in §4. This section follows the whole pipeline end to end and collects every measured number in the repo._

Repo: the repo @ `main` 85dcb993. All `file:line` refs are against `main` unless marked **[branch]**.
Read-only research; nothing in the repo was changed.

There is also a **Windows port on unmerged branches** (`origin/windows-key-stores-and-final-gaps`, last commit 2026-07-13) that already solved most "non-Apple" problems: a change-driven capture backend with a pure frame-rate cap (`crates/capture-screen/src/frame_schedule.rs`), a `media-decode` crate, speakrs on **CPU** (10-file ONNX subset instead of 76-file CoreML set, commit 48c81f6f), and a measured **candle-CPU** embedding run (commit 62d6340c). It is the closest template for Linux.

---

### Headline numbers (every measured figure found)

All power numbers are a fanless **M4 MacBook Air**, macOS 26.x, unless stated.

| What | Number | Source |
|---|---|---|
| Whole-app marginal power while recording (before OCR retune) | **1.4 W** (9.1 W recording vs 7.7 W queue drained), 57.5 °C vs 52.8 °C; "OCR is the whole of Mnema's CPU cost" (stack shows `VNRecognizeTextRequestRevision3`, CoreRecognition, ANE queues) | `apps/desktop/src-tauri/src/ocr_budget.rs:35-41`, commit 47b418f7 |
| OCR duty cycle | **20 %** while recording (multiplier 4.0), **40 %** catch-up when paused/stopped (1.5); clamp 1–10 s / 0.25–2 s | `ocr_budget.rs:31-48,514-526` |
| SCK privacy filter cost (ADR 0006) | no filter ~**649 mW**; app-exclusion ~**867 mW**; window-only ~2245 mW; app+excepted windows ~2889 mW (CPU/GPU/ANE rails only) | `docs/adr/0006-use-app-level-live-privacy-exclusion-only.md:11-14` |
| Same, with DRAM rails **[branch power/05]** | plain display filter ~+0.2 W; excluding one running app ~**+2.4 W** (GPU busy 64 %, DRAM power ×5) — "most of screen recording's **+3.7 W**" | commit 64fa1080 (not on main) |
| Mic left running while locked **[branch power/03]** | **+0.67 W** (mic-only bench) | commit 73161da5 |
| Audio-only display poll **[branch power/01]** | `CGDisplayCreateImage` ~**22 MB** Retina readback 2–3×/s per display just for an activity fingerprint; still on main at `crates/capture-screen/src/lib.rs:1067-1094` | commit 7229415b |
| Embedding (old fastembed/ort, CPU) | ~**140 % CPU** on P-cores, ~**1.7 GB** resident | `docs/adr/0037-...md:18-19` |
| Embedding (candle Metal F16) | ~**35 % CPU**, ~**720 MB** RAM; F16 ~11 % slower than F32; cosine 1.0 at F32, 0.986 overlap at F16 | `docs/adr/0037-...md:21-23,58-60` |
| Embedding backfill power | uncapped pacing: **65 % duty, 7.4 W**, rose to 86 % under Low Power Mode; two concurrent embed workers **10.2 W** (+38 %, +9 °C, 86 % of GPU saturation); after fix one 16-anchor batch ≈ **3.5 s**, duty = 1/(1+3.0) = 25 % | `apps/desktop/src-tauri/src/semantic_search_worker.rs:183-224`, commit 54870754 |
| Embedding per chunk cap | cap 1: 2.75 W, 4.53 anchors/s; cap 2 (shipped): **2.81 W, 2.40 anchors/s**, ~7 min / 1000 anchors; cap 3: 2.94 W, 1.60 anchors/s | commit 69b8d4fa, `docs/adr/0037-...md:160-170` |
| Embedder memory | process peak **2.0 GB** with embedders warm; 574 MB unmapped-graphics returned on release (candle Metal `private_buffers` never swept); steady ~800–890 MB avg, ~1.9 GB peaks; model file 547 MB, reload churn was ~8 GB/h of reads → 0.5 GB/h after warm-lifetime 300→3600 s | `semantic_search_worker.rs:82-105`, commit 19c36e43 |
| **candle-CPU on Windows x86_64** (nearest Linux proxy) **[branch]** | nomic-embed-text-v1.5: **5.33 texts/s** batched (~7.6 k chars/s) at **~7.2 cores**, 0.76 s load, **542 MiB** after load (1052 MiB transient peak), **~24 ms/query** | commit 62d6340c |
| DB read amplification fixed | sqlite-vec anti-join was ~**210 MB of DB read per worker pass** on a 970 MB install | CLAUDE.md "Semantic Index Backfill" |
| Vector size | int8[768] = **768 B/vector** (was 3072 B f32) | commit 98fe4584 |
| Disk per day (measured anchor) | **270 MB/day** at one frame / 3 s, 720p, medium bitrate, pause-on-inactivity on, both audio sources, 14-day window = 168 video + 47 OCR index + 2×23 audio + 2×4.5 transcript; ~405 MB/day at the 2 s default | `apps/desktop/src/lib/onboarding/disk-estimate.ts:1-30`, `feature-cost.ts:40-60` |
| Gecko URL read via AX | ~**1.4 s** per read | `docs/agents/capture-pipeline.md:169` |
| JPEG export latency | ~**100–300 ms** after capture, more under load | `docs/agents/capture-pipeline.md:170` |
| Privacy frame guard **[branch]** | CGWindowList check ~**1.3 ms** per frame | commit 64fa1080 |

Benchmark harness of record: `scripts/bench-app.sh` (signed release build, own identity) + `scripts/bench-day/collect.sh` (macmon power, powermetrics **per-coalition** CPU/GPU, `vmmap` physical footprint — "never `ps` RSS", collect.sh:34-36) + `report.py`. Commit 22f867ac says the public numbers come from a **23.5 h** loaded day, an idle attribution run, and a **60 min** active run — the results themselves are **not in the repo**. Also: `crates/ocr/examples/*_benchmark.rs` (no recorded results), `crates/semantic-search/examples/thermal.rs`, `scripts/semantic_bench/`, `scripts/diarization_bench/` (DER, not speed).

Key caveat for Linux: on macOS a lot of cost lands in **other processes** (WindowServer composing the filtered stream, mediaanalysisd, coreaudiod) — collect.sh:24-27 says per-process numbers "made rem look cheap while mediaanalysisd burned 30-50%". On Linux the analogues are the compositor (gnome-shell/kwin), PipeWire and wireplumber; a Linux bench must measure them too.

---

### Defaults that set the load

From `default_recording_settings()` (`apps/desktop/src-tauri/src/native_capture_settings.rs:92-138`):

- Screen **on**, mic **off**, system audio **off**, auto-start **on**, retention **Never**.
- **0.5 fps** (one frame / 2 s); allowed 1/60 … 10 fps (`:41-42`).
- **720p** (Original only where SCK scaling is unavailable).
- Segment **60 s**, hard max 300 s (`:37`, ADR 0005).
- Bitrate = w×h×fps×{0.07,0.10,0.14}, clamped **500 kbps … 120 Mbps**, 250 kbps steps (`:43-45,608-660`). At defaults the formula gives ~46 kbps so the **500 kbps floor** applies; real VBR usage is ~1/10 of that (disk-estimate.ts:13-17).
- Pause on inactivity **on**, idle timeout **10 s** (`crates/capture-types/src/recording.rs:72-78`).
- OCR: Apple Vision, mode **Fast**, language correction off (`recording.rs:186-207`). (ADR 0004 forbids *falling back* to Fast; Fast is still the shipped default.)
- Transcription: Local Whisper **base**, mic on / system audio off, 30 s chunks, unload after 300 s idle (`recording.rs:299-340`).
- Speaker separation/recognition **off** by default (`recording.rs:386-392`).
- VAD: **Silero** (`recording.rs:98`).
- Semantic search: model-gated, no model ships (ADR 0037).

---

### Per-feature pipeline baseline

#### Screen capture (ScreenCaptureKit)

**Path.** `start_screen_capture_kit_session` (`crates/capture-screen/src/lib.rs:3240`) → `SCShareableContent.current` with a 20 s timeout (`:3283-3311`) → **first display only** (`displays.first()`, `:3314`) → content filter (privacy, §3.11) → `configured_screen_capture_kit_stream_cfg` (`:3415-3442`):
- width/height from resolution preset, `minimumFrameInterval = 1000/fps ms` (`:3428-3431`),
- pixel format **32BGRA** (so equivalence can read packed pixels, `:3432-3434`),
- `shows_cursor = true` (`:3435`; the cursor is removed only on **[branch power/04]**),
- `captures_audio=false`, `capture_mic=false` (`:3438-3439`),
- `queueDepth` **not set** → SDK default 8 surfaces (branch power/04 sets 3).
Output delegate on a serial dispatch queue (`:3362-3384`).

**Per frame** (`handle_stream_did_output_sample_buf`, `:1870-1940`):
1. Drop unless `SCFrameStatus == Complete` and the buffer has an image (`should_append_screen_sample`, `:2052-2085`). Idle/blank frames never reach the writer.
2. Activity fingerprint over `contentRect`/`screenRect` + sparse pixel probe (dirty rects deliberately ignored because cursor churns them, `:783-806`).
3. Frame-equivalence fingerprint: 8×8 hint grid, 16×16 proof grid, 3×3 samples/tile quantised by 32 (`crates/capture-screen/src/equivalence.rs:1-12`) — pure Rust, CPU, sparse; portable as is.
4. Lazily create the AVAssetWriter (H.264, `AVVideoAverageBitRateKey`, real-time) (`crates/capture-writers/src/lib.rs:1107-1180`); append the sample. VideoToolbox hardware encode is implied (not configured explicitly).
5. If appended and ≥ `DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL` (1 s, `lib.rs:70`) since last export → hop to a second serial queue → `CGImage` from `CVPixelBuffer` → JPEG via `CGImageDestination` (`:1656-1710,1729-1800`); push a frame-index entry with live `video_offset_ms` (`.frame-index.bin`, 24-byte entries, `:215-218`).
6. `on_frame_exported` → bounded Tokio mpsc (capacity **64**, backpressure not drop) → frame artifact worker → `FrameBatchStore` → OCR job enqueue in the same transaction (`apps/desktop/src-tauri/src/native_capture/segments.rs:40-45,2172-2182`; capture-pipeline.md:84).

**When.** Frames arrive at most every `1/fps` and only when content changes; rotation every 60 s (segment loop 1 s tick, `segments.rs:46`; `tick_rotation`, `lifecycle.rs:975`).

**Threads.** Apple callback queues (stream output, export), a dedicated `mnema-native-capture-*` std thread for the segment loop (`segments.rs:162-164`), Tokio for persistence.

**Performance.** Frames are GPU-memory IOSurfaces; scaling and BGRA conversion done in hardware by SCK; H.264 in VideoToolbox. Only CPU touches: the sparse fingerprints and the 1/s JPEG encode. Screen recording's total cost per branch bench ≈ **+3.7 W**, of which ~2.4 W was the app-exclusion filter making WindowServer compose a second display copy.

**OS guarantees relied on.** SCK only delivers Complete frames on change (Idle status otherwise); in-compositor app exclusion; hardware scaling; IOSurface zero-copy; TCC Screen Recording grant persisting per signed identity.

**Failure modes handled.** Delegate stop `-3815` on display sleep → `DisplayUnavailable` suspension (screen only) with tail-segment commit, 2 s recovery probe via `CGGetActiveDisplayList` (`segments.rs:52`, `lib.rs:1154`), never escalates (ADR 0021). Writer `-11800`, empty/missing-`moov` finalization rejected (`lib.rs:455-485`). Rotation backstop suspends instead of failing (ADR 0021 amendment). **Multi-monitor: only the first display is recorded** — a gap the Linux port should match deliberately or fix.

**How macOS achieves it.** SCK hands back IOSurface-backed GPU buffers with hardware-accelerated capture, scaling and pixel/colour conversion "to achieve high-performance capture with reduced CPU usage", plus per-frame metadata and dirty rects ([WWDC22 10155](https://developer.apple.com/videos/play/wwdc2022/10155?time=276)). `SCFrameStatus` has Complete/Idle/Blank/Suspended ([binding docs](https://docs.rs/objc2-screen-capture-kit/latest/objc2_screen_capture_kit/struct.SCStreamConfiguration.html); exact Idle wording on [Apple's page](https://developer.apple.com/documentation/screencapturekit/scframestatus/idle) not retrievable — *treat "Idle = no change" as Apple-documented but unverified here*). `minimumFrameInterval` default is 1/60 s.

**Linux counterpart.** xdg-desktop-portal ScreenCast → PipeWire stream. DMA-BUF is offered first, SHM/memfd as fallback; mmap-reading tiled DMA-BUFs can be "extremely slow", so import through EGL/Vulkan/VA-API ([PipeWire DMA-BUF docs](https://antlarr.pages.freedesktop.org/pipewire/page_dma_buf.html)); memfd means more CPU, especially on NVIDIA ([KDE T12863](https://phabricator.kde.org/T12863)). Damage metadata (`SPA_META_VideoDamage`) is optional — Hyprland sends it ([xdph commit](https://github.com/hyprwm/xdg-desktop-portal-hyprland/commit/e2b68a6d39eb72b6d945f0f7f47a3f317fae79ad)), Mutter historically did not ([mutter#1298](https://gitlab.gnome.org/GNOME/mutter/-/issues/1298)), and dropped buffers lose damage events ([KDE 510561](https://bugs.kde.org/show_bug.cgi?id=510561)). Frame rate is negotiated as a variable range ([mutter 2018 commit](https://lists.gnome.org/archives/commits-list/2018-July/msg01696.html)); the Windows branch's `frame_schedule.rs` cap logic carries over. Permission: portal dialog per session unless `persist_mode=2` + single-use `restore_token` ([portal ScreenCast spec](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.ScreenCast.xml); transient mode broke in portal 1.18 per [issue 1124](https://github.com/flatpak/xdg-desktop-portal/issues/1124) — *version-dependent*). Encoding: VA-API/NVENC via GStreamer/FFmpeg, else x264 on CPU.

#### Frame export, dedup and OCR

**Dedup.** Two-stage: capture-time fingerprint (above) + `CapturedFrameEquivalenceResolver` in app-infra skips the OCR job when an earlier equivalent frame already has one (capture-pipeline.md:79-84). Pure Rust → portable.

**Admission budget** (`apps/desktop/src-tauri/src/ocr_budget.rs:11-30`): high queue pressure ≥ **3** jobs; a "representative" read every **15 s**; visual-novelty reads at most one per **2 s** per scope, ring of 128 fingerprints, suppressed after **10** consecutive novel frames (video). Budget state is in-memory only (ADR 0004).

**Execution.** Worker `NonTranscriptionAndFrameBatch`, a Tokio task polling every **500 ms** idle / 2 s after error (`apps/desktop/src-tauri/src/app_infra.rs:43-44,2530-2560`). Cooldown = run time × 4.0 (recording) or × 1.5 (catch-up), clamped (`ocr_budget.rs:31-48`). Note: Apple Vision runs **synchronously inside the async fn** (`crates/app-infra/src/processing/engine.rs:92-99`), i.e. it occupies a Tokio worker thread; Paddle uses `spawn_blocking` (`crates/ocr/src/paddle.rs:175`).

**Vision call** (`crates/app-infra/src/processing/apple_vision.rs:162-345`): decode JPEG with `image`, downscale so the long side ≤ **1800 px** (`:29,202-215`), convert to 8-bit grey, wrap in a CVPixelBuffer, `VNRecognizeTextRequest` level Fast/Accurate, language correction per setting, inside an autorelease pool. Stores text + bounding boxes (geometry zstd-compressed, commit d308f386).

**Performance.** OCR was the entire CPU cost of the app (commit 47b418f7). Vision runs partly on the ANE (stack trace names ANE queues). Public community timings: Vision accurate ~207–233 ms, fast ~131–200 ms per image on M-series ([ocrmac README](https://github.com/straussmaximilian/ocrmac)) — *single image, size unstated*.

**Linux counterpart.** Tesseract (`tesseract-rs`, feature `tesseract-embedded`, `crates/ocr/Cargo.toml:10,19`) or PaddleOCR via `ocr-rs`/ONNX (`:8-9,15`). Third-party CPU numbers for document pages: Tesseract ~0.16 s (800×600) and RapidOCR (Paddle models on ONNX) ~0.21 s ([Codesota](https://codesota.com/ocr/best-for-python)); Tesseract LSTM 0.3–0.8 s/page ([imagetotable](https://imagetotable.ai/blog/ocr-speed-vs-accuracy-tradeoff)) — *not screenshots, not 1800 px; measure*. Screenpipe's own docs say Linux has "Tesseract (only option)" and call Tesseract "slower, higher-CPU" than Vision ([screenpipe perf doc](https://www.mintlify.com/screenpipe/screenpipe/advanced/performance)). No ANE on Linux; everything is CPU unless an ONNX GPU provider is added. The duty-cycle governor (1/(1+m)) self-corrects for slower OCR, so it trades **indexing lag**, not watts.

#### Redaction, FTS, embeddings, storage

- **Secret redaction**: pure Rust regex/heuristics, `secret-redaction-v3`, 512-char windows (`crates/secret-redaction/src/lib.rs:9-10`), applied inside the result-persist transaction (`crates/app-infra/src/processing/secret_redaction_pipeline.rs:146`). Portable, cheap.
- **Storage**: SQLCipher via `libsqlite3-sys` `bundled-sqlcipher` + `sqlite-vec =0.1.9` + FTS5 (`crates/app-infra/Cargo.toml:36-41`). WAL, writer pool 4 conns, busy_timeout 10 s, `synchronous=NORMAL` (`crates/app-infra/src/db.rs:240-320,370-373`). On Linux `bundled-sqlcipher` needs a crypto library (OpenSSL) where macOS uses CommonCrypto — *libsqlite3-sys behaviour, verify* (there is a `bundled-sqlcipher-vendored-openssl` feature).
- **Embeddings**: semantic backfill worker, 16 anchors/pass, sub-batches of 8, 256-token window, first 2 windows per document (`crates/semantic-search/src/runtime.rs:87,96,110`; `semantic_search_worker.rs:57`), idle poll 20 s, drop embedder after ~10 min idle, recycle after 60 min (`:62,80,105`), one global GPU slot for the two background embedders (`:223`). Cooldown 3.0× work, 150 ms–30 s (`:183-206`). The comment at `:162-170` already anticipates Linux: on candle-CPU "the forward is the CPU burn the cooldown paces".
- **Linux**: candle-CPU F32 (`backend/candle.rs:110-112`). Windows proxy: ~7.2 cores while embedding, 5.33 texts/s. At 25 % duty that is ~1.8 cores average during backfill — an order above the Metal path's ~35 % of one core. candle's CUDA backend is deferred (ADR 0037:61); no Vulkan backend in candle. Unified memory isn't a factor on CPU, but RAM rises (F32 weights ≈ 2× F16; ModernBERT arms ~596 MB, ADR 0037:155).

#### Microphone

**Path.** AVCaptureSession + `AVCaptureAudioDataOutput` delegate on a dispatch queue (`crates/capture-microphone/src/lib.rs:1662-1759`). Format observed from samples, writer created once the format is stable (3 consecutive / 5 observed, `:469-471`), pending buffer ≤128 samples (`:476`). AAC m4a 48 kHz ≤2 ch via AVAssetWriter (`crates/capture-writers/src/lib.rs:583,843-849`). Peak-since-last-poll kept for the 1 s inactivity tick (`capture-microphone/src/lib.rs:154-160,402-421`). A VAD feed resamples to **16 kHz, 320-sample (20 ms) frames**, ring of 96 (`:71-74,1178-1260`); idle-pause tail trim uses VAD boundaries.

**Linux counterpart.** PipeWire capture stream (or cpal → ALSA/Pulse/PipeWire), encode AAC/Opus via GStreamer/FFmpeg. No per-app mic TCC prompt on bare Linux (Flatpak: portal/device permission). Mic indicator is DE-specific.

#### System audio (Core Audio process tap)

**Path.** `SystemAudioTapSession::start`: global stereo tap **excluding** a process-object list, unmuted, private (`crates/capture-system-audio/src/tap.rs:61-63`) → private aggregate device with the real output as main sub-device and drift compensation (`:202,218`) → IOProc block on a non-nil serial dispatch queue (`:97-120`). Writer follows the tap format (44.1/48 kHz, or 16 kHz on Bluetooth HFP), ≤8 queued buffers (`writer.rs:47-52`). Zero-watchdog 30 s → 600 s backoff (`watchdog.rs:21-26`). Rebuild on default-device change / device death / exclude-list move / watchdog; every rebuild is a segment boundary (ADR 0052). Exclude list = own pid + privacy-listed apps, kept in sync by a listener on the audio-process list (`exclude.rs:156-280`).

**OS guarantees.** Per-process exclusion inside coreaudiod; TCC category with no query API (permission is inferred, `permission.rs`). Tap API is macOS 14.2+ per [Apple](https://developer.apple.com/documentation/coreaudio/audiohardwarecreateprocesstap(_:_:).md) (AudioCap says 14.4 — *conflicting*); app gate is 15.0.

**Linux counterpart.** PipeWire monitor of the default sink, or per-node links. Excluding specific apps means routing/linking around them (e.g. OBS's PipeWire app-capture "capture all except" mode, [obs-pipewire-audio-capture](https://obsproject.com/forum/resources/pipewire-audio-capture.1458/updates)) — there is no single "global tap minus processes" primitive; the port must build the exclusion graph itself and rebuild on node add/remove (same shape as the existing rebuild engine).

#### VAD

Silero (ONNX via `silero 0.3`) default, WebRTC VAD alternative, exact selection with no fallback (`crates/capture-vad/Cargo.toml:7-13`, capture-pipeline.md:140). Model path `resources/vad/silero_vad.onnx`, threshold 0.5 (`src/silero_vad.rs:5-9`). System audio speech gating is a post-hoc `system_audio_speech_activity` job (capture-pipeline.md:142). Portable; ONNX Runtime CPU on Linux like macOS.

#### Transcription

Worker `AudioTranscription` (Tokio, `app_infra.rs:2545-2551`) picks jobs enqueued at segment commit. Audio decode is **AVAssetReader** (`crates/audio-transcription/src/macos_audio_decode.rs`, 934 lines) → mono f32. Whisper (Metal on macOS, ggml CPU elsewhere), contexts cached and evicted after 120 s idle by a `whisper-idle-unload` thread (`providers/local_whisper.rs:26,285-345`). Parakeet ONNX **CPU** everywhere, idle-unload threads (`providers/parakeet.rs:545-600,894-946`). Apple Speech provider is macOS-only. Deepgram is plain HTTPS (portable).

**Numbers.** whisper.cpp base encode with Metal on M1 Pro ~60–71 ms ([whisper.cpp releases](https://www.github.com/ggerganov/whisper.cpp/releases), *column meaning inferred*). No CPU baseline found; Linux would be ggml CPU (AVX2) or a Vulkan/CUDA build.

**Linux gap.** Audio decode (need FFmpeg/GStreamer/symphonia — the Windows branch built `crates/media-decode`), Apple Speech hidden, Whisper loses Metal.

#### Diarization

speakrs CoreML in a subprocess (`speaker_analysis_runtime.rs`), AVFoundation decode (`crates/speaker-analysis/src/macos_audio_decode.rs`). Off by default. speakrs README: CoreML on M4 Pro **529× realtime**, 7.1 % DER vs pyannote MPS 24×; CUDA 59–121× ([speakrs docs](https://docs.rs/crate/speakrs/latest/source/README.md)). **CPU RTF not published** — the Windows branch runs the CPU mode (ORT + MKL) but recorded no speed. On Linux: ORT CPU + OpenBLAS/MKL; expect one to two orders of magnitude slower than CoreML (*inference from the CoreML vs CUDA spread, unmeasured*).

#### Window/app metadata and browser URL

`NSWorkspace` notifications (activate/deactivate/space change → focus refresh; launch/terminate → app refresh) (`apps/desktop/src-tauri/src/native_capture_metadata.rs:218-245`) + 1 s privacy poll fallback (`segments.rs:47`) + `CGWindowListCopyWindowInfo` on-screen windows (`native_capture_metadata.rs:477-504`). Snapshot ring of 32 so frames are stamped by **capture time** (`:28`, capture-pipeline.md:170). Browser URL: `osascript` subprocess for Chromium/WebKit (`:614`), AX `AXURL` for Gecko (~1.4 s, budgets 400/500 ms, `native_capture_browser_url_ax.rs:116-124`); title-gated cache, 5 s backstop, 1.5 s re-probe floor (`crates/capture-metadata/src/lib.rs:12,24`).

**Linux.** X11: `_NET_ACTIVE_WINDOW`/`WM_CLASS`. Wayland has no portable "active window" API; wlr-foreign-toplevel gives title/app_id/activated on Sway/Wayfire/etc. ([wayland.app](https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1)) but not GNOME/KDE (they need shell extensions / KWin scripts — *not verified here*). Browser URL: AT-SPI accessibility tree or nothing.

#### Inactivity / idle

1 s segment-loop decisions (capture-pipeline.md:137) from: `CGEventSourceSecondsSinceLastEventType(CombinedSessionState, any)` (`native_capture_system_idle.rs:8-43`), SCK frame fingerprints (debounce 250 ms, `capture-screen/src/lib.rs:441`), mic/system-audio peaks. Without a screen stream, main still reads every display with `CGDisplayCreateImage` for a fingerprint (`lib.rs:1067-1094`) — removed on branch power/01 as a ~22 MB readback 2–3×/s.

**Linux.** `ext-idle-notify-v1` (Mutter 45+, KWin 5.27+, Sway, Hyprland), Mutter `org.gnome.Mutter.IdleMonitor.GetIdletime` D-Bus, X11 XScreenSaver ([wayland.app ext-idle-notify](https://wayland.app/protocols/ext-idle-notify-v1), [user-idle3](https://docs.rs/crate/user-idle3/0.7.2)). ext-idle-notify is **threshold-notification**, not a "seconds since input" read — the inactivity code's polling shape needs adapting.

#### Privacy exclusion

Windows: SCK `ContentFilter::with_display_excluding_apps_excepting_windows` (`capture-screen/src/lib.rs:2707-2790`), recomputed by the 1 s privacy poll, coalesced generations (`native_capture/privacy.rs`). Audio: tap exclude list (§3.5). On main, Mnema's own windows are **not** hidden (`sharingType=.none` only on branch power/04). Cost: +218 mW (ADR 0006) to +2.4 W with DRAM (branch).

**Linux.** No portal API excludes apps from a monitor stream. Options: per-window streams instead of monitor, or post-capture frame dropping when an excluded app is visible (the branch's "drop frames while a left-out app shows a window", fails closed) — requires reliable window geometry, which Wayland withholds. SUPPORTS.md principle: never silently downgrade; expose "degraded".

#### Sleep/wake, display-unavailable, lock

`NSWorkspaceWillSleep`/`DidWake`/`ScreensDidSleep`/`ScreensDidWake` observers (`native_capture.rs:1465-1500`, `handle_system_will_sleep` `:1302`) + `CGDisplayRegisterReconfigurationCallback` for dark/deep-idle wakes (`:1634`). Screen-only suspension, mic/system audio continue (ADR 0021/0052). Lock screen frames are tagged `loginwindow` and treated as away (capture-pipeline.md:151). Branch power/03 stops the mic while locked/displays asleep.

**Linux.** logind `PrepareForSleep` (D-Bus), session `Lock`/`Unlock`/`LockedHint`; PipeWire stream state changes / node removal on monitor unplug; DRM hotplug via udev. Lock screen on Wayland usually stops portal frames entirely (compositor-owned) — *verify per DE*.

#### Low disk

`fs2::available_space` at segment boundaries only (`native_capture/disk_space.rs:159-163`), floor 1 GiB, estimate `(bitrate/8 + 48 000 B/s) × segment` (`:29,37`), recovery probe 10 s (`segments.rs:58`), all sources suspended, partial deleted on ENOSPC (ADR 0040). Fully portable (`statvfs`).

#### Secrets and DB key

Vault master in Keychain `day.mnema.vault` (`crates/app-infra/src/secret_vault.rs:20`); DB key in DP keychain access group `day.mnema.capture-index`, with `/usr/bin/security` fallback (`capture_index_key_store.rs:22,369-436`). Relies on Keychain ACL + code-signature identity so the `mnema-cli` sidecar reads silently.

**Linux.** Secret Service (gnome-keyring/KWallet) over D-Bus; a locked collection triggers a prompt and a client can block indefinitely waiting ([dbus-secret-service docs](https://docs.rs/dbus-secret-service)); headless sessions need manual unlock. No per-binary ACL equivalent — any process in the session can read the item, so the CLI-sidecar trust story weakens.

#### Notifications and webview

`UNUserNotificationCenter` for the not-recording reminder (`native_capture/not_recording_reminder/macos_notification.rs`), poll 30 s (`not_recording_reminder.rs:26`). WKWebView memory-cache purge on blur, ≥60 s apart (`webview_cache.rs:21`) and a render-idle gate because WebKit strands IOSurfaces when repainting with screens asleep (SUPPORTS.md rows). Linux: `org.freedesktop.Notifications`; WebKitGTK has different memory behaviour — the WKWebView workarounds don't apply but its own leaks are unmeasured.

---

### One line per feature

- **Screen capture**: macOS gets change-only BGRA frames at ≤1/fps with hardware scaling via SCK IOSurfaces; on Linux portal+PipeWire DMA-BUF (or memfd) with optional damage via the compositor; gap = no guaranteed "idle" status (must hash/damage-check ourselves), scaling/conversion may land on CPU, per-session permission dialog unless restore tokens work, first-display-only behaviour must be chosen again.
- **Encode**: macOS does H.264 with VideoToolbox via AVAssetWriter; on Linux H.264 with VA-API/NVENC (or x264 CPU) via GStreamer/FFmpeg; gap = no universal hardware encoder, at 0.5 fps the CPU fallback is cheap.
- **Frame export/JPEG**: macOS does CGImage→JPEG via ImageIO on a dispatch queue; on Linux `image`/turbojpeg after a GPU→CPU download; gap = a DMA-BUF readback per exported frame (1/s max).
- **Dedup**: pure Rust on BGRA bytes; same on Linux; gap = only needs mapped pixels.
- **OCR**: macOS does Vision Fast/Accurate on ANE+CPU at 20 % duty, ~1.4 W marginal; on Linux Tesseract or Paddle-ONNX on CPU; gap = several-times slower per frame, no ANE, so either longer index lag at the same duty or more watts.
- **Redaction/FTS**: Rust + SQLCipher FTS5 both sides; gap = SQLCipher crypto backend (CommonCrypto → OpenSSL).
- **Embeddings**: macOS does candle Metal F16 ~35 % CPU / ~720 MB; on Linux candle-CPU F32 ~7 cores while running, ~540 MB (Windows proxy); gap = ~20× CPU while embedding, needs heavier pacing or CUDA.
- **Mic**: AVCaptureSession → AAC; on Linux PipeWire/cpal → AAC/Opus via GStreamer/FFmpeg; gap = no AVAssetWriter, own encoder, no TCC prompt.
- **System audio**: Core Audio global tap minus processes, rebuilt on device events; on Linux PipeWire monitor/links built by the app; gap = no "exclude these processes" primitive, privacy parity must be hand-built.
- **VAD**: Silero ONNX / WebRTC both sides; gap ≈ none.
- **Transcription**: Whisper on Metal, Parakeet on ORT CPU, decode via AVAssetReader; on Linux Whisper on CPU (or Vulkan/CUDA), Parakeet unchanged, decode via FFmpeg/symphonia; gap = Whisper GPU loss and a new decoder.
- **Diarization**: speakrs CoreML ~529× realtime in a subprocess; on Linux speakrs ORT CPU (+OpenBLAS); gap = unmeasured CPU RTF, likely 10–100× slower, model set differs (10-file ONNX subset).
- **Window metadata**: NSWorkspace notifications + CGWindowList; on Linux X11 EWMH or compositor-specific Wayland (wlr-foreign-toplevel, GNOME/KDE extensions); gap = no portable Wayland API for focused app.
- **Browser URL**: AppleScript/AX; on Linux AT-SPI; gap = unproven, likely partial.
- **Idle**: CGEventSource seconds-since-input; on Linux ext-idle-notify / Mutter D-Bus / XScreenSaver; gap = notify-style API, DE fragmentation.
- **Sleep/wake/display**: NSWorkspace + CG reconfiguration callback + SCK delegate; on Linux logind PrepareForSleep + session lock + PipeWire stream state; gap = different event set, lock behaviour compositor-dependent.
- **Low disk**: `fs2` statvfs both sides; gap = none.
- **Privacy exclusion**: SCK in-compositor app filter + tap exclude list; on Linux no portal-level exclusion; gap = largest privacy gap, needs per-window capture or frame drop on visibility (needs window geometry).
- **Secrets**: Keychain with per-binary ACL; on Linux Secret Service over D-Bus; gap = no binary-scoped ACL, locked-keyring prompts.
- **Notifications**: UNUserNotificationCenter; on Linux freedesktop Notifications; gap = small.
- **Webview**: WKWebView + IOSurface workarounds; on Linux WebKitGTK; gap = memory profile unmeasured.

---

### Five biggest expected performance gaps

1. **OCR without Vision/ANE.** It is already the whole of Mnema's CPU cost on macOS. Tesseract/Paddle on CPU is several times slower per 1800-px frame; the 1/(1+m) governor holds watts and pushes index lag instead. Needs a Linux OCR bench on real 720p frames before picking a duty.
2. **Embeddings on candle-CPU.** ~7.2 cores while embedding vs ~35 % of one core on Metal; 25 % duty → ~1.8 cores average during backfill. Probably needs a lower duty on Linux, CUDA as an opt-in, or keep semantic search off by default.
3. **Screen frame path off the GPU.** SCK delivers scaled BGRA IOSurfaces only on change. PipeWire may deliver tiled DMA-BUFs (need GPU import) or SHM copies at full resolution, with damage info unreliable on GNOME; scaling 4K→720p and the per-export readback may land on CPU. Must hash frames ourselves to emulate "Idle".
4. **Diarization and Whisper lose CoreML/Metal.** speakrs CoreML 529× realtime and Whisper Metal won't exist; CPU RTF is unmeasured. Both are background jobs, so the cost is CPU minutes per recorded hour.
5. **Privacy exclusion and compositor cost.** macOS already showed app exclusion can cost +2.4 W in WindowServer. Linux has no exclusion primitive; any replacement (per-window streams, visibility checks) adds compositor work and depends on Wayland window geometry that is often unavailable.

Honourable mentions: SQLCipher crypto backend swap; per-session portal permission dialogs; no shipped Linux bench harness (macmon/powermetrics/vmmap are macOS tools — Linux needs RAPL/`perf`/`smem`/`pidstat` and must include the compositor + PipeWire + wireplumber processes).

### Things not checked

- Bench-day result files (not in repo); exact SCK `.idle` wording from Apple.
- GNOME/KDE focused-window APIs, AT-SPI URL reliability, lock-screen PipeWire behaviour.
- Tesseract/Paddle/Whisper/speakrs CPU speed on Linux hardware (none measured anywhere in the repo or found online for this workload).

## 4. Subsystems

### 4.1 Screen capture, video writing and frame previews


_Researched 2026-11-01. Repo read-only at `main` 85dcb993. No builds run._

**One-line answer:** Record the screen on Linux through the desktop's **screen-sharing portal + PipeWire** (`ashpd` 0.13 + `pipewire` 0.10), ask once and keep a **"remember this" token**, and replace AVFoundation with a **self-contained Rust media stack**: OpenH264 (encode and decode H.264), `mp4-atom` (write/read the file), and the `image` crate (JPEG, already a dependency). Don't use GStreamer or FFmpeg. X11 is a small fallback through `xcap`. **Live per-app exclusion cannot match macOS on GNOME.** Make it a capability that degrades, and record that in an ADR.

---

#### (a) Current macOS implementation (file refs)

| Piece | Where | What it does |
|---|---|---|
| Capture backend | `crates/capture-screen/src/lib.rs:3240-3412` (`start_screen_capture_kit_session`) | ScreenCaptureKit (SCK) stream, needs macOS 15 (`:3251`). Captures **only the first display** (`displays.first()`, `:3314`). The privacy filter also targets display `0` = first (`apps/desktop/src-tauri/src/native_capture/privacy.rs:208`). |
| Stream config | `lib.rs:3414-3443` | SCK scales on the GPU to the requested size (`set_width/height`). Frame interval goes below 1 fps (default **0.5 fps**, `native_capture_settings.rs:98`, range 1/60–10 fps `:41-42`). BGRA pixels, cursor shown (`:3436`), no audio. |
| Legacy fallback | `lib.rs:3062+` (`start_avfoundation_capture_session`) | AVCaptureScreenInput. No frame export, original resolution only. Not relevant to Linux. |
| Idle-frame skipping | `lib.rs:2052-2085` | Only SCK frames with status `Complete` get appended, so unchanged screens add no frames (variable frame rate). |
| Video writer | `crates/capture-writers/src/lib.rs:1061-1267` | `AVAssetWriter` writes QuickTime `.mov` with **H.264** (`:1121`) at an optional average bitrate (`:1128-1137`; 0.5–120 Mbps clamp, `native_capture_settings.rs:43-44`). Created lazily from the first sample, real-time input. |
| Segment rotation / pause | `capture-screen/src/lib.rs:2586-2697` | The stream stays live. The writer context is swapped (rotate) or finished (inactivity pause). Segments are capped at 5 min (ADR 0005). |
| Frame export (JPEG for OCR) | `lib.rs:1729-1815`, `1656-1709` | Throttled to 1 s (`DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL`, `:70`). Pixel buffer → CGImage → ImageIO JPEG on a serial queue. Records `frame-<ts>-<idx>.jpg` in a hidden workspace. |
| Frame index sidecar | `lib.rs:215-348` (format `SFI1`), `:1324+` (finalize reads the real sample times through AVAssetReader), `:1514` (rebuild) | `video_offset_ms = sample_pts − first_pts`, taken live. Pairing by position is forbidden (`docs/agents/capture-pipeline.md:54`). |
| Finalize validation | `lib.rs:2106-2150` | Reopens the `.mov` and requires a video track. Unopenable files are rejected. |
| Activity fingerprint | `lib.rs:547-1139` | Per-sample pixel probes, plus `CGDisplayCreateImage` polling of all displays when the session isn't live (`:1106-1139`, called from `native_capture/activity.rs:128-134`). |
| Display liveness | `lib.rs:1153-1170` (`CGGetActiveDisplayList`) | Feeds the `DisplayUnavailable` suspension (ADR 0021; `native_capture/segments.rs:1831, 3460`). |
| Exact frame preview | `apps/desktop/src-tauri/src/app_infra/frame_preview.rs:1521-1656` | `AVAssetImageGenerator` with zero tolerance when an index entry exists. |
| Scrub previews | `frame_preview.rs:1698-1851` | Batched AVAssetImageGenerator, 1 s buckets, 360 px JPEG q72 (`:41-45`). Non-macOS stubs return errors (`:1888-1907`). **This is the seam.** |
| Live privacy exclusion | `capture-screen/src/lib.rs:2707-2792` + `native_capture/privacy.rs:589-622` | SCK `ContentFilter::with_display_excluding_apps…` keyed by **bundle id**, re-applied by a 1 s refresh. App-level only (ADR 0006: window-level filtering cost ~3× the power). |
| Mnema's own windows | **Not excluded at capture.** `day.mnema` is just an opt-in privacy candidate (`native_capture.rs:259-271`). Self-capture is removed **after the fact** in User Context (`crates/app-infra/src/user_context/capture_source.rs:335-346`). The system-audio tap is the only place that always excludes Mnema. |
| Audio helpers in the same crate | `capture-writers/src/lib.rs:1535` (`/usr/bin/afconvert`), `:26` (`trim_audio_file_to_m4a`, AVAudioFile) | Covered in §4.5. Note: **`SUPPORTS.md:22` is stale.** FFmpeg isn't used anywhere (`capture-writers/src/lib.rs:20-24, 1742`). |

**Linux compile break (confirmed by the compile check in §2):** `capture-screen/src/lib.rs:20-21` gates `mod equivalence;` behind `cfg(target_os = "macos")`, but `lib.rs:61-65` re-exports from it unconditionally, and `app-infra` calls it cross-platform. `equivalence.rs` has no Apple code. The fix is to delete one `#[cfg]` line. The fingerprint math (`lib.rs:547-1056`) is also pure byte code under a needless macOS gate.

#### (b) How it works and performs on macOS

- At the default operating point (one display, 0.5 fps, 720p, H.264 via VideoToolbox), encoding is essentially free. ADR 0006's measured cost is in the window server: ~649 mW with no filter, ~867 mW with app filtering, ~2245 mW with window filtering (`docs/adr/0006-…md`).
- Idle frames are never written, so a static screen costs about nothing.
- Display sleep, lock or disconnect is a transient suspension that recovers on its own (ADR 0021). The tail segment is kept.
- **Multi-monitor is already "primary display only"** on macOS. Linux parity therefore means one monitor, not all of them.

#### (c) Recommended Linux approach

##### Capture: xdg-desktop-portal ScreenCast + PipeWire (Wayland, the main target)

**Session setup:**
1. Call `ashpd::registry::register("day.mnema")` first, for non-Flatpak installs. Without it, a host app gets an empty or systemd-derived app id, and permissions stored under an empty id match too broadly. Registration must be the **first** portal call and needs a matching `.desktop` file ([Registry docs](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.host.portal.Registry.html), [GNOME blog](https://blogs.gnome.org/ignapk/category/portals), [KDE portal permissions](https://develop.kde.org/docs/administration/portal-permissions/)). Flatpak gets its app id for free.
2. Call `Screencast::create_session` and then `select_sources` with:
   - `types = MONITOR`, `multiple = false` (parity with macOS's single display);
   - `cursor_mode = Embedded` (macOS shows the cursor);
   - `persist_mode = 2` (until revoked), plus the stored `restore_token`.
3. Call `start` and **save the new restore token every time.** Tokens are **single-use**; a failed restore falls back to the dialog ([ScreenCast spec v6](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)).
   - Screenpipe's shipping Linux code uses `PersistMode::Application`, which only lasts while the process runs, so its users are re-prompted on every launch. Don't copy that.
4. Call `open_pipe_wire_remote` to get a file descriptor, then `pipewire::Context::connect_fd`. Connect a stream to the node, preferably targeted by `pipewire-serial` (spec v6).

**Stream format:**
- Offer `BGRx/BGRA/RGBx/RGBA` with `MAP_BUFFERS` (shared memory). **Don't start with DMA-BUF.** Mnema runs at ≤10 fps (default 0.5), so the GPU→CPU copy per frame doesn't matter. DMA-BUF only pays off at video frame rates, and it brings modifier-negotiation bugs (example: [OBS #13669](https://obs-versions.com/community/issues/13669), Intel Arc + KWin fallback).
- Request a low max framerate in the format pod. Mutter has let consumers clamp the rate since 2018 ([mutter commit](https://lists.gnome.org/archives/commits-list/2018-July/msg01696.html)). Below 1 fps, also drop frames ourselves (the same throttle `should_export_screen_frame` already does).

**Per frame:**
- Skip buffers with `chunk.size == 0` or the CORRUPTED flag. These are cursor-only or metadata-only updates and are the equivalent of SCK's non-`Complete` frames.
- Read the presentation time from `spa_meta_header` (monotonic ns). Use it for the frame index, keeping the "offsets from the sample's own timestamp" rule.
- Downscale on the CPU to the resolution preset with `fast_image_resize` 6.1. The portal delivers native size, unlike SCK's GPU scaling. Cheap at ≤10 fps.
- Feed the same bytes to the existing interleaved-bytes equivalence function and the activity fingerprint (both platform-neutral once ungated). Encode JPEG with `image` 0.25, which `capture-screen/Cargo.toml` already depends on.
- Hand the frame to the encoder.

**Threading:** run the PipeWire main loop on a dedicated thread (same shape as SCK's serial dispatch queue). Rotation, pause and resume swap the writer context exactly like `lib.rs:2586-2697`; the portal session stays up across segments.

**Liveness:**
- Watch both the portal session `Closed` signal **and** the PipeWire stream state. A GNOME bug can drop `Closed` when the user stops sharing from the top-bar menu ([x-d-p-gnome #64](https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/issues/64)).
- Map a user stop to a new "stopped by system" state. Map lock or display loss to the existing `DisplayUnavailable` suspension (ADR 0021).
- **Don't try to re-create or restore a session while the screen is locked.** One report says Mutter refuses while locked, and a failed attempt can burn the token, so the user gets re-prompted after unlock ([pinggy write-up, secondary source, unverified](https://pinggy.io/blog/remote_desktop_into_wayland_without_a_vps/)). Gate recovery on logind `LockedHint == false`.
- `screen_display_available()` on Linux becomes "unlocked + stream not errored".

**Crates:**

| Crate | Version / last update | Notes |
|---|---|---|
| `ashpd` | 0.13.13 (2026-07-17) | Actively maintained; includes Registry |
| `pipewire` (pipewire-rs) | 0.10.1 (2026-08-19) | freedesktop GitLab; links system `libpipewire-0.3` |
| `xcap` | 0.9.8 | Fine for X11. Its own README marks Wayland as not fully supported and recording as WIP ([xcap](https://github.com/nashaofu/xcap)) |
| `scap` | 0.0.8 (2025-08) | Effectively stale, no GitHub releases ([scap](https://github.com/helmerapp/scap)). **Reject.** |
| `gstreamer-rs` | 0.25.4 | Works, but see below |

**Why not GStreamer `pipewiresrc`:** plugin availability and codec elements vary by distro. The DMA-BUF path also broke after GStreamer 1.24 and needed a PipeWire fix ([PipeWire MR 1881](https://gitlab.freedesktop.org/pipewire/pipewire/-/merge_requests/1881)). Users report caps errors and still-high CPU even with `vah264enc` ([GStreamer discourse](https://discourse.gstreamer.org/t/h264-pipewire-issues/4764)). Mnema also needs Rust hooks on every frame (activity, equivalence, JPEG), so a pipeline would end in an `appsink` anyway.

##### Capture on X11 (fallback only)

`xcap` 0.9 `Monitor::capture_image()` on a timer at the configured rate, or `x11rb` 0.14 with XShm if `xcap` turns out too slow at 4K/10 fps. X11 has no consent dialog and no indicator. **It is a shrinking target:**
- GNOME 50 removed the X11 session ([linuxiac](https://linuxiac.com/gnome-50-ends-the-x11-era-after-decades/)).
- KDE Plasma 6.8 (2026-10-14) is Wayland-only ([Phoronix](https://www.phoronix.com/news/KDE-Plasma-68-Wayland-Exclusive)).

Ship it only because it's small (S). Idle detection there means comparing frames, because X11 has no damage-driven delivery unless we add XDamage.

##### Video writing: replace AVAssetWriter

**Recommendation: write H.264 with the `openh264` crate** (0.9.9, 2026-10-15; BSD-2; encoder **and** decoder; [openh264-rs](https://github.com/ralfbiedert/openh264-rs)). Mux with `mp4-atom` 0.17 (kixelated, updated 2026-10-30, reads and writes).

- **Cost:** the published single-thread benchmark is ~24 ms per 1080p frame encode (~8 ms with nasm) and ~6 ms per decode, on a desktop CPU, with the same repo caveat. At the 720p / 0.5 fps default that is well under 1% of a core. At the worst setting (10 fps, original 4K) expect roughly 0.3–1 core, which needs a benchmark.
- **Profile:** OpenH264 is Constrained Baseline. Expect somewhat larger files than High profile at the same quality. Acceptable for screen content at these rates.
- **Patents / licensing (H.264 is the one real cost):**
  - Via LA's AVC pool charges **no royalty for the first 100,000 units per year** per legal entity, then $0.20 per unit ([Via LA fee table](https://www.via-la.com/licensing-2/avc-h-264/avc-h-264-license-fees)). Mnema would likely owe nothing, but **the license must still be signed. Counsel question.**
  - Building OpenH264 from source does **not** inherit Cisco's patent grant. Only Cisco's binary, downloaded by the end user, does ([openh264-rs #43](https://github.com/ralfbiedert/openh264-rs/issues/43), [Debian copyright](https://metadata.ftp-master.debian.org/changelogs/main/o/openh264/stable_libopenh264-cisco7.copyright)).
  - The crate's `libloading` feature can load that binary at runtime. But Flathub **dropped** its Cisco-binary extension in 2025 over download and timeout problems, and the binary is served without HTTPS or strong checksums ([Flathub discourse](https://discourse.flathub.org/t/upcoming-freedesktop-23-08-runtime-release-will-drop-openh264-extension/9022), [itsfoss](https://itsfoss.com/news/freedesktop-sdk-drops-openh264/)). So the runtime-download route is worse UX for little gain.
  - **Preferred: static build plus a signed Via LA license.**
- **File container:** write ISO-BMFF but **keep the `.mov` file name.** `hidden_segment_workspace.rs:139` and the cleanup code derive the visible sibling as `.mov`. Mnema's own reader is the only consumer, because the webview never plays video (there is no `<video>` in `apps/desktop/src`). This avoids a path migration. Optionally write fragmented MP4 so a crash leaves a playable prefix; ADR 0040 discards partial files today anyway.
- **Frame index on finalize:** the writer knows every sample's timestamp, so build the index from the writer's own sample table instead of re-reading the file. Validation is "reopen with `mp4-atom` and find one video track with at least one sample".
- **No HEVC** (several patent pools, and hardware encode is stripped on Fedora, see below).
- **Hardware encode (VA-API/NVENC) is not needed** at Mnema's rates. Treat it as a later optimisation. Fedora/openSUSE mesa ship **without** VA-API H.264/HEVC on AMD ([Phoronix](https://www.phoronix.com/news/Fedora-Disable-Bad-VA-API)), so hardware H.264 can't be the baseline anyway.

##### Frame preview: replace AVAssetImageGenerator

- **Exact preview:** demux the sample table, seek to the sync sample at or before the indexed offset, decode forward with OpenH264 until PTS == target (miss → existing `video_exact_miss` log/fallback path), downscale, JPEG. Keep a keyframe interval of ≤30 frames (60 s at 0.5 fps) so the worst case decodes ~30 small frames.
- **Scrub previews:** one sequential decode pass per segment (≤300 one-second buckets, ADR 0005), sampled at the indexed offsets. That's cheaper than AVFoundation's per-time requests.
- **Where it goes:** implement the two cfg'd functions at `frame_preview.rs:1853-1907`. Everything else in that 3,892-line file is already platform-neutral (cache, DTOs, index lookup).

##### Live privacy exclusion and excluding Mnema's own windows

**The portal has no API to exclude apps or windows from a monitor stream** (spec: SelectSources only offers types/multiple/cursor/persist). Per desktop:

| Desktop | What exists | Verdict |
|---|---|---|
| **GNOME (Mutter)** | Nothing. The app can't even learn the focused app without a Shell extension. | Impossible at capture time; pause while in front once the Mnema extension is on (§10 decision 3) |
| **KDE Plasma ≥ 6.6** | KWin "Hide from ScreenCast" (`ExcludeFromCapture`, applied to popups too). Exposed in the titlebar/task-manager menu; window rules mentioned ([KWin MR 8442](https://invent.kde.org/plasma/kwin/-/merge_requests/8442), [ostechnix](https://ostechnix.com/exclude-certain-windows-from-screen-recording-in-plasma-6-6/)). Whether an app can set it (KWin script or window rule over D-Bus) and whether it applies to **monitor** casts is **unverified**. | Possibly full; needs a spike |
| **Hyprland ≥ 0.50** | `noscreenshare` window rule (draws black boxes), settable live with `hyprctl keyword` ([Hyprland wiki](https://wiki.hypr.land/0.51.0/Configuring/Window-Rules/)) | Full by class |
| **Sway / other wlroots** | `ext-image-copy-capture` exists, but has no exclusion ([sway releases](https://github.com/swaywm/sway/releases)). Focused app is knowable via foreign-toplevel. | Degraded (pause while focused) |
| **X11** | Know every window's geometry and stacking (`_NET_CLIENT_LIST_STACKING`) and black out excluded windows' visible rectangles before encode/JPEG. Window-level cost is our CPU, not a compositor. | Degraded-to-full (masking) |

- **Practical fallback everywhere: focus-gated exclusion.** Suspend screen frames while an excluded app is focused, using a **new** transient, screen-only suspension kind that behaves like `DisplayUnavailable` (not the existing `PrivacyFilter` kind, which means "a filter apply failed" and escalates to a manual restart, `native_capture/runtime.rs:36-39`; see ADR 0060). This is a **weaker promise** than ADR 0006: an unfocused excluded window visible in the background still gets recorded. It needs a new ADR and honest UI.
- **Self-exclusion is not a parity item.** macOS doesn't do it at capture either; it filters downstream (`capture_source.rs:335-346`). On Linux that filter depends on frame metadata naming the app, which GNOME Wayland can't provide. **Cheap fix:** tag frames as self-capture whenever any Mnema Tauri window has focus (the app knows that itself on every platform). On KDE 6.6+ and Hyprland, also hide Mnema's own windows at capture if the spike works.
- **Identity:** the privacy list is keyed by macOS bundle id. Linux needs Wayland `app_id` / X11 `WM_CLASS` / `.desktop` id. That's shared with §4.2.

#### (d) Expected Linux performance and the gap

- **Capture:** Mutter on DMA-BUF adds "almost no extra CPU" ([mutter MR 1086](https://gitlab.gnome.org/GNOME/mutter/merge_requests/1086), [Phoronix](https://www.phoronix.com/news/GNOME-Wayland-Screencast-DMA)). Shared memory adds one GPU readback per delivered frame: trivial at 0.5 fps, noticeable at 10 fps on 4K.
  - Unlike SCK, the compositor delivers on **damage** at up to the negotiated max rate. If Mutter's minimum negotiable rate is 1 fps, at 0.5 fps we receive and discard every other frame. Small cost, needs a check.
- **Encode:** OpenH264 on the CPU vs VideoToolbox on a hardware block. At the default it's negligible. At 10 fps original-4K it's a real CPU and power gap (~0.3–1 core), which matters on laptops.
- **Downscaling** moves from GPU (SCK) to CPU. A few ms per frame with SIMD.
- **Activity polling while not live:** there's no silent screenshot on Wayland. Rely on input idle plus frames from the live stream. `poll_screen_activity()` stays a no-op (it already is, `lib.rs:1141-1144`).
- **No published Screenpipe or OBS CPU numbers were found for this operating point.** Must be measured. Screenpipe's Linux capture (ashpd + pipewire 0.9, shared memory) is the closest prior art ([screenpipe repo](https://github.com/screenpipe/screenpipe), `crates/screenpipe-screen/src/monitor/linux_portal.rs`).

#### (e) Fallbacks

1. **Codec fallback: AV1 with `rav1e` 0.8 (encode) + `dav1d` 0.11 (decode)** if counsel won't sign the AVC license. Royalty-free, and Fedora keeps AV1 VA-API. Costs more CPU: rav1e speed 10 does ~8–15 fps at 4K on desktop CPUs ([OpenBenchmarking](https://openbenchmarking.org/result/2312317-PTS-RAV1ESVT30&export=html)), fine at 720p/0.5 fps, heavy at 4K/10 fps. Its last release was 2025-06, and SVT-AV1 has no maintained Rust binding (`svt-av1-sys` 0.1.1 dates from 2020).
2. **Media fallback: `gstreamer-rs` 0.25** with system plugins (`openh264enc`/`vah264enc`, `decodebin`) if the self-contained stack hits a wall. Only with a runtime plugin check and capability flags.
3. **Capture fallback:** X11 via `xcap`. There is no Wayland fallback: compositor-private protocols (`ext-image-copy-capture`, wlr-screencopy) only work on wlroots/COSMIC/Hyprland and are a second codepath. Skip for v1.
4. **Codec licensing fallback (option 3):** Cisco binary via `openh264`'s `libloading` feature. Worse UX (download, no HTTPS).

#### (f) Risks

1. **Repeated consent prompts.** Restore tokens are single-use. Lock-screen attempts may burn them. Backend support varies (KDE fixed a token bug only in 6.6.6, [KDE bug 517454](https://bugs.kde.org/show_bug.cgi?id=517454); xdg-desktop-portal-wlr token support unverified). Host apps need the Registry call. If any of these fail, an always-on recorder nags after every login or unlock. **This is the #1 product risk.**
2. **Per-app privacy can't match macOS on GNOME, the largest Linux desktop.** It needs an ADR, honest UI, and probably focus-gating. Over-promising is a privacy bug.
3. **H.264 patent posture** for a paid app that ships its own encoder: Via LA signature or a switch to AV1. Also the CPU cost of software encode at high settings.
4. **Always-visible indicator.** GNOME shows a screen-sharing indicator, with a stop control, whenever a portal cast is active ([gnome-shell commit](https://lists.gnome.org/archives/commits-list/2022-August/msg04201.html), [Mozilla 1668358](https://bugzilla.mozilla.org/show_bug.cgi?id=1668358)). Users will see it permanently and can stop capture from outside Mnema.
5. **Compositor quirks.** NVIDIA proprietary + shared-memory paths, fullscreen unredirect freezing captures ([NVIDIA forum](https://forums.developer.nvidia.com/t/choopy-desktop-capture-using-pipewire-and-obs-studio-30-0-2-on-wayland/278807)), and Mutter crashing if PipeWire restarts mid-cast ([mutter #1251](https://gitlab.gnome.org/GNOME/mutter/-/issues/1251)).
6. **The `.mov` file name holding ISO-BMFF** is harmless internally, but any future "export video" feature should rename it to `.mp4`.

#### (g) Effort

| Piece | Effort |
|---|---|
| Portal + PipeWire capture with token persistence, liveness, rotation/pause wiring | **M** (2–3 wk) |
| OpenH264 + mp4-atom writer, validation, frame-index finalize | **M** (1.5–2 wk) |
| Exact + scrub preview decode | **S–M** (1 wk) |
| X11 backend | **S** (3–4 d) |
| Privacy capability model + focus-gating + ADR + per-desktop spikes (KWin, Hyprland) | **M** (1.5–2 wk) |
| Untangling `cfg(macos)` gates in `segments.rs` (108 gates), `runtime.rs` (37), `native_capture.rs` (68) | **M**, shared with §4.9 |
| **Subsystem total** | **L** (~7–10 engineer-weeks) |

#### (h) Parity matrix

| Capability | macOS | GNOME Wayland | KDE Wayland | wlroots (Sway/Hyprland) | X11 |
|---|---|---|---|---|---|
| Screen capture | full — SCK, silent after TCC grant | degraded — portal dialog once; restore token; permanent indicator; lock may re-prompt | degraded — portal dialog once; restore token (≥6.6.6 fixes) | degraded — xdpw/xdph chooser; token support varies | full — no consent at all |
| Multi-monitor | degraded — primary display only (`lib.rs:3314`) | full (parity) — user picks one monitor in dialog | full (parity) — same | full (parity) — chooser picks output | full (parity) — pick a `xcap` monitor |
| Self-exclusion (Mnema UI) | degraded — not at capture; downstream filter by bundle id | degraded — downstream only, via "Mnema window focused" tagging | degraded→full — downstream; KWin ExcludeFromCapture if scriptable (6.6+, unverified) | Hyprland full (`noscreenshare` on own class); Sway degraded (downstream) | full — mask own window rects before encode |
| Per-app exclusion | full — SCK app filter, live | degraded — impossible to hide at capture (no API); pause while in front with the Mnema extension, unavailable without it | degraded/maybe full — 6.6+ per-window hide if scriptable; else focus-gated pause | Hyprland full (`noscreenshare` rule); Sway degraded (focus-gated via foreign-toplevel) | degraded — rectangle masking from window stacking (occlusion edge cases) |
| Video writing | full — AVAssetWriter H.264 `.mov` (hardware) | full — OpenH264 CPU, ISO-BMFF named `.mov` | full — same | full — same | full — same |
| Frame preview (exact + scrub) | full — AVAssetImageGenerator | full — demux + OpenH264 decode + `image` JPEG | full — same | full — same | full — same |

#### (i) Files to move or split (concrete names)

`crates/capture-screen/src/lib.rs` is 5,376 lines, against the 800-line rule. **Keep the existing free-function API as the seam** (`start_capture_session_with_options`, `rotate_screen_capture_session`, `pause_screen_outputs_for_inactivity`, `resume_screen_outputs`, `stop_screen_capture_session`, `update_active_privacy_filter`, `screen_display_available`, `support_for_current_platform`). Dispatch by module per OS. Inside Linux, a two-variant enum (Portal | X11) is enough; no trait object is needed.

**`crates/capture-screen/src/`**
- `lib.rs` — shared types plus re-exports only.
- `frame_index.rs` — `lib.rs:215-348`, `1505-1512`; neutral.
- `activity.rs` — fingerprint math `lib.rs:547-1056`; neutral (ungate it).
- `equivalence.rs` — fix the gate at `lib.rs:20`.
- `frame_export.rs` — throttle + artifact naming; neutral parts of `lib.rs:1599-1843`.
- `platform/macos/{sck_session.rs, avfoundation_session.rs, content_filter.rs, validate.rs, stream_delegates.rs}`.
- `platform/linux/{portal_session.rs (ashpd + token store + Registry), pipewire_stream.rs, x11_session.rs, privacy.rs (capability + focus gate + X11 masking), liveness.rs (logind LockedHint)}`.

**`crates/capture-writers/src/`**
- `macos/{video_writer.rs, audio_writer.rs, audio_decode.rs, trim.rs}` (from `lib.rs:26-1535`).
- `video_writer_openh264.rs` — Linux, shared by any future non-Apple target.
- `video_reader.rs` — mp4-atom demux + OpenH264 decode, used by previews + index rebuild.

**`apps/desktop/src-tauri/src/app_infra/frame_preview.rs`** (3,892 lines)
- Split into `frame_preview/{cache.rs, dto.rs, scrub.rs, video_extract_macos.rs (lines 1386-1887), video_extract_linux.rs}`.

**Other files**
- `crates/capture-screen` `ScreenCaptureSupport` (`lib.rs:395-399`): add `privacy_exclusion: PrivacyExclusionSupport { Full, FocusGated, Masked, Unavailable }` and `consent: ConsentKind { Tcc, PortalDialog, None }`, so the frontend renders from data instead of `isMac` checks.
- `apps/desktop/src-tauri/src/native_capture/privacy.rs`: consume the capability; the focus-gated path enters a new transient, screen-only `CaptureSuspensionKind` variant (ADR 0060), not `PrivacyFilter`.
- `Cargo.toml` of `capture-screen`: add a `[target.'cfg(target_os = "linux")'.dependencies]` block with `ashpd`, `pipewire`, `xcap`, `fast_image_resize`; `capture-writers` gets `openh264`, `mp4-atom`.

#### (j) What must degrade and how the UI should show it

- **Permission:** replace the TCC row with "Screen sharing: allowed (remembered) / will ask / was denied or stopped". The state is inferred from whether a restore token exists and the last outcome (same tri-state idea as system audio). Onboarding should say: "Your desktop will show its screen-sharing icon while Mnema records — this is expected."
- **Stopped from the system indicator:** show a status-bar and dashboard state "Screen recording stopped from system menu — Resume". It must **not** look like an error or silently retry; a silent retry would just re-prompt.
- **Lock/unlock:** reuse "Paused — display unavailable" (ADR 0021). If a re-prompt happens anyway, notify once rather than looping.
- **Settings → Privacy, per-capability copy:**
  - Full: same as macOS.
  - FocusGated: "Excluded apps pause screen recording while they're in front. If an excluded app is visible behind another window, it is recorded."
  - Masked (X11): "Excluded windows are blacked out."
  - Unavailable (GNOME): "This desktop doesn't let apps hide other apps from recordings. Use Pause, or Delete Recent Capture."
- **Status bar:** hide "Exclude Current App" (`status_bar.rs:646` area) where the focused app is unknown (GNOME Wayland).
- **Multi-monitor:** say "Records the monitor you picked" and add a "Change monitor…" action that re-opens the portal dialog (one deliberate re-prompt).

#### (k) Benchmark on real hardware

1. **Prompt durability:** across reboot, logout, lock/unlock ×20, suspend/resume, monitor hot-plug, and portal-service restart, on GNOME 49/50, KDE 6.6/6.8, Sway + xdpw, Hyprland + xdph. Count re-prompts. Target: zero after the first grant.
2. **Overall cost:** whole-system CPU % and package power (RAPL / `powerstat`) at 0.5 / 2 / 10 fps × 720p / 1080p / original. Compare shared memory vs DMA-BUF and Intel vs AMD vs NVIDIA proprietary. Include compositor CPU (`gnome-shell`, `kwin_wayland`), not just Mnema.
3. **Encode cost:** OpenH264 time per frame and file size per 5-min segment vs the macOS bitrate presets. Also the rav1e comparison for fallback 1.
4. **Frame rate floor:** does Mutter/KWin honour a max framerate below 1 fps, or must we drop frames ourselves? Measure delivered frames on a static screen (damage-only delivery).
5. **Preview latency:** exact preview p50/p95 and scrub batch time for a 5-min segment, against the existing 5 s / 2 s timeouts (`frame_preview.rs:33,40`).
6. **Fullscreen and games:** fullscreen-unredirect frozen captures (GNOME + NVIDIA).
7. **KDE 6.6+ ExcludeFromCapture:** does it hide a window from a **monitor** portal cast, and can it be set programmatically? Same question for Hyprland `noscreenshare` with xdph monitor casts.

#### (l) Open questions

- **Counsel:** sign the Via LA AVC license (free tier under 100k units/yr), or go AV1 on Linux? This decides the codec.
- **ADR needed:** is focus-gated exclusion an acceptable Linux privacy promise, or should per-app exclusion be "unavailable" outside KDE/Hyprland/X11? ADR 0006 assumes app-level exclusion that is guaranteed at capture time.
- **Restore token storage:** plain app settings or the vault? It's a capability that grants silent screen capture to whoever replays it under Mnema's app id. Recommend the app data dir with mode 0600; it's single-use anyway.
- **Multiple monitors:** stay on "one picked monitor", or record N streams as N segment files? The latter is a schema/timeline change.
- **Lock behaviour:** does an existing GNOME/KDE cast survive a screen lock or get torn down? It changes whether lock maps to a pause or to a session re-create.
- **Packaging:** Flatpak vs .deb/AppImage changes the app-id story (Registry), and `libpipewire` is a runtime dependency. See §4.12.
- **Self-capture tagging:** add "Mnema window focused" stamping cross-platform, which would also harden macOS? Small and useful everywhere.

#### Sources

- ScreenCast portal spec: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- Host Registry: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.host.portal.Registry.html ; https://blogs.gnome.org/ignapk/category/portals ; https://develop.kde.org/docs/administration/portal-permissions/
- Restore-token misuse report: https://screenconnect.product.connectwise.com/communities/1/topics/5169-bug-linux-client-repeatedly-opens-gnome-remote-desktop-permission-dialogs-and-ignores-restore-token
- Lock-screen restore claim (secondary): https://pinggy.io/blog/remote_desktop_into_wayland_without_a_vps/
- KDE token bug: https://bugs.kde.org/show_bug.cgi?id=517454
- GNOME Closed-signal bug: https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/issues/64
- KWin exclude-from-capture: https://invent.kde.org/plasma/kwin/-/merge_requests/8442 ; https://ostechnix.com/exclude-certain-windows-from-screen-recording-in-plasma-6-6/
- Hyprland noscreenshare: https://wiki.hypr.land/0.51.0/Configuring/Window-Rules/
- Sway image-capture protocols: https://github.com/swaywm/sway/releases ; https://wayland.app/protocols/ext-image-capture-source-v1
- Mutter framerate negotiation: https://lists.gnome.org/archives/commits-list/2018-July/msg01696.html ; DMA-BUF: https://gitlab.gnome.org/GNOME/mutter/merge_requests/1086 ; https://www.phoronix.com/news/GNOME-Wayland-Screencast-DMA
- GNOME indicator: https://lists.gnome.org/archives/commits-list/2022-August/msg04201.html ; https://bugzilla.mozilla.org/show_bug.cgi?id=1668358
- PipeWire GStreamer DMA-BUF: https://gitlab.freedesktop.org/pipewire/pipewire/-/merge_requests/1881 ; https://discourse.gstreamer.org/t/h264-pipewire-issues/4764
- OBS DMA-BUF fallback issue: https://obs-versions.com/community/issues/13669
- xcap: https://github.com/nashaofu/xcap ; scap: https://github.com/helmerapp/scap ; screenpipe: https://github.com/screenpipe/screenpipe
- openh264-rs: https://github.com/ralfbiedert/openh264-rs ; issue #43: https://github.com/ralfbiedert/openh264-rs/issues/43 ; Debian copyright: https://metadata.ftp-master.debian.org/changelogs/main/o/openh264/stable_libopenh264-cisco7.copyright
- Via LA AVC fees: https://www.via-la.com/licensing-2/avc-h-264/avc-h-264-license-fees
- Flathub openh264 removal: https://discourse.flathub.org/t/upcoming-freedesktop-23-08-runtime-release-will-drop-openh264-extension/9022 ; https://itsfoss.com/news/freedesktop-sdk-drops-openh264/
- Fedora VA-API codec removal: https://www.phoronix.com/news/Fedora-Disable-Bad-VA-API
- rav1e / SVT-AV1 benchmarks: https://openbenchmarking.org/result/2312317-PTS-RAV1ESVT30&export=html
- X11 session removal: https://linuxiac.com/gnome-50-ends-the-x11-era-after-decades/ ; https://www.phoronix.com/news/KDE-Plasma-68-Wayland-Exclusive
- NVIDIA fullscreen capture: https://forums.developer.nvidia.com/t/choopy-desktop-capture-using-pipewire-and-obs-studio-30-0-2-on-wayland/278807 ; mutter #1251: https://gitlab.gnome.org/GNOME/mutter/-/issues/1251
- Crate versions: crates.io API, queried 2026-11-01 (ashpd 0.13.13, pipewire 0.10.1, xcap 0.9.8, scap 0.0.8, gstreamer 0.25.4, ffmpeg-next 9.0.0, openh264 0.9.9, mp4-atom 0.17.0, rav1e 0.8.1, dav1d 0.11.1, fast_image_resize 6.1.0, x11rb 0.14.0).

### 4.2 Active app and window tracking


**Short answer:** Linux has no single API for "which app is in front". Build one small Rust trait with four backends, picked at startup: a KWin script on KDE, a Mnema-shipped GNOME Shell extension on GNOME, the wlroots toplevel protocol (plus compositor IPC for the PID) on Sway/Hyprland/niri/labwc, and `x11rb` on X11. Keep the existing `bundle_id` string fields and store a **desktop-file id** in them. Expect GNOME to be the expensive part: users must install the extension and log out and back in, and every GNOME release needs a version bump.

---

#### (a) Current macOS implementation

| Piece | Where | What it does |
|---|---|---|
| Shared types | `crates/capture-metadata/src/lib.rs:84-97` (`FrameMetadataSnapshot`: `app_bundle_id`, `app_name`, `window_title`, `window_id`), `:129-148` (`MetadataContext`, `WindowContext` incl. `owner_pid`), `:225-232` (`NativeActiveWindowSnapshot`: bundle_id, app_name, pid, window_id, window_title) | Platform-neutral data. The crate itself has no OS deps (`crates/capture-metadata/Cargo.toml`). |
| Window picking | `crates/capture-metadata/src/lib.rs:234-255` (`RawWindowInfo`, `select_frontmost_pid_window`) | First on-screen, layer-0, non-zero-size window owned by the frontmost PID. |
| Lock screen | `crates/capture-metadata/src/lib.rs:99-116` (`LOCK_SCREEN_BUNDLE_IDS = ["com.apple.loginwindow"]`) | Frames whose frontmost app is the lock screen count as time away. |
| Frontmost app | `apps/desktop/src-tauri/src/native_capture_metadata.rs:455-469` | `NSWorkspace.runningApplications` → the one with `isActive`, gives pid, bundle id, localized name. |
| Window title | `native_capture_metadata.rs:471-508` | `CGWindowListCopyWindowInfo(OnScreenOnly)` → `kCGWindowName` for the frontmost PID's window. |
| Combined snapshot | `native_capture_metadata.rs:426-441` (`collect_native_active_window_snapshot`), `:347-424` (`collect_active_window_metadata`; non-macOS branch returns empty at `:412-423`) | |
| Focus notifications | `native_capture_metadata.rs:218-268` (`start_metadata_notifier`, 7 NSWorkspace notifications); stubbed out on other OSes at `:270-271`; started from `lib.rs:934` | |
| Refresh pipeline | `apps/desktop/src-tauri/src/native_capture/privacy.rs:106-114` (`PrivacyRefreshReason`), `:380-438` (request; skipped unless recording), `:440-526` (collector thread, cached pre-pass then live pass); **the whole file is `#[cfg(target_os = "macos")]`**, with a `Mutex<()>` placeholder at `:103-104` | |
| Fallback poll | `native_capture/segments.rs:47` (`PRIVACY_FILTER_POLL_INTERVAL = 1s`), `:4667-4673` | |
| Frame stamping | `native_capture_metadata.rs:17-66` (32-entry history ring, `snapshot_in_effect_at`) | Each frame gets the app that was in front at its capture time. |
| Tray "Exclude Current App" | `apps/desktop/src-tauri/src/status_bar.rs:640-692`; non-macOS shows "available only on macOS" (`:684-691`); menu id `tray_exclude_current_app` (`:19`) | Takes a snapshot, refuses `day.mnema`, confirms, then calls `privacy_redaction_sources::add_or_enable_privacy_excluded_app_from_app_handle`. |
| App list + icons | `apps/desktop/src-tauri/src/native_capture.rs:191-198` (`PrivacyAppCandidate`), `:258-288` (`list_privacy_app_candidates`), `:290-366` (`resolve_app_icons`, returns no icons on non-macOS at `:353-364`), `:474-560` (scan `/Applications`, `/System/Applications`), `:644-800` (render `.icns` → PNG into the AppCache icon dir, added to the asset scope at runtime) | |
| Recommended exclusions | `apps/desktop/src-tauri/src/sensitive_capture_recommendations.rs:43-64` (catalog: `com.apple.Passwords`, `com.apple.keychainaccess`, `com.1password.1password`, `com.bitwarden.desktop`), `:141-199` (joins the catalog with app candidates + the browser list from `capture_metadata::KNOWN_BROWSER_APPS`) | |
| Lock-screen check for the reminder | `native_capture/not_recording_reminder.rs:196` → `native_capture_metadata.rs:443-453` | |
| Persistence | `app_bundle_id` column is read across `crates/app-infra/src/brokered_access.rs` (e.g. `:2017`, `:2821`) and `search/projection.rs:134-172` | It is an opaque string; nothing parses it as reverse-DNS. |

SUPPORTS.md says all of this is macOS-only: rows "Live app privacy exclusion" (`SUPPORTS.md:36`), "Active app/window metadata" (`:37`), "Recommended app exclusions" (`:39`), and "Status bar / tray … Exclude Current App" (`:55`).

#### (b) How it works and performs on macOS

- It mixes **events and a poll**. Seven NSWorkspace notifications (activate, deactivate, space change, launch, terminate, hide, unhide) request a refresh (`native_capture_metadata.rs:223-259`). A **1 s fallback poll** (`segments.rs:47`, `:4667`) catches title changes, which macOS sends no notification for.
- Collection only runs **while recording** (`privacy.rs:391-402`). It runs on a one-shot background thread so it never holds the capture lock (`privacy.rs:463-525`).
- A focus change makes **two publishes**. A cheap "cached URL" pass lands within milliseconds, then a live pass upgrades the browser URL (`privacy.rs:480-486`), which can take about 1.4 s for a Gecko browser.
- Cost per tick is one `runningApplications` walk plus one on-screen window-list copy. Getting window titles from that copy needs the Screen Recording permission.

#### (c) Recommended Linux approach

**One trait, picked at runtime, event-driven where the desktop allows it.** It reuses `NativeActiveWindowSnapshot` and feeds the same `request_privacy_filter_refresh` path. The backend choice reads `XDG_SESSION_TYPE`, `XDG_CURRENT_DESKTOP` and `WAYLAND_DISPLAY`:

1. **KDE Plasma 6 (Wayland and X11): KWin script.** Load a bundled JS script through `org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript` and `run`. The script listens to `workspace.windowActivated` and `captionChanged`, then calls `callDBus` back to a Mnema D-Bus object with `caption`, `resourceClass`, `desktopFileName` and `pid`. All of these properties exist on `KWin::Window` (https://develop.kde.org/docs/plasma/kwin/api/). ActivityWatch's `awatcher` uses this exact pattern, and the user sets up nothing (https://github.com/2e3s/awatcher, loader code https://raw.githubusercontent.com/2e3s/awatcher/main/watchers/src/watchers/kwin_window.rs; awatcher only forwards caption and class, not pid). Crate: `zbus` (already in the lockfile at 5.14.0, `Cargo.lock`). Don't use `org_kde_plasma_window_management`: KWin only grants it to apps that request it in their desktop file, and a third-party developer reported getting no events even then (https://mail.kde.org/pipermail/kwin/2024-January/005305.html).
2. **GNOME (Wayland): Mnema's own small Shell extension.** It should export a D-Bus object with a `FocusChanged(app_id, title, pid, window_id)` signal and a `GetFocused()` method. App id comes from `Shell.WindowTracker.get_window_app(win).get_id()` (the `.desktop` id, already matched the way GNOME does it), plus `Meta.Window.get_pid()`, `get_title()`, and `notify::focus-window` / `notify::title`. GNOME's own tracker uses those methods (https://github.com/GNOME/gnome-shell/blob/main/src/shell-window-tracker.c). Exact GJS signatures were not checked against gjs-docs, so treat them as uncertain.
   - Why our own extension: `org.gnome.Shell.Eval` and `Introspect` have been locked down since GNOME 41 unless unsafe mode is on (https://github.com/jeffshee/hidamari/issues/40, https://discourse.gnome.org/t/terminal-command-to-open-app-drawer-and-switch-between-applications/8226).
   - Why not existing extensions: "Focused Window D-Bus" exposes only title and class, has no signal, and supports GNOME 43–51 (https://extensions.gnome.org/extension/5592/focused-window-d-bus/). "Window Calls" exposes pid and class but no focus signal, so it needs polling (https://github.com/ickyicky/window-calls). Shipping our own avoids depending on a third party for a privacy feature.
3. **wlroots-family (Sway, Hyprland, niri, labwc, Wayfire): `zwlr_foreign_toplevel_management_v1`.** It gives title, app_id and an `activated` state, but **no PID** (https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1; supported by Sway 1.11, Hyprland 0.52, niri 26.04, labwc, Wayfire; not by KWin, Mutter or COSMIC). Crates `wayland-client` and `wayland-protocols-wlr` are already in `Cargo.lock`. For the PID, add compositor IPC where it exists:
   - Sway: `GET_TREE` nodes have `pid` and `app_id`, and `WINDOW` events include `focus` and `title` (https://man.archlinux.org/man/sway-ipc.7.en).
   - niri: `Window.pid` is available (https://docs.rs/niri-ipc/latest/niri_ipc/struct.Window.html).
   - Hyprland: the `activewindow` event on socket2 (https://wiki.hypr.land/IPC/). I believe `hyprctl activewindow -j` returns a `pid`, but I could not confirm it.
4. **COSMIC:** `ext-foreign-toplevel-list-v1` alone is **not enough**. It lists windows with title, app_id and an identifier, but has no "activated" state (https://wayland.app/protocols/ext-foreign-toplevel-list-v1). We would need COSMIC's own `cosmic-toplevel-info` state extension, which is what awatcher uses (https://github.com/2e3s/awatcher). **Decided: build it in v1** (§10 decision 12), because Pop!_OS 24.04 ships COSMIC by default.
5. **X11 session: `x11rb`** (already in `Cargo.lock` at 0.13.2). Read root `_NET_ACTIVE_WINDOW`, then `_NET_WM_NAME` (UTF-8), `WM_CLASS`, `_NET_WM_PID`, subscribing to `PropertyNotify` on the root and the active window so it is event-driven (EWMH: https://specifications.freedesktop.org/wm/latest/). `_NET_WM_PID` is set by the client and may be missing. The XRes extension's client-id query is the more reliable PID source and `x11rb` supports it. **Never** use the X11 path inside a Wayland session: it only sees XWayland windows.

**App identity (the macOS bundle-id stand-in):** store the **desktop-file id** without `.desktop` (e.g. `org.mozilla.firefox`, `firefox`, `com.bitwarden.desktop`). xdg-shell recommends that app_id match the desktop file's basename (https://wayland.app/protocols/xdg-shell). Normalize like this:
- GNOME and KDE already give the desktop id (`WindowTracker` / `desktopFileName`).
- On wlroots and X11, resolve `app_id` / `WM_CLASS` against installed `.desktop` files by filename first, then the `StartupWMClass` key, falling back to the raw app_id. Crate: `freedesktop-desktop-entry` (pop-os, https://github.com/pop-os/freedesktop-desktop-entry).
- Flatpak apps report their Flatpak id as app_id, which is already a good reverse-DNS key.

**Icons:** take the desktop entry's `Icon=` value, look it up in the icon theme with the `freedesktop-icons` crate (https://docs.rs/freedesktop-icons), and copy the SVG or PNG into the existing AppCache icon dir, which is already added to the asset scope (`native_capture.rs:644-666`). WebKitGTK renders SVG, so no rasterizing is needed. App candidates come from scanning `.desktop` files instead of `/Applications` (`native_capture.rs:474-546`).

**Recommended-exclusions catalog:** add a `linux_ids: &[&str]` alias list per entry in `SENSITIVE_APP_CATALOG` (`sensitive_capture_recommendations.rs:43-64`). These ids are from memory and need checking on real installs:
- Bitwarden: `com.bitwarden.desktop` (Flathub), `bitwarden` (deb).
- 1Password: `1password`.
- KeePassXC: `org.keepassxc.KeePassXC`.
- Seahorse: `org.gnome.seahorse.Application`.
- KWallet Manager: `org.kde.kwalletmanager`.

Drop the Apple-only entries on Linux.

#### (d) Expected Linux behaviour and the gap

- **KDE, Sway/niri, X11:** close to macOS, and better in one way: title changes arrive as events, so there is less 1 s polling lag.
- **GNOME:** **nothing works until the user installs our extension, enables it, and logs out and back in.** A newly installed extension can't load into a running Wayland GNOME Shell without a session restart (https://github.com/ddterm/gnome-shell-extension-ddterm/blob/master/docs/INSTALL.md). That is the biggest gap.
- **Hyprland/labwc/Wayfire without IPC:** app and title work, the PID is missing.
- **Screen exclusion (detailed in §4.1, but it drives the "exclude current app" cell):** the portal screencast can't hide one app's windows the way ScreenCaptureKit can. On Linux, the realistic way to honor an exclusion is to **drop or blank frames while an excluded app is focused**. That misses an excluded app that is visible but not focused, which is weaker than macOS.

#### (e) Fallbacks

1. GNOME with no extension: if "Window Calls" is installed, poll it at 1 Hz. Otherwise run with **metadata off**, show a clear banner, and record frames unlabeled.
2. KDE where loading the script fails: try XWayland-only X11 reads, which only see some apps. Show the result as degraded.
3. Any compositor without a protocol: no metadata, and exclusion falls back to the static-list "pause when this app is focused" rule, which needs focus data and so can't run. Say this plainly in the UI.
4. Lock screen: use logind `LockedHint` on `org.freedesktop.login1.Session` over `zbus`. Every desktop has it, and on Wayland the lock screen is not a normal window, so a toplevel check can't see it.

#### (f) Risks

1. **GNOME extension upkeep.** It must be ported every six months (`shell-version` in `metadata.json`; ES modules since GNOME 45, https://gjs.guide/extensions/overview/anatomy.html). extensions.gnome.org review adds delay and rejects some version ranges (https://extensions.gnome.org/review/75224). Plus the logout-to-activate step. Many users won't do it.
2. **Privacy promise is weaker.** Exclusion becomes "while focused" instead of window-level hiding. Fast alt-tab or a background excluded window can still leak into frames. That is a product and legal wording risk, not just engineering.
3. **App-id chaos.** Electron apps (1Password, Bitwarden, Slack) report app_id and WM_CLASS inconsistently between X11 and Wayland and between deb and Flatpak, so exclusions and app grouping can miss or split one app. This needs an alias-matching layer and real-device testing.
4. **Packaging Mnema as a Flatpak breaks most of this.** It needs `--talk-name=org.kde.KWin` and `--talk-name=org.gnome.Shell`, can't drop the extension into `~/.local/share/gnome-shell/extensions`, can't map host PIDs, and wlroots compositors may hide privileged protocols from sandboxed clients via security-context-v1 (https://wayland.app/protocols/security-context-v1). awatcher states its Flatpak/AppImage builds lack Wayland tracking (https://github.com/2e3s/awatcher). **Ship deb/rpm (and a tarball) first.**
5. On KDE, clicking the tray menu might move focus to plasmashell. Read the last non-Mnema, non-shell entry from the history ring (`native_capture_metadata.rs:30-66`) instead of a fresh snapshot.

#### (g) Effort: **L** (roughly 3–4 weeks)

- KWin script backend: S.
- X11: S.
- wlroots protocol plus Sway/niri IPC: M.
- Desktop-id resolver, icons and candidate list: M.
- GNOME extension, its install UX and per-release port: M now plus ongoing work.
- Making `privacy.rs` and the notifier non-macOS: S–M.
- Real-hardware matrix: M.

#### (h) Parity

| Capability | macOS | GNOME Wayland | KDE Wayland (Plasma 6) | wlroots (Sway/Hypr/niri/labwc) | X11 |
|---|---|---|---|---|---|
| Frontmost app | full: NSWorkspace `isActive` | degraded: only with Mnema's extension plus a relogin | full: KWin script `windowActivated` | full: wlr toplevel `activated` (COSMIC needs its own protocol) | full: `_NET_ACTIVE_WINDOW` |
| Window title | full: needs Screen Recording | degraded: extension only | full: `caption` | full: `title` event | full: `_NET_WM_NAME` |
| PID | full | degraded: extension `get_pid()` | full: `Window.pid` | degraded: Sway/niri IPC yes, Hyprland likely, labwc/Wayfire no | degraded: `_NET_WM_PID` is optional, XRes is better |
| App icon | full: `.icns` → PNG | full: desktop entry + icon theme | full | full when app_id resolves to a desktop file | full (same resolver via `WM_CLASS`) |
| Exclude current app (tray) | full: SCK window hiding | degraded: needs the extension and the AppIndicator extension for the tray; enforcement is "drop frames while focused" | degraded: same "while focused" enforcement | degraded: same, and many have no tray host | degraded: same; X11 geometry would allow masking but is not recommended for v1 |

#### (i) Files to move or split behind a platform trait

- **`apps/desktop/src-tauri/src/native_capture_metadata.rs`** (838 lines, already over the 800-line rule):
  - Keep the ring, the refresh logic and the browser cache.
  - Move `start_metadata_notifier` (`:218-271`), `collect_native_active_window_snapshot` (`:426-441`), `frontmost_app_is_lock_screen` (`:443-453`), `frontmost_running_app_snapshot` and the CG helpers (`:455-584`) into `native_capture/active_window/{mod.rs, macos.rs, linux/{mod.rs, kwin.rs, gnome.rs, wlr.rs, sway_ipc.rs, niri_ipc.rs, x11.rs}}`.
  - `mod.rs` holds `fn snapshot() -> NativeActiveWindowSnapshot`, `fn start_notifier(app)` and `fn screen_is_locked()`.
- **`native_capture/privacy.rs`:** remove the blanket `#[cfg(target_os = "macos")]`. `PrivacyRefreshReason` and the collector are platform-neutral, and only the `capture_screen::PrivacyContentFilter` application is macOS-specific.
- **`native_capture.rs:258-800`** (app candidates and icons): move to `app_catalog/{mod.rs, macos.rs, linux.rs}`, with `linux.rs` doing the desktop-entry scan and icon-theme lookup.
- **`status_bar.rs:640-692`:** drop the cfg split and call the trait. Self-exclusion must check Mnema's own desktop id (`day.mnema`) on Linux too.
- **`sensitive_capture_recommendations.rs:43-64`:** give the catalog per-OS id lists.
- **`crates/capture-metadata/src/lib.rs:99-116`:** the lock-screen ids become per-OS, or are replaced by `screen_is_locked()`.
- **Don't change:** DB columns and serde field names (`app_bundle_id`, `bundleId`). They are opaque strings; renaming them means a migration for no gain. Document the meaning instead. The Windows research proposes a richer identity record (`docs/windows/permissions-privacy-metadata-research.md:133-160`), which is worth reconciling later but not needed for Linux v1.

#### (j) What must degrade and how the UI shows it

- **Add a capability call**, e.g. `get_active_window_support` → `{ backend: "kwin" | "gnome-extension" | "wlr" | "x11" | "none", pid: bool, needs_setup: "install-gnome-extension" | null }`.
  - Settings → Privacy shows one status line, for example "App tracking: on (KDE)" or "App tracking needs the Mnema GNOME extension — Install (requires log out)".
  - Onboarding offers that install step on GNOME only.
- **App exclusions:** on Linux the copy must say exclusions hide frames *while the app is in front*, not that the app is hidden entirely. The tray dialog text in `status_bar.rs:656-658` needs a Linux variant.
- **No backend:** the timeline shows "Unknown app" grouping, the tray's "Exclude Current App" is greyed out with a tooltip, and recording itself still works.

#### (k) What to test on real hardware

1. Fedora 43/44 GNOME (Wayland, no AppIndicator by default) and Ubuntu 24.04/26.04 GNOME: the extension install → logout → signal flow, and the extension surviving a GNOME minor update.
2. KDE Plasma 6.x on Wayland and on X11: script load and unload across a KWin restart, behaviour when the tray menu is open, and multi-monitor focus.
3. Sway, Hyprland, niri, labwc: a protocol-only run and an IPC run, plus the PID mapping.
4. X11 under i3/XFCE: an Electron app missing `_NET_WM_PID`, and XRes.
5. Flatpak vs deb copies of Bitwarden, 1Password, Firefox, Chrome and Slack: what app_id / WM_CLASS each reports and whether desktop-id resolution and icons work.
6. CPU and wakeups at idle with each backend (target: no polling beyond the existing 1 s tick).
7. The lock screen through logind `LockedHint` on each desktop.

#### (l) Open questions

1. ~~GNOME extension: EGO or side-load?~~ **Decided:** bundle and side-load in v1, publish to EGO later (§10 decision 8).
2. ~~Is pause-while-focused an acceptable Linux privacy promise?~~ **Decided:** yes, with plain wording (§10 decision 3, ADR 0060).
3. ~~Is Flatpak a target?~~ **Decided:** not in v1 (§10 decision 12).
4. ~~COSMIC backend in v1?~~ **Decided:** yes (§10 decision 12).
5. Hyprland `activewindow -j` `pid` and the exact GNOME GJS method names (`get_pid`, `WindowTracker.get_window_app`) are unverified against current docs and need checking.

### 4.3 Accessibility tree and browser URL capture


**Answer in one line:** On Linux, read the active tab's URL through the desktop accessibility bus (AT-SPI2, via the `atspi` Rust crate). Firefox/Zen work out of the box once Mnema turns accessibility on. Chromium-family browsers only work if accessibility was on **when the browser started**, so the user has to restart them once. Treat a browser extension as a fallback that needs a new ADR, because ADR 0013 already removed one.

**Brief correction:** The brief said a WebExtension exists. **It does not.** No `manifest.json`, no `nativeMessaging`, no `NativeMessagingHosts`, and no extension directory exist anywhere in the repo (grep of the whole tree, excluding `node_modules`/`target`, returned nothing). ADR 0013 records that the extension, native messaging host, pairing flow and credential-entry suspension were **built on a branch and removed** (`docs/adr/0013-browser-integrated-sensitive-capture-v2.md:8-20`). ADR 0010 is rejected by 0013 (`docs/adr/0010-credential-entry-capture-suspension.md:1-7`). ADR 0039 lists an extension as an "explicit last resort" (`docs/adr/0039-gecko-browser-active-tab-url-via-accessibility-api.md`, Context and Considered Options). For Windows, SUPPORTS.md says "do not add browser extension plumbing without an ADR" (`SUPPORTS.md:208`).

**Credential-field / secure-field detection does not exist on macOS either.** A grep for `AXSecure`, `secure text`, `password field` and `credential field` across `crates/` and `apps/` found nothing. The only AX use in the codebase is the Gecko URL reader. On Linux, parity here means building nothing.

---

#### (a) Current macOS implementation

| Piece | Where |
|---|---|
| Per-browser strategy enum: `AppleScript(dialect)` for Chromium/WebKit, `Accessibility` for Gecko | `crates/capture-metadata/src/lib.rs:280-288` |
| Known-browser catalog, keyed by macOS **bundle id** (Safari, Chrome×4, Chromium, Edge×4, Firefox, Zen, Brave×3, Arc…) | `crates/capture-metadata/src/lib.rs:309-420` (Firefox `:385`, Zen `:391`) |
| Lookups: `is_known_browser_bundle`, `browser_url_strategy`, `browser_url_applescript`, `sanitize_url` | `crates/capture-metadata/src/lib.rs:452, 461, 476, 535` |
| Re-probe cache: same window title is re-checked every 5 s, with a 1.5 s floor between probes | `crates/capture-metadata/src/lib.rs:12, 24, 163-220` |
| URL used only as metadata, never for privacy decisions (`collect_browser_url_for_privacy: false`) | `crates/capture-metadata/src/lib.rs:259-270` |
| Dispatcher: pick a strategy, then run osascript or the AX read | `apps/desktop/src-tauri/src/native_capture_metadata.rs:587-598` |
| osascript runner (1 s kill timeout, 20 ms poll) | `apps/desktop/src-tauri/src/native_capture_metadata.rs:601-643` |
| Cached vs live read mode (lock-holding paths never do a live read) | `apps/desktop/src-tauri/src/native_capture_metadata.rs:289-345` |
| Non-macOS branch returns no snapshot at all, so Linux has no app/title/URL today | `apps/desktop/src-tauri/src/native_capture_metadata.rs:412-423` |
| AX reader for Gecko: `AXFocusedUIElement`, then up to 16 `AXParent` hops to the outermost `AXWebArea`, then `AXURL`. Hand-written FFI | `apps/desktop/src-tauri/src/native_capture_browser_url_ax.rs:57-73, 147, 197` |
| Trust check `accessibility_trusted()`, `request_accessibility_with_prompt()`, one-time first-sighting prompt | `native_capture_browser_url_ax.rs:78, 88, 104` |
| Time budgets: 500 ms cold-wake poll (50 ms steps), 400 ms per attempt, 0.5 s per AX message | `native_capture_browser_url_ax.rs:116-130` |
| Module mount | `apps/desktop/src-tauri/src/native_capture.rs:4-5` |
| Tauri commands `get_/request_/open_browser_url_accessibility…` (open uses an `x-apple.systempreferences:` URL) | `apps/desktop/src-tauri/src/native_capture.rs:2587-2665` (`:2658`); registered at `apps/desktop/src-tauri/src/lib.rs:775-777` |
| Permission helper window, `accessibility` kind | `apps/desktop/src-tauri/src/windows/permission_helper.rs:20, 35-36` |
| UI: Settings → Privacy row, onboarding item | `apps/desktop/src/lib/settings/state/gecko-url-access.svelte.ts`, `apps/desktop/src/routes/onboarding/onboarding-permissions.svelte.ts:44-147`, `apps/desktop/src/lib/settings/panels/capture/Privacy.svelte`, `apps/desktop/src/routes/onboarding/screens/PermissionsScreen.svelte` |
| Broker guard (only host+path leaves the app) | ADR 0038; `SUPPORTS.md:129` |

#### (b) How it works and performs on macOS

- **Chromium/WebKit:** spawns `osascript` per probe. No permission needed. Capped at 1 s (`native_capture_metadata.rs:613-643`). Probes are gated on title changes, with a 1.5 s floor and a 5 s backstop.
- **Gecko:** needs the Accessibility (TCC) permission. The first read wakes Firefox's accessibility engine, which takes about 100–150 ms (measured, per ADR 0039), and the engine then stays loaded for the life of that browser process. Worst case is about 0.9 s per attempt. Lock-holding paths only ever serve the cached URL.
- If the read fails, the URL is blank. The code never guesses from the address bar or the window list (ADR 0039, "Considered Options").

#### (c) Recommended Linux approach

**One recommendation:** add a Linux AT-SPI2 reader that sits behind the existing `BrowserUrlStrategy::Accessibility` and serves **every** supported browser on Linux. There is no AppleScript equivalent on Linux, so Chromium has to go through this path too.

- **Crate:** `atspi` 0.30.0 (released 2026-05-06; 0.29 2025-09-18; ~17M downloads; maintained by the Odilia screen-reader project; also used by `accesskit_unix`). It is pure Rust on `zbus ^5.5` (crates.io API, https://crates.io/crates/atspi; https://github.com/odilia-app/atspi). **`zbus` 5.14.0 is already in both lockfiles** (`Cargo.lock`, `apps/desktop/src-tauri/Cargo.lock`), so this adds little new to the dependency tree. Prefer its blocking or tokio API on the existing off-lock metadata thread.
- **What to read:** the URL attribute on the AT-SPI `Document` interface of the active document.
  - **Firefox/Zen** publish `DocURL` (and `MimeType`): Gecko `accessible/atk/nsMaiInterfaceDocument.cpp`, constants `kDocUrlName = "DocURL"` (https://searchfox.org/mozilla-central/source/accessible/atk/nsMaiInterfaceDocument.cpp).
  - **Chromium** publishes `URI` (plus `DocType`, `MimeType`): `ui/accessibility/platform/ax_platform_node_auralinux.cc:4634-4663` on main (https://chromium.googlesource.com/chromium/src/+/refs/heads/main/ui/accessibility/platform/ax_platform_node_auralinux.cc).
  - The algorithm mirrors macOS: find the focused element, walk up its parents to the outermost `DocumentWeb` role, then read the attribute. Keep the same time budgets and the same "no guessing" rule.
- **Turning accessibility on** (the Linux stand-in for the TCC grant). There is no permission dialog. Instead:
  - **Firefox/Gecko** watches the `org.a11y.Status.IsEnabled` property live and starts accessibility when it flips to true, with no restart (https://searchfox.org/firefox-main/source/accessible/tests/browser/atk/browser_forceEnabled.js). Mnema can set this property itself.
  - **Chromium** checks **only once, at startup**: the env vars `ACCESSIBILITY_ENABLED`/`GNOME_ACCESSIBILITY`/`QT_ACCESSIBILITY` = `1`, then `org.a11y.Status.IsEnabled`, then the GSettings key `org.gnome.desktop.interface toolkit-accessibility` (`ui/accessibility/platform/atk_util_auralinux.cc`, `ShouldEnableAccessibility()` and the `static bool initialized` in `InitializeAsync()`; https://chromium.googlesource.com/chromium/src/+/refs/heads/main/ui/accessibility/platform/atk_util_auralinux.cc). Chromium also scans for a running `orca` process and switches to full mode if it finds one (https://chromium.googlesource.com/chromium/src/+/lkgr/content/browser/accessibility/browser_accessibility_state_impl_auralinux.cc). Mnema is not Orca, so that path never applies.
  - **The UX that follows:** a single "Read browser URLs" toggle that (1) sets `IsEnabled`/`toolkit-accessibility` and drops `ACCESSIBILITY_ENABLED=1` into `~/.config/environment.d/`, then (2) tells the user to restart their Chromium-family browsers. **The user approves this before anything is written**, because it is a setting that affects the whole desktop. **Needs the owner's decision (§10).**
- **Identifying the browser and its window:** apps on the AT-SPI registry carry a name and a D-Bus connection, from which the pid can be read. The frame with the `Active` state is the focused window. This also works on GNOME Wayland, which has no standard "which window is active" API. The active-window question is covered in §4.2, but AT-SPI may be the best source for it on GNOME. **Unverified on hardware.**
- **The catalog needs Linux keys.** Linux has no bundle ids. Key browsers by `.desktop` id / Flatpak app id / process name. `org.mozilla.firefox` happens to be both the macOS bundle id and the Flatpak id; `firefox.desktop`, `chromium.desktop`, `google-chrome.desktop`, `brave-browser.desktop`, `zen.desktop` and the Flatpak ids would be new.

#### (d) Expected Linux behavior and gaps

- **Firefox/Zen:** should match macOS: a live, correct tab URL once accessibility is on. Same cost inside the browser as on macOS (the engine stays loaded).
- **Chromium/Chrome/Brave/Edge/Vivaldi:** this is a **downgrade from macOS**, which needs no permission and no restart. On Linux it needs (a) accessibility on at browser launch and (b) a browser restart after the user opts in. One more uncertainty: it is **not confirmed** whether the web-content tree, and with it the `URI` on the document, fills in when only the startup accessibility setup ran, without `--force-renderer-accessibility`. In 2020 Orca users had to pass that flag after Google turned off auto-detection because of slowdowns (https://mail.gnome.org/archives/orca-list/2020-May/msg00298.html). **This must be tested on hardware.**
- **Desktop-wide cost:** `IsEnabled=true` makes **every** toolkit app on the desktop (GTK3, Qt, Electron) export its accessibility tree, so this is not just a browser cost. Chromium's 2015 attempt to auto-enable via the bus was reverted because performance bots went red (https://codereview.chromium.org/1124173005/).
- **Wayland:** AT-SPI runs over D-Bus and does not depend on the compositor, so GNOME/KDE/wlroots behave the same for *reading trees*. GNOME 48 / KWin 6.4 / AT-SPI 2.56 only restricted **key-event** listeners to an allowlist; reading trees was not restricted (https://www.heise.de/en/news/Linux-desktop-Gnome-Between-financial-difficulties-and-technical-progress-10513277.html). The Wayland-native "Newton" project, if GNOME adopts it, could eventually replace or restrict the AT-SPI bus (https://www.osnews.com/story/139996/update-on-newton-the-wayland-native-accessibility-project/; https://wiki.gnome.org/Projects/GTK/Accessibility). Its status in late 2026 is **unverified**.
- **Minimal wlroots setups** (sway etc.) may not run `at-spi-bus-launcher`. It is normally D-Bus-activated by `at-spi2-core`, so it should start on first use **if the package is installed**.

#### (e) Fallbacks, in order

1. **WebExtension that posts to a localhost HTTP port** (the ActivityWatch model). `aw-watcher-web` asks for `127.0.0.1:5600` access and sends heartbeats over HTTP. It does **not** use native messaging (AMO permission list: https://addons.mozilla.org/en-GB/firefox/addon/aw-watcher-web; source: https://github.com/ActivityWatch/aw-watcher-web). It ships through AMO and the Chrome Web Store. The upsides: it works in Snap/Flatpak browsers (their network access is not sandboxed), on every compositor, and with no accessibility cost.
   - **Costs:**
     - It needs a new ADR that overturns 0013 and 0039's "last resort" stance.
     - It needs store accounts plus Firefox signing (release Firefox requires AMO signing, even for unlisted add-ons: https://extensionworkshop.com/documentation/publish/self-distribution/).
     - It needs a pairing token, because any local process or web page could POST fake URLs to the port. This is a new attack surface right next to the broker.
2. **WebExtension plus a native messaging host.** Manifest paths:
   - **Firefox:** `~/.mozilla/native-messaging-hosts/<name>.json`, system-wide `/usr/lib{,64}/mozilla/native-messaging-hosts/` (https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_manifests).
   - **Zen:** `~/.zen/native-messaging-hosts/`. This comes from a single user report only (https://www.1password.community/discussions/1password-work/request-for-zen-browser-communication-to-app-support-in-1password-linux/163326). **Verify it.**
   - **Chrome:** `~/.config/google-chrome/NativeMessagingHosts/`, `/etc/opt/chrome/native-messaging-hosts/`. **Chromium:** `~/.config/chromium/NativeMessagingHosts/`, `/etc/chromium/native-messaging-hosts/` (https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging). `allowed_origins` must list exact extension ids.
   - **Edge:** `~/.config/microsoft-edge/NativeMessagingHosts`. **Brave:** `~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts` (sources disagree: KeePassXC's wiki says Brave reads Chrome's path). **Vivaldi:** `~/.config/vivaldi/NativeMessagingHosts`. Sources: https://github.com/keepassxreboot/keepassxc-browser/wiki/Troubleshooting-Guide; https://git.hackliberty.org/Git-Mirrors/keepassxc/commit/a6e92c96d51f175c08b2a5214dba926a2841bf10.
   - **Snap/Flatpak breaks this.** The WebExtensions *portal* was never merged into a xdg-desktop-portal release; it shipped only as an Ubuntu patch. Its replacement, `xdg-native-messaging-proxy`, lives under the Flatpak org and ships in Ubuntu 26.04. Firefox picks it via `widget.use-xdg-desktop-portal.native-messaging-proxy` (https://firefox-source-docs.mozilla.org/toolkit/components/extensions/webextensions/native-messaging-portal-design.html; https://lists.freedesktop.org/archives/flatpak/2025-June/002390.html). Whether Chromium-family Flatpaks support it is **unverified**. **Ubuntu's default Firefox is a Snap**, so this fallback is weaker than (e)1.
3. **No URL.** This is the current Linux behavior, and the honest degrade.
4. **Window-title parsing (X11 `_NET_ACTIVE_WINDOW` + `_NET_WM_NAME`): rejected.** Browser titles hold the page title, not the URL, and ADR 0039 rejects guessed URLs. The title is already captured separately as window metadata where an active-window source exists.

#### (f) Risks

1. **Chromium needs a restart, and its web tree may stay empty without `--force-renderer-accessibility`.** If that is the case, Chromium URLs are effectively impossible without a launcher hack or an extension. **Top hardware unknown.**
2. **Turning accessibility on for the whole desktop has a performance and privacy cost.** Every app exports its UI tree onto a bus any same-user process can read. It also changes the user's desktop state, so turning it off has to undo exactly what Mnema set (track who flipped it).
3. **Future GNOME a11y lockdown.** Newton, or allowlisting of tree readers the way key events were allowlisted in GNOME 48, could break an unprivileged reader. If Mnema ships as a Flatpak itself, it needs `--talk-name=org.a11y.Bus`. Whether the Flatpak a11y proxy lets a sandboxed app *read* other apps' trees is **unverified**.

#### (g) Effort

**M.** The pieces:
- About 300 lines of reader with `atspi`, mirroring `native_capture_browser_url_ax.rs`.
- Linux browser catalog keys.
- Enable/disable plumbing for accessibility (D-Bus property, GSettings, `environment.d`).
- Linux Settings/onboarding copy plus a "restart your browser" hint.
- Splitting the over-limit metadata file.

An extension fallback would be **L**: ADR, two store listings, signing, pairing, Snap/Flatpak handling.

#### (h) Parity cells

| Capability | macOS | GNOME Wayland | KDE Wayland | wlroots | X11 |
|---|---|---|---|---|---|
| Chromium URL | **full**: AppleScript, no permission | **degraded**: AT-SPI `URI`; needs a11y on at browser start plus a restart; web tree may need `--force-renderer-accessibility` (unverified) | **degraded**: same as GNOME; uses `IsEnabled`/env var, since there is no GNOME GSettings key | **degraded**: same, and the at-spi bus must be installed | **degraded**: same as GNOME/KDE (AT-SPI does not depend on the compositor) |
| Firefox/Zen URL | **full**: AX `AXURL`, opt-in TCC | **full**: AT-SPI `DocURL`, turns on live via `IsEnabled`, no restart | **full**: same | **full** if at-spi2-core is installed | **full**: same |
| Credential-field detection | **not built**: rejected by ADR 0013; no code exists | **n/a**: nothing to port; technically possible via AT-SPI `ROLE_PASSWORD_TEXT` if the feature is ever revived | **n/a** | **n/a** | **n/a** |

#### (i) Files to move or split

- `apps/desktop/src-tauri/src/native_capture_metadata.rs` is **838 lines, already over the 800-line rule.** Move the macOS-only pieces (`frontmost_running_app_snapshot`, CG window list, `cf_*` helpers, `run_osascript`, `active_browser_url*`, lines ~426-643) into `native_capture_metadata_macos.rs`. Add `native_capture_metadata_linux.rs` with the Linux `active_browser_url` dispatch.
- New `apps/desktop/src-tauri/src/native_capture_browser_url_atspi.rs` (`#[cfg(target_os = "linux")]`). Mount the existing `browser_url_ax` module with `#[cfg(target_os = "macos")]` (`native_capture.rs:4-5`).
- `crates/capture-metadata/src/lib.rs` (1051 lines, also over the limit): split the browser catalog into `browser_catalog.rs`, with a per-OS identity field or per-OS tables, and make `browser_url_strategy` return `Accessibility` for every Linux entry.
- `native_capture.rs:2587-2665`: Linux versions of status/request/open. "Open settings" on Linux means GNOME Settings → Accessibility, or nothing. Rename the `geckoBrowsers` field, or add `chromiumNeedsRestart`. Also update `permission_helper.rs:35-36` and the frontend `gecko-url-access.svelte.ts`.
- Add `atspi` to `apps/desktop/src-tauri/Cargo.toml` under `[target.'cfg(target_os = "linux")'.dependencies]`.
- `SUPPORTS.md:38, 109-110, 276, 303`: update the rows.

#### (j) Degradation and how the UI handles it

- Accessibility off: no URL (as today), and the Settings → Privacy row shows "Turn on browser URL reading".
- Accessibility on but a Chromium browser was started before it: show "Restart Chrome/Brave/… to capture URLs". To detect this, look for a browser's AT-SPI app that has no document children, or that is missing from the registry entirely.
- The read times out: blank URL, same as macOS, logged at Debug. Never log the URL itself (CLAUDE.md logging rules).
- Unsupported compositor or no at-spi bus: hide the row. A failed probe must never block capture, matching `SUPPORTS.md:110`.

#### (k) What to test on hardware

1. Chromium/Chrome with `ACCESSIBILITY_ENABLED=1`, **without** `--force-renderer-accessibility`: does the document's `URI` show up in `accerciser`/`busctl`? Repeat with the flag.
2. Firefox on Snap (Ubuntu) and Flatpak: does flipping `IsEnabled` at runtime expose `DocURL`? Check Zen split view: does the focused pane win?
3. CPU and RSS of Firefox and Chrome with accessibility off vs on, idle and while scrolling a heavy page. Also the cost to other apps (Electron, GTK) once `IsEnabled` is true.
4. GNOME 48+/49 Wayland, KDE Plasma 6 Wayland, sway, and an X11 session: can the frontmost browser be found via the AT-SPI `Active` frame state?
5. Read latency, cold and warm, against the 400 ms budget.
6. Private windows: does AT-SPI expose a private-window marker? This is relevant if `private_browser_window_ids` (`capture-metadata/src/lib.rs:137`) ever gets filled in.

#### (l) Open questions

1. Is it acceptable to change the **desktop-wide** accessibility setting and require a Chromium restart, or is Chromium URL capture on Linux worth an extension (overturning ADR 0013)?
2. If an extension is chosen: should it talk over localhost HTTP (works under Snap/Flatpak) or native messaging (more private, but breaks under Snap/Flatpak)?
3. Will Mnema ship on Linux as a Flatpak? If so, AT-SPI reading from inside a sandbox has to be checked first.
4. Should Linux get its own ADR extending 0039 ("all Linux browsers use the Accessibility strategy")?

### 4.4 OCR


**Answer in one line:** On Linux, use the PaddleOCR engine Mnema already ships (`ocr-rs` on the MNN runtime, prebuilt for linux-x86_64 and linux-aarch64). Today it is compiled into every build but hidden from the provider picker. Unhide it, make it the default on Linux, keep Tesseract as the fallback, and run the repo's own OCR benchmark harness on a Linux laptop before you promise any speed figures.

Tags: [V] = verified in code or a primary source. [U] = uncertain or vendor-reported. Treat [U] items as hypotheses until the benchmark in (k) has run.

---

#### (a) What exists today (macOS)

**Three providers live in `crates/ocr`. All three are registered at runtime, but the UI offers only two.**

| Provider id | Engine | Code | Compiled on Linux? |
|---|---|---|---|
| `apple_vision` | Vision.framework `VNRecognizeTextRequest` via `cidre` | `crates/ocr/src/lib.rs:490-747`. The `cidre` dep is macOS-only (`crates/ocr/Cargo.toml:25-26`). | Compiles, but `recognize_apple_vision` returns "only available on macOS" (`lib.rs:749-755`), and `provider_runtime_available` reports it as unavailable (`lib.rs:1541-1548`). |
| `tesseract` | Tesseract built from source by `tesseract-rs 0.2.0`. Feature `tesseract-embedded` (`crates/ocr/Cargo.toml:10,19`). | `crates/ocr/src/tesseract.rs` | Yes. The build script has a Linux branch, caches to `~/.tesseract-rs`, links `stdc++`, and sets `OPENMP_BUILD=OFF` (`~/.cargo/registry/.../tesseract-rs-0.2.0/build.rs:27,241,347`). |
| `paddle_ocr` | PaddleOCR PP-OCRv5 mobile **on MNN, not ONNX**, via `ocr-rs 2.2.2` (`crates/ocr/Cargo.toml:8,17`) | `crates/ocr/src/paddle.rs` | Yes. `ocr-rs` downloads prebuilt MNN for `linux-x86_64` / `linux-aarch64` (`ocr-rs-2.2.2/build.rs:234-238`) and falls back to building MNN from source otherwise (`build.rs:56-60`). |

- **Desktop wiring.** `apps/desktop/src-tauri/Cargo.toml:69` enables both `paddle-rs` and `tesseract-embedded`. All three providers are registered at `apps/desktop/src-tauri/src/app_infra.rs:1514-1521`.
- **PaddleOCR is hidden from the picker.**
  - `is_desktop_selectable_ocr_provider` lets only `apple_vision | tesseract` through (`apps/desktop/src-tauri/src/ocr_models.rs:493-498`).
  - The fallback options in `Ocr.svelte:83-84` list only those two.
  - This has been true since commit `ae825586`. The reason isn't recorded anywhere. See (l).
- **Default provider and mode.**
  - Provider: `OcrProvider::AppleVision` on every platform (`crates/capture-types/src/recording.rs:186-188`).
  - Recognition mode: `Fast` (`recording.rs:201-203`; frontend `recording.svelte.ts:428`).
  - Language correction: off (`recording.rs:205`).
  - ADR 0004 only forbids *automatically falling back* to fast. It doesn't forbid fast as the default.
- **How a provider is selected per job.** The user's settings are frozen into each job's payload by `merged_ocr_payload_json` (`app_infra.rs:1243-1320`). Each provider gets its own options:
  - Apple Vision: `recognitionMode` and `languageCorrection`.
  - Tesseract: page-segmentation mode, preprocess mode, upscale factor, character whitelist.
  - PaddleOCR: none.
  - The backend then routes the job by provider id (`crates/app-infra/src/processing/ocr.rs:46-50,92`).
- **Model download.**
  - Apple Vision is managed by the OS and needs no download.
  - Tesseract downloads `eng` and `osd` `.traineddata` files from **tessdata_fast 4.1.0**, about 14.7 MB (`lib.rs:989-1024`).
  - PaddleOCR downloads det `.mnn`, rec `.mnn` and `charset.txt`, about 8.7 MB, pinned to a commit of `zibo-chen/rust-paddle-ocr` (`lib.rs:1026-1060`).
  - Every artifact is checked against a SHA-256 and can be swapped out with `MNEMA_OCR_*_BUNDLE_*` env vars (`lib.rs:966-987`).
  - Download, cancel and garbage-collection live in `apps/desktop/src-tauri/src/ocr_models.rs`.
- **Language configuration.**
  - The picker shows Tesseract codes: eng, fra, deu, spa, ita, por, nld, jpn, chi_sim (`apps/desktop/src/lib/settings/ocr-languages.ts:12-22`).
  - Apple Vision maps a few of those codes to BCP-47 tags and defaults to `en-US` (`lib.rs:684-709`).
  - Tesseract passes the code straight to `api.init` (`tesseract.rs:157-181`). **Only `eng.traineddata` is ever downloaded**, so choosing `fra` or `jpn` with Tesseract should fail at init. This is an existing gap, inferred from the code and not run.
  - PaddleOCR's bundle is English-only (`DEFAULT_PADDLE_OCR_LANGUAGE = "en"`, `lib.rs:38`).
- **Dead code that is easy to confuse with the live path.** `crates/app-infra/src/processing/apple_vision.rs` (506 lines) and `processing/engine.rs` are not declared in `processing/mod.rs:1-12`, so they are never compiled. They are an old copy of the Vision path. Delete them; don't port them.
- **Tests and benchmarks.**
  - Unit tests cover Tesseract option parsing and preprocessing (`tesseract.rs:422-506`), the Paddle manifest (`paddle.rs` tests), and backend routing (`processing/ocr.rs:118-391`).
  - Benchmark examples exist for all three providers. They run on two fixtures, 1710×1112 and 1108×720 (`crates/ocr/examples/*.rs`, harness `examples/common/mod.rs`), and report user/system CPU time and peak memory.
  - **No measured ms/frame or accuracy figures are committed anywhere in the repo.**

#### (b) How OCR runs on macOS

- **Frame input.** Screen-frame JPEGs are exported every 1 s (`crates/capture-screen/src/lib.rs:70`). Duplicate frames are filtered out by the captured-frame equivalence check before a job is queued (`docs/agents/capture-pipeline.md:79`).
- **Threading.**
  - One processing-worker loop runs one OCR job per pass (`app_infra.rs:3090-3112`).
  - PaddleOCR runs on `spawn_blocking` with **2 MNN threads**. You can override that with `MNEMA_PADDLE_OCR_THREADS` (`paddle.rs:385-391`). The engine is cached per bundle (`paddle.rs` `cached_runtime`).
  - **Tesseract creates and initialises a fresh `TesseractAPI` on every frame** (`tesseract.rs:179-181`). That is cheap to fix and likely matters on Linux.
- **Budget (ADR 0004, `apps/desktop/src-tauri/src/ocr_budget.rs`).**
  - The cooldown after each job is its run time × 4 while recording (about 20% duty cycle, clamped to 1–10 s). Paused or catching up, it is run time × 1.5 (about 40% duty, clamped to 0.25–2 s) (`ocr_budget.rs:31-48,514-530`).
  - Admission skips low-value frames when more than 3 jobs are queued (`:11`).
  - A representative frame is admitted every 15 s (`:12`), plus at most one "visually new" frame every 2 s (`:16`).
- **Measured cost on Mac.** "OCR is the whole of Mnema's CPU cost", about 1.4 W marginal on a fanless M4 Air (`ocr_budget.rs:37-41`).
- **Vision fast vs accurate.**
  - Apple documents fast as a small character-level model and accurate as a neural line/word model, with accurate as the framework default (https://developer.apple.com/documentation/vision/vnrecognizetextrequest/recognitionlevel).
  - No public per-frame millisecond figures exist for Apple Silicon Macs. A third-party test on iPhone 16 at 1080×1920 measured fast about 4× faster than accurate (30 vs 4 captures/s), and found that fast drops text smaller than 1/32 of the image height (https://pub.dev/packages/text_sight/versions/0.2.1) [U].
  - Apple doesn't document whether Vision runs on the Neural Engine (ANE). It's generally assumed to use the ANE or GPU [U].
  - Mnema downscales to at most 1800 px on the long side before Vision (`lib.rs:410,478-488`).

#### (c) Recommended Linux approach

**Recommendation: PaddleOCR (the existing `paddle_ocr` provider) as the Linux default, CPU-only.**

1. **Pick the default per platform.** `default_ocr_provider()` should return `PaddleOcr` when the target isn't macOS (`capture-types/src/recording.rs:186`). Onboarding and the settings fallbacks need the same change (`onboarding-settings-sync.ts:168`, `Ocr.svelte:83`).
2. **Unhide PaddleOCR.** Add `PADDLE_OCR_PROVIDER_ID` to `is_desktop_selectable_ocr_provider` (`ocr_models.rs:493`).
3. **Hide Apple Vision off macOS.** The capability already exists (`provider_runtime_available`). The UI just needs to filter on it, as `SUPPORTS.md:309` already intends.
4. **Keep the current bundle for the first cut.** That is PP-OCRv5 mobile English, 8.7 MB, already pinned and hashed.
   - Second step: bump `ocr-rs` from 2.2.2 to **2.4.x** (latest 2.4.1, 2026-08-05 — https://crates.io/crates/ocr-rs) and evaluate **PP-OCRv6 small**.
   - PP-OCRv6 shipped in PaddleOCR 3.7.0 on 2026-06-11. One model covers Chinese, English, Japanese and 46 Latin-script languages. It comes in tiny, small and medium tiers. Vendor-reported: tiny is 6.1× faster on M4 (https://github.com/PaddlePaddle/PaddleOCR/releases/tag/v3.7.0) [U].
   - `ocr-rs` already loads v6 files (https://github.com/zibo-chen/rust-paddle-ocr).
   - Korean, Cyrillic, Arabic, Devanagari and Thai still use PP-OCRv5 script-specific recognition models.
5. **No new crates.** The `ort`/ONNX route (RapidOCR-style) would add a second ML runtime for no gain, because MNN is already linked.

**Why not the alternatives:**
- **Tesseract** is mature and needs nothing new, but it is weaker on screen text:
  - It needs a dark→light inversion that Mnema doesn't do. Mnema's preprocessing is grayscale or Otsu only (`tesseract.rs:319-336`). Tesseract's own docs call out dark backgrounds (https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html).
  - On a form benchmark it scored F1 0.607 vs PaddleOCR's 0.787 (https://anthusai.github.io/Biblicus/guides/benchmark-results.html) [U: documents, not screens].
  - screenpipe uses Tesseract on Linux (https://docs.screenpi.pe/architecture). That proves it works, not that it is the best choice.
- **`ocrs`/rten** is still in "early preview" and reads Latin script only (https://github.com/robertknight/ocrs).
- **docTR, Surya and EasyOCR** are Python/PyTorch and GPU-oriented. They don't fit an embedded Rust app.

#### (d) Expected Linux performance and the gap

These are expectations, not measurements. Run (k) before you quote any of them to users.

- **Speed.** I found no published benchmark of PP-OCRv5-mobile on MNN with screenshots.
  - PaddleOCR's own CPU table gives about 1.75 s per *document* image for v5 mobile on a Xeon Gold 6271C, using Paddle Inference with roughly 10 cores busy. v4 mobile is about 1.37 s (https://www.paddleocr.ai/latest/en/version3.x/algorithm/PP-OCRv5/PP-OCRv5.html). Those are dense documents at full resolution.
  - Mnema caps detection input at 960 px (`paddle.rs:26`) and uses 2 threads.
  - My estimate for a 1080p screenshot on a 4–8 core x86 laptop: **about 0.3–1.5 s per frame** [U].
  - A cleaned-up Apple Vision "fast" path is probably several times faster per frame on M-series, but I have no Mac figures to compare against [U].
- **CPU.** The duty-cycle budget is relative to each job's run time (`ocr_budget.rs:42`). So average CPU stays bounded: about 20% of one job's thread budget (2 threads) while recording. A slower engine doesn't burn more CPU; it **indexes fewer frames and later**.
  - At 1 s per frame, the cooldown is 4 s, which gives about one OCR'd frame every 5 s.
  - The 15 s representative cadence and the 2 s novelty cap are unchanged, so expect more admission skips under pressure.
- **Accuracy.** PaddleOCR detects text at the line level, so it should handle dark mode and UI fonts better than Tesseract. Small text (under about 10 px) may lose out at the 960 px detection cap. Compare against Vision accurate on the same frames.
- **The user-visible gap:** OCR text becomes searchable later, and a bit more fan or heat on laptops. English-only until a v6 or multilingual bundle ships. CJK is available only after the v6 upgrade.
- **GPU.** MNN has optional `vulkan`, `opencl` and `cuda` features. Leave them off for v1, because they add driver risk and the work is paced anyway.

#### (e) Fallbacks (in order)

1. **Tesseract (already shipping).**
   - Fix the per-frame `TesseractAPI::new()`/`init` by caching the API per language (`tesseract.rs:179`).
   - Add a dark-background invert to preprocessing.
   - Download the traineddata for each language the user picks.
   - Consider `tessdata_best` over `tessdata_fast` for accuracy.
2. **PaddleOCR with a smaller or larger tier.** v6 tiny if CPU is tight, v6 small or medium if accuracy is short. It's a bundle swap only; the manifest already describes paths and thresholds (`paddle.rs:51-117`).
3. **RapidOCR / `ort` route.** Only if MNN prebuilts break on some distro or architecture.
4. **Turn the budget down** (see (j)).

#### (f) Risks

1. **Unpinned MNN prebuilt.** `ocr-rs 2.2.2` downloads MNN from `zibo-chen/MNN-Prebuilds` at tag **`dev`** (`ocr-rs-2.2.2/build.rs:7-8`). That tag can move, so builds aren't reproducible, and it's a supply-chain risk for release CI. Vendor or pin it, or use `build-mnn-from-source` in CI.
2. **No data, and a hidden-provider history.**
   - PaddleOCR was hidden from users for an unknown reason. It may have been an accuracy or crash problem found on Mac.
   - There are no committed quality numbers. The engine is a single-maintainer crate that moved from 2.2 to 2.4 in about a month, which brings API churn.
3. **Language coverage.** Today's bundle is English-only. Non-Latin users on Linux get nothing until v6 or script-specific bundles are wired up. The Tesseract language picker is already broken for anything other than English (only `eng` is downloaded).

Smaller risks:
- glibc/libstdc++ compatibility of the prebuilt `.a` with an older AppImage or Flatpak baseline [U].
- `tesseract-rs` builds Tesseract **5.3.4** (`build.rs:14`), but Mnema labels the model `tesseract-5.5.2` (`lib.rs:35`). This is cosmetic only.

#### (g) Effort

**M overall.**
- S: default plus unhide plus UI gating — about a day.
- S/M: Linux benchmark run and threshold tuning.
- M: v6 bundle plus multilingual model download and language mapping.
- S: MNN pin in CI.
- S (optional): Tesseract fixes.

#### (h) Parity

OCR works on image files after capture, so it is **independent of the desktop environment**. Whether there are frames to read is the screen-capture subsystem's problem, not OCR's.

| macOS | GNOME Wayland | KDE Wayland | wlroots | X11 |
|---|---|---|---|---|
| full (Vision) | degraded* | degraded* | degraded* | degraded* |

\* "Degraded" means: a different engine, slower per frame (so more budget skips and text searchable later), and English-only until the v6 or multilingual bundle. Nothing is impossible.

#### (i) Files to move, split or delete

- **Delete (dead code):** `crates/app-infra/src/processing/apple_vision.rs`, `crates/app-infra/src/processing/engine.rs`.
- **Split `crates/ocr/src/lib.rs`** (1719 lines, over the 800-line rule): move the Vision code to `apple_vision.rs` (`#[cfg(target_os="macos")]`) and the model manifest/install code (`lib.rs:757-1540`) to `models.rs`.
- **Edit:**
  - `capture-types/src/recording.rs:186`: per-platform default.
  - `ocr_models.rs:493`: selectable providers.
  - `Ocr.svelte:83-118`: the provider list comes from the backend, with no Apple fallback off macOS.
  - `ocr-languages.ts`: per-provider language lists.
  - `SUPPORTS.md:26-27,260`: update the OCR rows.
- `ocr_models.rs` (1219 lines) and `ocr_budget.rs` (1203 lines) are already over 800 lines and platform-neutral. Split them only if you touch them.

#### (j) Degradation and UI handling

- **No new mechanism is needed.** The budget already adapts to slower engines through run-time-relative cooldowns (`ocr_budget.rs:514-530`).
- **Optional small tweak:** a lower detection cap (`maxSideLen` 960 → 736) or v6 tiny on machines with low core counts.
- **Settings UI:**
  - List only runtime-available providers.
  - Show the engine's languages, not Tesseract codes.
  - Keep the existing "recording waits for the engine" copy (`Ocr.svelte:67`), so the PaddleOCR download (8.7 MB) is part of onboarding.
- **Never switch the user's provider or mode silently** (ADR 0004, Alternatives).

#### (k) Benchmark plan

- **Harness:** the existing examples (`cargo run -p ocr --features paddle-rs --example paddle_ocr_benchmark`, plus the Tesseract one). They already report wall time, CPU time and peak memory, and accept `--image` and expected text (`examples/common/mod.rs:36-47`).
- **Dataset:** about 100 real Mnema frames captured at 1080p, 1440p and 4K (downscaled the way capture does). Cover:
  - Light and dark mode.
  - IDE, terminal, browser, chat and spreadsheet.
  - Small text at 9–11 px.
  - CJK, Cyrillic and Arabic pages.
  - Ground truth from Apple Vision *accurate* on a Mac, hand-corrected.
- **Metrics:**
  - Character error rate and word recall. Recall matters most, because search is the use case.
  - Mean and p95 latency.
  - CPU-seconds per frame and peak memory.
  - Frames OCR'd per minute under the real budget during a 30-min recording.
- **Hardware:** a 4-core and an 8-core x86 laptop (one Intel, one AMD) and one aarch64 machine.
- **Configurations:** MNN at 1, 2 and 4 threads. Compare v5-mobile, v6 tiny and v6 small, Tesseract (fast vs best data, with and without inversion), and Vision fast/accurate on Mac as the reference.

#### (l) Open questions

1. Why has PaddleOCR been hidden since `ae825586`? Ask the user or check the commit discussion. It may hide a known defect.
2. Should the Linux default ship English-only v5 (already pinned) or go straight to v6 small for multilingual?
3. Pin MNN by vendoring a prebuilt, or build from source in CI?
4. Is CJK/Arabic coverage a Linux v1 requirement?
5. Should the default Mac provider also move to PaddleOCR for cross-platform consistency? Not recommended unless the benchmark shows it matching Vision.

### 4.5 Audio capture, transcription and speaker detection


_Line numbers refer to commit `85dcb993`._

#### Answer in one sentence

All of this ports to Linux. Two pieces are hard: recording "everything except the privacy-listed apps" (PipeWire has no ready-made equivalent of the macOS tap's exclude list, so we build one by linking ports ourselves), and every AVFoundation encode or decode call, which has to be replaced. Diarization is easier than the brief assumed: speakrs ships a CPU (ONNX Runtime) path, and the model files Mnema already downloads include the ONNX models it needs.

#### Brief correction: the diarizer is speakrs, not CAM++

- The repo uses **speakrs 0.4.2**, which runs pyannote community-1 segmentation, WeSpeaker ResNet34 embeddings and PLDA/VBx clustering (`Cargo.lock` `name = "speakrs" version = "0.4.2"`; `crates/speaker-analysis/Cargo.toml` `speakrs = { version = "0.4", default-features = false, features = ["coreml", "openblas-static"] }`). CAM++ does not appear anywhere.
- **speakrs has a CPU and ONNX path off Apple.** In the vendored source at `~/.cargo/registry/src/*/speakrs-0.4.2`:
  - `ort` (2.0.0-rc.12) is a non-optional dependency (`Cargo.toml:142-144`).
  - The features are `coreml`, `cuda`, `migraphx`, `load-dynamic`, `openblas-static`, `openblas-system`, `intel-mkl` and `default-linalg`. `coreml` is a hard `compile_error!` off macOS (`src/lib.rs:202-203`).
  - `ExecutionMode::Cpu` loads `PLDA_FILES` plus `ONNX_FILES` (`src/models.rs:182-188`). The README's mode table lists "`cpu` | ONNX Runtime CPU".
  - The README states the BLAS rule: on x86_64 the default is statically linked Intel MKL, and a no-default build must enable exactly one of `intel-mkl`, `openblas-static` or `openblas-system`.
- **No published CPU benchmarks.** speakrs' benchmark tables only cover CoreML (M4 Pro, 529x real time) and CUDA (RTX 4090, 59x). See https://github.com/avencera/speakrs/tree/master/benchmarks. CPU throughput on Linux has to be measured.
- What Mnema hard-codes today:
  - `ExecutionMode::CoreMl` (`crates/speaker-analysis/src/providers/speakrs.rs:189-192`).
  - A manifest that lists only the CoreML file set (`crates/speaker-analysis/src/lib.rs:155-158`, `SPEAKRS_COREML_FILES`; 76 files, 419 MB, `lib.rs:331-371`).
  - Chunking tuned for CoreML memory (`SPEAKRS_SAFE_CHUNK_SECONDS = 180`, `speakrs.rs:53`).
  - Decode through AVFoundation only (`providers/shared.rs:287-309`, `macos_audio_decode.rs:1-12`).

---

#### (a) Current macOS implementation

| Piece | Where | macOS-only mechanism |
|---|---|---|
| Microphone | `crates/capture-microphone/src/lib.rs` (2852 lines, over the 800-line rule) | `AVCaptureSession` plus an `AudioDataOutput` delegate on a dispatch queue (`lib.rs:1662-1760`, `AvFoundationMicrophoneCaptureSession` `:1763`). Device list and change notifier at `:2357`, `:2403-2600`. Permission at `:2790-2850`. Non-macOS stubs already exist (`:203`, `:304`, `:2381`, `:2602-2610`, `:2845`). |
| Mic VAD feed and trim | same file. VAD PCM feed `:321-352`, resample to 16 kHz `:1243-1278`, VAD boundary trim `:2058-2172` | Feeds silero/webrtc VAD. Trims the finished file with `trim_audio_file_to_m4a` (AVFoundation). |
| System audio | `crates/capture-system-audio/src/` (lib 124, tap 316, rebuild 716, exclude 790, writer 524, watchdog 187, activity 207, permission 174, probe 179) | Core Audio process tap plus aggregate device (`tap.rs`). The exclude list maps bundle ids to Core Audio process objects, including the "responsible app" (`exclude.rs:1-114`). Rebuild engine with five triggers (`rebuild.rs:1-22`). Gated to macOS 15 (`lib.rs:52-60`). |
| Platform-neutral already | `watchdog.rs`, `activity.rs`, `permission.rs`, `compute_exclude_list` in `exclude.rs:77` | No `cfg(target_os)` in them, so they are reusable. |
| Desktop adapter | `apps/desktop/src-tauri/src/native_capture/system_audio.rs` (556 lines) | A "segment bus" between the planner and `SystemAudioCaptureSession`. Pause keeps the tap alive (`:30-45`). |
| Writers | `crates/capture-writers/src/lib.rs` (2146 lines) | `AVAssetWriter` m4a/AAC (`:871-930`) with a default 48 kHz stereo format (`:583`). Tail-trim buffer keyed on `cm::SampleBuf` (`:323-520`). `trim_audio_file_to_m4a` (`:26`, AVAudioFile). `decode_audio_file_to_mono_pcm` (`:165`). |
| VAD | `crates/capture-vad` (silero 0.3 on `ort` rc.12, plus `webrtc-vad` 0.4) | No macOS code outside tests (`lib.rs:621-626`). It is portable as is. |
| Transcription | `crates/audio-transcription` | **Apple Speech:** `providers/apple_speech.rs`, already returns `UnsupportedPlatform` off macOS (`:518-526`). **Whisper:** `whisper-rs` 0.16. The macOS build adds `metal`. The non-macOS dependency is already declared without GPU features (`Cargo.toml` target sections), and `use_gpu(true)` is macOS-only (`local_whisper.rs:297-299`). **Parakeet TDT 0.6B v3 (ONNX):** runs on the **CPU execution provider even on macOS** (`parakeet.rs:561`). **Deepgram (cloud):** works today. One catch: it sends `Content-Type: audio/mp4` (`deepgram.rs:102`). |
| Transcription decode | `macos_audio_decode.rs` (934 lines) uses AVAudioFile with an AVAssetReader WAV fallback (`:188-206`). Waveform peaks at `:104`. | Linux stubs return errors (`local_whisper.rs:370-371`, `parakeet.rs:1278-1279`). |
| Diarization | `crates/speaker-analysis` (speakrs provider `providers/speakrs.rs`, mapping, enrollment `enrollment.rs`) | Runs out of process through the app binary re-exec'd as a helper (`speaker_analysis_runtime.rs:37`, `:164-330`). The helper is portable Unix code (`#[cfg(unix)]` at `:283`). |
| System-audio speech activity | `crates/app-infra/src/processing/system_audio_speech_activity.rs:143-165` | Decodes through AVFoundation. The non-macOS branch returns an error. |
| Voice enrollment | `apps/desktop/src-tauri/src/voice_enrollment.rs:143-175` | Bounded mic clip, macOS-gated. |
| Playback | `app_infra.rs:1150-1186` (`audio_segment_mime_type`, `"audio/mp4; codecs=mp4a.40.2"`). ADR 0049 plays clips in a webview `<audio>` element. | On Linux the webview (WebKitGTK) uses GStreamer, so AAC playback depends on which plugins the user has installed. See risks. |
| m4a hard-coding | The planner names every segment `*.m4a` (`crates/capture-runtime/src/lib.rs:209-238`). 38 files and about 199 non-test lines mention `.m4a`/`"m4a"`. | Matters if Linux switches codec. |

#### (b) How it works and performs on macOS

- **Microphone:** AVFoundation delivers `CMSampleBuffer`s on a serial dispatch queue. The delegate:
  - feeds the activity level and the VAD ring;
  - holds samples back until the format has been stable three times in a row (`MicFormatStabilityState`, `:454`);
  - appends to `AVAssetWriter`, which handles AAC encoding;
  - finalizes on 5-minute rotation, then runs an AVFoundation re-encode for the VAD boundary trim.
- **System audio:** a Core Audio IOProc copies each delivery into a `cm::SampleBuf` aligned for the encoder (`writer.rs:39-47`). The writer is created lazily once the format settles. **Recovery is only ever a rebuild**, and it happens on the caller's `poll` tick, never inside a listener (that would deadlock, `rebuild.rs:18-22`). There are five triggers:
  - default output device changed;
  - nominal sample rate changed (Bluetooth A2DP ↔ HFP);
  - device died;
  - exclude list moved;
  - the zero-watchdog fired (about 30 s of silence, backing off from 30 s to 600 s).
- **Sample rate:** the writer follows the aggregate device's nominal rate. Trusting the tap's claimed rate records speed-shifted audio.
- **Processing:** decode to mono 16 kHz, then:
  - Whisper on Metal;
  - Parakeet on CPU through ORT;
  - speakrs on CoreML at about 529x real time per its README.

#### (c) Recommended Linux approach and crates

**One recommendation:** go native PipeWire for both sources and switch the Linux audio format to Opus in Ogg.

1. **Microphone: `cpal` 0.18.2 with its `pipewire` and `pulseaudio` features.**
   - cpal 0.18.0 (2026-06-06) added a native PipeWire host and a PulseAudio host. On Linux it prefers PipeWire, then PulseAudio, then ALSA (local `cpal-0.18.2/CHANGELOG.md:95-96,167`; https://github.com/RustAudio/cpal/blob/master/CHANGELOG.md).
   - Streams opened on the default device follow default-device changes (`src/host/pipewire/device.rs:419-430`, `DefaultDeviceMonitor`).
   - Reuse Mnema's pure-math code (`resample_microphone_vad_frame`, the format-stability state machine and the activity peak tracking). Move it out of the `cfg(macos)` blocks.
   - **Permissions:** native Linux has no microphone permission gate, so the state is `Granted` when the stream opens. Under Flatpak, mic access comes from the static `--socket=pulseaudio` (or PipeWire) permission. No microphone portal is in use (uncertain).

2. **System audio: the `pipewire` crate (pipewire-rs, the 0.10 line cpal already pulls in) driven directly. Rebuild "tap with exclude list" as "link every app stream except the excluded ones":**
   - Create one virtual capture target that Mnema owns. The OBS-proven shape is a `support.null-audio-sink`-style node; a `Stream/Input/Audio` node with `node.autoconnect=false` is the alternative. Fix it at **48 kHz, F32, stereo**. PipeWire's adapter resamples each app stream into it, so the speed-shift bug cannot happen and no device rate needs tracking.
   - Watch the registry for `media.class = Stream/Output/Audio` nodes. For each one that is **not** excluded, create links from its output ports to our input ports. PipeWire mixes several links into one input port.
   - Exclusion rule:
     - `pipewire.sec.pid`, which the daemon verifies itself, or else `application.process.id`, walked up the `/proc/<pid>/stat` parent chain (this replaces the "responsible app" lookup);
     - plus `application.process.binary`, the Flatpak app id and the `.desktop` id;
     - plus Mnema's own pid tree, which covers the WebKitWebProcess that plays Receipt audio and the speaker helper.
   - This is the mechanism the OBS "PipeWire Audio Capture" plugin ships as "capture all except selected / multiple apps" mode (https://obsproject.com/forum/resources/pipewire-audio-capture.1458/updates, https://github.com/dimtpap/obs-pipewire-audio-capture; needs PipeWire 0.3.62+ and WirePlumber).
   - **What it replaces:** the device-change, device-death and sample-rate triggers mostly disappear, because we tap app streams rather than the output device, so a Bluetooth A2DP↔HFP switch does not touch our links. Exclude-list movement becomes **incremental** (add or remove one link) instead of a full rebuild.
   - **What stays:** the rebuild as the single recovery path for "PipeWire core error or disconnect" (daemon restart), and the platform-neutral `ZeroWatchdog` (`watchdog.rs`).
   - **Timeline continuity:** with no app playing, a plain capture stream is not scheduled and delivers nothing. The macOS tap delivers continuous zeros. So use a node that is always driven (null-sink style, or `node.always-process=true`), or pad silence from a monotonic clock. Transcript offsets assume the file timeline equals wall time.
   - **Use the RT callback only to copy** into an SPSC ring (for example `rtrb`). Encode on a writer thread.

3. **Encoding: Opus in Ogg on Linux. Keep AAC/m4a on macOS.**
   - Use libopus through the `opus`/`audiopus` crates (statically built) plus the `ogg` crate for pages. Opus is royalty-free.
   - Why not AAC on Linux:
     - It needs `fdk-aac(-free)`, LC profile only. Its licence excludes any patent grant and is GPL-incompatible. Fedora treats it as free but no longer "allowed" (https://en.wikipedia.org/wiki/Fraunhofer_FDK_AAC, https://gitlab.com/fedora/legal/fedora-license-data/-/issues/52). US AAC-LC patents are reported expired, but that comes from secondary sources and needs legal confirmation.
     - The alternative is FFmpeg's native encoder through `ffmpeg-next`, which is a large native dependency with distro variance.
     - **Webview playback of AAC on Linux needs gst-libav or the fdkaac plugin, which is often not installed.** WebKitGTK logs "The GStreamer FDK AAC plugin is missing, AAC playback is unlikely to work" (https://forum.manjaro.org/t/webkit2gtk-doesnt-work-well-with-m3u8-streams/168103). Ogg/Opus playback needs only gst-plugins-base, which WebKitGTK already depends on.
   - Deepgram accepts Ogg/Opus. Change its `Content-Type` to come from the file extension (`deepgram.rs:102`).
   - Make the extension a per-platform constant in `SegmentPlanner` (`capture-runtime/src/lib.rs:209-238`). Then fix the other 37 files that match on `m4a`, along with the MIME map (`app_infra.rs:1150-1160`).
   - **Trim:** for segments of 5 minutes or less, decode, slice and re-encode. Keep the same function signature and rename it something like `trim_audio_file`.

4. **Decoding: one shared non-macOS `decode_audio_file_to_mono_pcm`.**
   - Use `symphonia` 0.6 (Ogg demux) with `symphonia-adapter-libopus`, since Symphonia still has no native Opus decoder (https://lib.rs/crates/symphonia-adapter-libopus, https://docs.rs/crate/symphonia/0.6.0/source/README.md), plus `rubato` to resample to 16 kHz.
   - Put it in `capture-writers`, next to the existing function (`lib.rs:159-165`).
   - Call it from `audio-transcription` (whisper and parakeet stubs), `speaker-analysis` (`shared.rs:301`), `system_audio_speech_activity.rs:157`, and waveform peaks.

5. **Transcription:**
   - **Linux default: Parakeet v3 int8 on CPU.** The code is unchanged, because it is already CPU-EP on macOS.
   - **Whisper on whisper-rs:** CPU by default, with an optional `vulkan` feature (it exists in `whisper-rs-0.16.0/Cargo.toml` `[features]`) for AMD, Intel or NVIDIA GPUs. `cuda` and `hipblas` are also options but a heavier ship.
   - Apple Speech is hidden. Deepgram is unchanged.

6. **Diarization: speakrs with `default-features=false, features=["openblas-static"]`** (the same BLAS choice as macOS, so `openblas-build-env.sh` carries over).
   - `ExecutionMode::Cpu` behind `cfg(not(target_os="macos"))`.
   - A Linux manifest variant with the `PLDA_FILES + ONNX_FILES` set only, which is a much smaller download than the 419 MB CoreML set.
   - Re-tune `SPEAKRS_SAFE_CHUNK_SECONDS`.
   - Add `cuda` (with `load-dynamic`) later as an opt-in.

#### (d) Expected Linux performance and the gap

| Workload | macOS today | Linux x86 CPU (estimate) | Gap |
|---|---|---|---|
| Mic and system capture | negligible | negligible. PipeWire callback plus Opus encode of 2×48 kHz is under 1% of one core (estimate). | none |
| Parakeet v3 int8 | CPU EP | about **17–30x real time** (RTF 0.03–0.06) on an i7-4790 to i7-12700KF. Community numbers: https://github.com/groxaxo/parakeet-tdt-0.6b-v3-fastapi-openai | roughly parity, since macOS also runs on CPU |
| Whisper (whisper.cpp) | Metal GPU | CPU only. On a 5950X with AVX2 and 8 threads, a 30 s window takes about 1.46 s to encode with `small` and about 4.3 s with `medium` (https://www.github.com/ggerganov/whisper.cpp/releases). That works out to roughly 10–20x real time for small and 3–7x for medium; it is an estimate because the decoder adds time. Vulkan should claw much of this back. | large on CPU for medium and larger models |
| speakrs diarization | CoreML about 529x real time (M4 Pro) | **unmeasured.** My guess is 10–50x real time on a modern 8-core with ORT CPU, because WeSpeaker ResNet34 embeddings dominate. That is a **guess and must be benchmarked.** | 10–50x slower than CoreML, but still comfortably faster than real time for a 5-minute segment queue |
| silero VAD (ort) | CPU | CPU, same | none |

#### (e) Fallbacks (in order)

1. **System audio on PulseAudio-only systems,** or when we cannot get PipeWire registry and link rights (some Flatpak setups): capture the **default sink's monitor** through cpal (`STREAM_CAPTURE_SINK`, `cpal pipewire/device.rs:166-168`) or through the PulseAudio host. This has **no per-app exclusion**. Allow it only when the privacy list has no apps to filter or the user turned "Filter system audio" off. Otherwise keep system audio off and explain why in the UI.
2. **GStreamer instead of libopus/symphonia:** `gstreamer-rs` with `appsrc ! opusenc ! oggmux` and `decodebin`. GStreamer is already present wherever WebKitGTK runs. It is heavier, but one framework does both jobs, much like AVFoundation does on macOS.
3. **AAC on Linux,** only if keeping `.m4a` everywhere matters more: GStreamer `fdkaacenc` or `avenc_aac` at runtime, plus decoding AAC in Rust and serving PCM/WAV to the webview to avoid the GStreamer codec dependency.
4. **Diarization:** speakrs `cuda` on NVIDIA. Otherwise CPU only.

#### (f) Risks

1. **Privacy leaks through filter and loopback nodes (merge-blocker class).** EasyEffects, echo-cancel and filter-chain each own a `Stream/Output/Audio` node that carries *other apps' already-mixed audio*. Linking those nodes would record excluded apps, and the result would also be double audio. We must link only leaf app streams and skip virtual, filter and loopback nodes. Telling them apart is heuristic (`node.virtual`, owning client also exports an `Audio/Sink`, known binaries). It needs a dedicated drill equivalent to "Drill 1" in `docs/agents/system-audio-drills.md:29`.
2. **Identity and privacy-list model.** The privacy list is keyed on macOS **bundle ids** (`exclude.rs:61-71`). Linux needs `.desktop`/Flatpak app id plus binary name.
   - `application.process.id` is supplied by the client, and under Flatpak it is the pid inside the sandbox. A real dump shows `application.process.id=193` alongside `pipewire.sec.pid=3242` (https://bugs.kde.org/show_bug.cgi?id=516064).
   - Prefer `pipewire.sec.*`. How `sec.pid` behaves inside a sandbox is uncertain.
3. **Flatpak.** Native PipeWire needs `--filesystem=xdg-run/pipewire-0`; OBS documents this override (https://github.com/dimtpap/obs-pipewire-audio-capture). Flathub marks PipeWire and PulseAudio socket access as potentially unsafe (https://github.com/flathub/website/issues/1919). The access module tags Flatpak clients and restricts them (https://www.mankier.com/7/libpipewire-module-access). Whether a Flatpak client may **create links between other apps' nodes** is uncertain and needs a spike. Also note CVE-2026-5674, a PulseAudio-compat sandbox escape (https://bugzilla.redhat.com/show_bug.cgi?id=2501326).
4. **Our virtual sink becoming the default output.** WirePlumber could pick it, which would silence the user's speakers. Mark it not selectable or passive and hide it. Spike.
5. **Bluetooth headset mic forces HFP.** Recording from a Bluetooth headset mic flips the card to HFP (WirePlumber autoswitch), so all the user's playback drops to phone quality for as long as Mnema records. macOS behaves the same way, but a lifelogger always records, so this needs a product decision.
6. **Opus switch blast radius.** 38 files reference `m4a`. CLAUDE.md promises "Output is unchanged m4a/AAC", so the Linux format diverging is a cross-team invariant change, and ADR 0052 needs a Linux note.
7. **ORT prebuilt binaries.** `ort` `download-binaries` on older glibc. Two runtimes (ORT for Parakeet, silero and speakrs; ggml for whisper) on the same CPU contend for cores, so cap threads.
8. **The `<audio>` element and Tauri's custom URI scheme on WebKitGTK.** Media served through a custom URI scheme can fail to load on Linux; the fix is a blob URL (https://github.com/block/buzz/issues/2562). Check how `get_audio_segment_media` hands bytes to the page.

#### (g) Effort

| Item | Size |
|---|---|
| Mic on cpal, plus splitting the 2852-line file | M |
| System audio on PipeWire with link-based exclusion, self-exclusion, drills | **L** |
| Opus writer, trim, decode, extension constant across 38 files, MIME, Deepgram content type | M |
| Transcription (decode hook-up, Vulkan feature, hide Apple Speech) | S |
| Diarization (mode cfg, Linux manifest, chunk re-tune, benchmark) | S–M |
| Voice enrollment, probe and permission UX | S |
| **Total** | **L** |

#### (h) Parity

Audio on Linux does not depend on the compositor. It depends on the sound server: PipeWire everywhere modern, and PulseAudio or bare ALSA on older or minimal setups, which are more common among wlroots users. The cells below assume PipeWire with WirePlumber.

| Capability | macOS | GNOME Wayland | KDE Wayland | wlroots | X11 |
|---|---|---|---|---|---|
| Mic | full | full (cpal on PipeWire) | full | full (falls back to PulseAudio or ALSA) | full |
| System audio | full (process tap) | full (link app streams). Degraded to sink monitor on PulseAudio-only | same | same, PulseAudio-only more likely | same |
| Per-app audio exclusion | full (exclude list plus responsible app) | **degraded**: heuristic leaf-stream linking, filter nodes are a risk, Flatpak rights uncertain. **Impossible** on PulseAudio-only (monitor only) | same | same | same |
| Transcription | full (Apple Speech, Whisper on Metal, Parakeet, Deepgram) | degraded: no Apple Speech, Whisper on CPU or Vulkan; Parakeet and Deepgram at parity | same | same | same |
| Diarization | full (CoreML, about 529x) | degraded speed (ORT CPU, unmeasured), same model and DER class; CUDA optional | same | same | same |

#### (i) Files to move or split

- **`crates/capture-microphone/src/lib.rs` (2852 lines):** split into
  - `activity.rs`
  - `vad_feed.rs` (pure resample and boundaries)
  - `format_stability.rs`
  - `finalize.rs` (trim policy)
  - `macos.rs` (AVFoundation session and devices)
  - `linux.rs` (cpal)
- **`crates/capture-writers/src/lib.rs` (2146 lines):**
  - make the tail-trim buffer generic over a "timed PCM chunk", away from `cm::SampleBuf`;
  - split into `macos_writer.rs`, `opus_writer.rs` (Linux), `decode.rs` (macOS and symphonia) and `trim.rs`.
- **`crates/capture-system-audio`:** keep `watchdog.rs`, `activity.rs`, `permission.rs` and `compute_exclude_list`. Add a `pipewire/` module with `graph.rs` (registry and links), `identity.rs` (pid tree and app ids) and `session.rs`. Keep `SystemAudioCaptureSession`/`SegmentHooks` as the shared seam so `native_capture/system_audio.rs` does not change.
- **`crates/audio-transcription/src/macos_audio_decode.rs` (934 lines)** and **`crates/speaker-analysis/src/macos_audio_decode.rs`:** keep them for macOS and route Linux to the shared `capture-writers` decode.
- **`apps/desktop/src-tauri/src/audio_transcription_models.rs` (1629 lines):** already over the limit. Only the platform filter for Apple Speech changes.
- **`crates/speaker-analysis/src/lib.rs`:** per-platform `speakrs_required_files()` and artifact list.

#### (j) Degradation and UI

- **Settings → Sources → System audio:** on PulseAudio-only systems, or without PipeWire link rights, show "Can't filter per-app audio on this system". Turn system audio off when "Filter system audio" is on and the privacy list is non-empty. Never record silently without the filter.
- **Permissions:** the `x-apple.systempreferences:` helpers have no Linux counterpart. Replace the system-audio "possibly blocked" tri-state with simple "connected to PipeWire / not". Under Flatpak, show the `flatpak override` instruction.
- **Transcription provider picker:** hide Apple Speech. Default to Parakeet int8. Label Whisper medium and larger as "slow on CPU" unless Vulkan is detected.
- **Speakers:** show "Speaker analysis runs on CPU" and expect longer queue times.

#### (k) Benchmarks to run

1. speakrs `ExecutionMode::Cpu` on x86 (8-core desktop and 4-core laptop) and ARM64 Linux. Use the repo's DER harness `crates/speaker-analysis/src/bin/diarize_to_rttm_speakrs.rs` (with a mode switch). Measure RTFx, peak RSS per 180 s chunk, and DER versus CoreML on VoxConverse dev.
2. Parakeet v3 int8 and fp32 RTF and RSS on the same machines, with ORT thread caps.
3. whisper-rs small, medium and large-v3-turbo on CPU versus Vulkan, measuring RTF.
4. Capture overhead: CPU% and wakeups for cpal mic plus PipeWire link capture plus Opus encode over a 1-hour soak, plus quantum or xrun counts.
5. Exclusion drill matrix: browser (Chromium and Firefox), Flatpak app, EasyEffects active, Bluetooth A2DP↔HFP switch, `systemctl --user restart pipewire`. The expected result is that excluded apps are absent and the timeline stays continuous.

#### (l) Open questions

1. Accept Opus/Ogg on Linux (format divergence and a 38-file change), or keep `.m4a` using fdk-aac-free plus decode-to-PCM playback?
2. How should the Linux privacy list identify apps: `.desktop` id, Flatpak id or binary name? Should it reuse the screen-exclusion identity, covered in §4.1 and §4.2?
3. Can a Flatpak client create links between other clients' nodes under WirePlumber's default policy? This needs a spike. If not, ship system audio only as a native package (deb, rpm, AppImage).
4. Should the default mic avoid Bluetooth headsets so playback is not stuck in HFP?
5. Is the minimum supported PipeWire version 0.3.62, matching the OBS plugin, or 1.0?
6. ARM64 Linux: is speakrs' x86 MKL default irrelevant if we force `openblas-static`? I believe yes, but it is unverified.

### 4.6 ML inference runtimes


**Answer in one sentence:** Linux v1 should run every model on the CPU using code paths that already exist (candle on the CPU for embeddings, the ONNX Runtime that is already statically linked for Parakeet and for speakrs' CPU mode, whisper.cpp on the CPU), after three build fixes. Ship no CUDA, and gate the "Semantic Search works on Linux" claim on a measured benchmark, because candle on the CPU is several times slower than ONNX Runtime for BERT-sized models.

Repo facts below were checked against source at HEAD `85dcb993` and the crate sources in `~/.cargo/registry` (the exact locked versions). No cargo build was run.

---

#### (a) What runs on macOS today

| Runtime | Used by | How it's linked | Ref |
|---|---|---|---|
| **candle 0.10.2** (`candle-core/nn/transformers`, pinned exact) | Semantic Search embeddings (nomic-bert, XLM-R, Stella, ModernBERT) | Pure Rust. The `metal` feature is turned on only through a macOS target dep. | `crates/semantic-search/Cargo.toml:15-19,31-33`; `apps/desktop/src-tauri/Cargo.toml:70-74,120-124` |
| candle device pick | Tries Metal, falls back to CPU. F16 on Metal, F32 on CPU, ModernBERT always F32. | — | `crates/semantic-search/src/backend/candle.rs:108-113` (`arch_dtype`), `:609-624` (`pick_device`) |
| **ONNX Runtime 1.24.2 via `ort =2.0.0-rc.12`** | Parakeet transcription (CPU EP only). Also a hard dependency of speakrs. | Default features, so `download-binaries` is on and **`load-dynamic` is off**. ort-sys downloads pyke.io's prebuilt **static** `libonnxruntime.a` and links it into the binary (`cargo:rustc-link-lib=static=onnxruntime`). **No dylib is bundled on macOS.** `copy-dylibs` is on but does nothing for a static build. | `crates/audio-transcription/Cargo.toml:9,23`; ort-2.0.0-rc.12 `Cargo.toml` `default = [... "download-binaries", "copy-dylibs" ...]`; ort-sys `build/main.rs:154-160` |
| Parakeet sessions | `Session::builder().commit_from_file` with no EP list, so the default CPU EP, and the encoder is labelled `"cpu"` | — | `crates/audio-transcription/src/providers/parakeet.rs:550-562` |
| **whisper.cpp (ggml) via `whisper-rs 0.16`** | Local Whisper | Built from source with cmake. The `metal` feature is on for macOS only, the non-macOS dep has no features, and `use_gpu(true)` is set. | `crates/audio-transcription/Cargo.toml:37,40`; `providers/local_whisper.rs:297-299` |
| **speakrs 0.4.2** | Diarization and voice enrollment | `features = ["coreml","openblas-static"]`, run with `ExecutionMode::CoreMl` (native CoreML for segmentation and embeddings, plus **ort CPU** for the fbank front-end). | `crates/speaker-analysis/Cargo.toml:38`; `providers/speakrs.rs:189-192`; speakrs `src/inference.rs:266-267` |
| **OpenBLAS (static, built from source)** via `ndarray-linalg` | speakrs PLDA and VBx clustering (LAPACK) | `openblas-src 0.10.16`. `build.rs` force-loads static `libgfortran.a` and `libquadmath.a`, **but only when the target OS is macOS**. | `apps/desktop/src-tauri/build.rs:45-47,53-97`; `scripts/openblas-build-env.sh` (expects `brew install gcc` and `libgfortran.dylib`) |
| CoreML | speakrs only (the `.mlmodelc` bundles) | objc2-core-ml | speakrs `Cargo.toml` (macOS-target deps) |
| Other: **MNN** (via `ocr-rs 2.2.2`, `paddle-rs` OCR) | OCR (see §4.4) | The build script downloads a prebuilt MNN `dev` build from the third-party GitHub repo `zibo-chen/MNN-Prebuilds`. Linux x86_64 and aarch64 assets exist. | ocr-rs `build.rs:6-8,234-254` |

**Model files:** the speakrs manifest ships the CoreML set of 76 files, which includes `.mlmodelc` directories (`crates/speaker-analysis/src/lib.rs:63-80,330-345`). That set already contains every file that `ExecutionMode::Cpu` needs: PLDA `.npy` + `segmentation-3.0.onnx` + `wespeaker-voxceleb-resnet34.onnx[.data]` (speakrs `src/models.rs:181-205`).

#### (b) macOS behaviour and numbers (from repo)

- **Embeddings (candle on Metal, F16, nomic-embed-text-v1.5):**
  - Compared with the old fastembed/ort path, CPU use fell from about 140% to about 35%, and memory from about 1.7 GB to about 720 MB (`docs/adr/0037-...md:17-23`).
  - On an M4 Air, a batch of 16 anchors takes about 3.5 s, so the forward pass does about 4.6 anchors/s (`apps/desktop/src-tauri/src/semantic_search_worker.rs:185-190`).
  - The worker runs at a 25% duty cycle (cooldown multiplier 3.0, `:198`).
  - A backfill takes about 7 min per 1,000 anchors with the 2-window cap (`MAX_DOCUMENT_CHUNKS = 2`, `crates/semantic-search/src/runtime.rs:87`; ADR 0037 `:159-167`).
  - Windows are 256 tokens and sub-batches are 8 (`runtime.rs:96`).
- **Diarization:** upstream reports CoreML at about 529x real time on an M4 Pro, with 7.1% DER on VoxConverse dev ([speakrs README](https://github.com/avencera/speakrs)). Mnema splits audio into chunks of at most 180 s to cap CoreML memory (`providers/speakrs.rs:53`).
- **Parakeet:** already CPU-only ORT on macOS, so there is no GPU gap to port.

#### (c) Recommended Linux approach (one recommendation)

**Linux v1 is CPU-only for all three model families, and the work is build fixes, not new runtimes.**

1. **Embeddings: keep candle on the CPU (F32).** It is already the non-macOS path: the `metal` feature is macOS-target-only (`apps/desktop/src-tauri/Cargo.toml:120-124`), and `pick_device` returns `Device::Cpu` (`candle.rs:620-623`). This needs zero code. Do **not** turn on candle's `mkl` feature. It helped only about 1.5x in the one x86 report, adds a large static Intel binary, and doesn't fix the root cause (rayon overhead) ([candle#2877](https://github.com/huggingface/candle/issues/2877)).
2. **Transcription:**
   - **Parakeet:** use ort's CPU EP with the static pyke binary. That path already exists for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` (ort-sys `build/download/dist.txt`, `none` rows). No code change is needed.
   - **Whisper:** use whisper.cpp on the CPU (no feature flag). It **must** be built with `GGML_NATIVE=OFF`. Without it, ggml defaults `GGML_NATIVE=ON` (whisper-rs-sys `whisper.cpp/ggml/CMakeLists.txt:102-106,120`), which compiles with `-march=native`, so a CI-built binary will crash with an illegal instruction on older CPUs. whisper-rs-sys passes any `GGML_*` env var through to cmake (`build.rs:278-286`), so this is one CI env var. That var also needs to be added to `turbo.json` `passThroughEnv` if it goes through turbo.
3. **Diarization: speakrs `ExecutionMode::Cpu`** (ONNX Runtime CPU EP, the same static ORT) on non-macOS. This takes three edits:
   - Move speakrs' `coreml` feature to a **macOS target-dep entry** in `crates/speaker-analysis/Cargo.toml`, using the same feature-union trick as semantic-search. Today a Linux build hits speakrs' `compile_error!("the coreml feature is only supported on macOS")` (speakrs `src/lib.rs:202-203`). **This is a hard compile failure, confirmed by the compile check in §2.**
   - Pick the mode with `cfg` in `providers/speakrs.rs:189-192`.
   - Give non-macOS a manifest that is the subset of `SPEAKRS_COREML_FILES` without the `.mlmodelc` files (`lib.rs:63-80`).
4. **OpenBLAS on Linux:**
   - Keep `openblas-static`, built from source with **`OPENBLAS_DYNAMIC_ARCH=1`** for every distributed build. openblas-src reads it (`openblas-src-0.10.16/build.rs:176`). On x86 it builds kernels for every CPU family and picks one at runtime. Without it, OpenBLAS is tuned to the CI runner's CPU.
   - Extend `build.rs:45-47` with a Linux arm that statically links `libgfortran.a` and `libquadmath.a` (GNU ld: `-Wl,-Bstatic -lgfortran -lquadmath -Wl,-Bdynamic`, or a `-l:libgfortran.a` form). Otherwise the binary depends on `libgfortran.so.5`, which isn't installed by default on many distros. Note that `libquadmath` doesn't exist on aarch64.
   - `scripts/openblas-build-env.sh` hard-codes `libgfortran.dylib` and the hint `brew install gcc`. It needs a Linux branch (`.so`, `apt install gfortran`).
5. **No CUDA, ROCm or Vulkan in v1.** See (f) and (l) for why.

#### (d) Expected Linux throughput and gap (estimates, flagged)

| | macOS (Metal F16) | Linux candle-CPU F32 (est.) | Linux ort-CPU (fallback, est.) |
|---|---|---|---|
| nomic, 256-token windows, raw | ~4.6 anchors/s (M4 Air, repo) | **~0.5–1.5 anchors/s** on an 8-core AVX2 laptop | ~4–8 anchors/s |
| Backfill per 1,000 anchors at 25% duty | ~7 min | **~35–60 min** | ~7–15 min |
| Memory | ~720 MB (ADR 0037) | ~0.8–1.0 GB (137M params × 4 B ≈ 550 MB weights + activations) | ~0.6 GB with the arena off. ADR 0037 measured 1.7 GB with the arena on. |
| CPU while busy | ~35% | Several cores at 100% during each burst, paced to 25% duty | Same |

How the candle-CPU and ort-CPU numbers were estimated:
- [rust-embedding-bench](https://github.com/jerrythomas/rust-embedding-bench) measured all-MiniLM-L6-v2 at 200–256 tokens, batch 32, on M4 Max CPU cores: **candle 31 embeddings/s, ort 261, fastembed 315**. That makes candle about 8x slower.
- [candle#2877](https://github.com/huggingface/candle/issues/2877) measured MiniLM on an x86 i9-13900HX: **candle 50–122 ms per batch, PyTorch 14 ms, ort 8.5 ms**. MKL only got candle to about 54–83 ms. The issue is still open, and maintainers blame rayon overhead.
- nomic is about 6–8x the compute of MiniLM, so I scaled those numbers down. **Unverified; the benchmark in (k) decides.**

Diarization on the CPU: upstream publishes no CPU numbers ([speakrs README](https://github.com/avencera/speakrs)). For comparison, CUDA on an RTX 4090 runs at 59x real time, and CoreML at 529x. My guess for CPU is roughly 10–40x real time, which is still fine for 5-minute segments. That is an estimate to measure.

#### (e) Fallbacks (in order)

1. **If candle-CPU fails the gate:** add an **ort-CPU `SemanticSearchBackend`** for the default model only (nomic-embed-text-v1.5, which has an official ONNX export on Hugging Face). ort is already statically linked, so this adds **no new runtime**, only a second weights layout for one catalog entry. Apply the ADR 0037 mitigations: arena off (as speakrs does at `inference.rs:267`), an intra-op thread cap, and the existing duty-cycle pacing. This needs a short ADR, because it partly reverses ADR 0037 on Linux.
2. **Upgrade candle to 0.11.1** (released 2026-10-31, [crates.io](https://crates.io/api/v1/crates/candle-core)) if its CPU threading work lands. A maintainer reported large CPU gains on LLMs in #2877. You would need to re-run `candle_version_lockstep` and the parity tests.
3. **OpenBLAS:** switch to `openblas-system` and declare `libopenblas0` as a deb/rpm dependency, bundling it for AppImage. This avoids needing Fortran at build time, but makes the app depend on a system library.
4. **Opt-in GPU builds later:** ort `cuda` (pyke publishes `x86_64-unknown-linux-gnu+cu12/+cu13`, `+wgpu` and `+nvrtx` archives, ort-sys `dist.txt`). candle `cuda`. whisper-rs `vulkan`, `cuda` or `hipblas` (whisper-rs 0.16 features).

#### (f) Risks

1. **candle on the CPU is the weakest link.** It is about 6–14x behind ort on BERT-class models in public measurements, so backfill may take up to an hour per 1,000 anchors and the fans will run during bursts. The fallback in (e1) is ready but needs an ADR.
2. **CPU instruction baselines, which crash on older machines:**
   - pyke's x86-64 ORT binaries require **x86-64-v3 (AVX2/FMA, Haswell 2013 or later)** ([ort prebuilt binaries](https://ort.pyke.io/misc/prebuilt-binaries)).
   - ggml defaults to `-march=native`.
   - OpenBLAS without `DYNAMIC_ARCH` is tuned to the build machine.

   Mitigation: set `GGML_NATIVE=OFF` and `OPENBLAS_DYNAMIC_ARCH=1` in CI. Also check for AVX2 at startup and turn off Parakeet and speakrs with a clear message instead of crashing (`is_x86_feature_detected!("avx2")`).
3. **Compile and link breaks on Linux:**
   - speakrs `coreml` triggers a `compile_error!`.
   - The static gfortran trick in `build.rs` is macOS-only.
   - ORT links `stdc++`. ort-sys `static_link/mod.rs:20-30` defaults to `stdc++` on Linux. pyke's docs say Linux binaries "are built with Clang and depend on `libc++`", which is a **mismatch to check on a real Linux link**. If they really need libc++, set `ORT_CXX_STDLIB=c++` and ship or depend on `libc++1`.
4. **glibc floor:** Microsoft's own ORT 1.24 and 1.31 Linux builds target **manylinux_2_28 (glibc ≥ 2.28)** ([PyPI onnxruntime](https://pypi.org/project/onnxruntime/#files), [Anaconda 1.24.4](https://anaconda.org/anaconda/onnxruntime/files/modal/info/6a17631b4be4dc1619af8e61)). pyke doesn't publish its glibc floor (unverified). Check it with `objdump -T | grep GLIBC_` on the probe binary.
5. **Supply chain at build time:**
   - ort-sys downloads from `cdn.pyke.io`, checked by SHA-256 (`build/main.rs:126-137`).
   - ocr-rs downloads an unpinned `dev` MNN prebuilt from a personal GitHub repo.
   - Both need network access during the build. Flatpak builders run offline, so both artifacts would have to be vendored as Flatpak sources.
6. **What shipping CUDA would cost:**
   - The onnxruntime-gpu wheel alone is **255.6 MB**, versus **23.8 MB** for CPU ([PyPI](https://pypi.org/project/onnxruntime-gpu/#files)).
   - It also needs a matching CUDA 12.x or 13.x runtime plus **cuDNN 9** (ORT 1.21–1.26 are built against CUDA 12.8 + cuDNN 9; 1.27 and later against CUDA 13) ([ORT CUDA EP](https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html)). Bundling cuBLAS and cuDNN adds more than 1 GB (estimate).
   - It needs a proprietary NVIDIA driver, R525 or later for CUDA 12 minor-version compatibility. That figure is from memory and unverified ([NVIDIA compat](https://docs.nvidia.com/deploy/cuda-compatibility/)).
   - AMD: ORT **removed the ROCm EP in 1.23**, leaving only MIGraphX ([1.23 notes](https://newreleases.io/project/pypi/onnxruntime/release/1.23.0)).
   - Conclusion: no CUDA in v1.
7. **candle has no Vulkan or wgpu backend.** The 0.11.1 features are only `accelerate, cuda, cudnn, cutile, metal, metal-debug-labels, mkl, nccl` ([crates.io 0.11.1](https://crates.io/api/v1/crates/candle-core/0.11.1)). So AMD and Intel GPUs have no candle path ([WebGPU request #344](https://github.com/huggingface/candle/issues/344)).
8. **Side finding on macOS (not Linux):** ggml's `GGML_NATIVE=ON` default probably applies to macOS release builds too. An M4-built Whisper might use instructions an M1 lacks, the same class of bug as the OpenBLAS `DYNAMIC_ARCH` note in CLAUDE.md. **Unverified; worth checking.**

#### (g) Effort

**M overall.**
- **S** for the build fixes: speakrs feature gate, mode `cfg`, CPU manifest subset, Linux gfortran arm in `build.rs`, env script, and the CI env vars `GGML_NATIVE=OFF` and `OPENBLAS_DYNAMIC_ARCH=1`.
- **S** for the benchmark.
- **M** for fallback (e1), the ort embedding backend plus ADR, if the gate fails.

Audio decode, which blocks transcription and diarization on Linux (`providers/shared.rs:300-308`), is covered in §4.5 and isn't counted here.

#### (h) Parity cells (independent of desktop environment)

| | macOS | Linux CPU (v1) | Linux NVIDIA | Linux AMD/Intel GPU |
|---|---|---|---|---|
| **Embeddings** | candle on Metal, F16 (ModernBERT F32) | candle-CPU F32, about 5–8x slower to backfill (est.). Fallback: ort-CPU. | Deferred: candle `cuda` or ort `cuda`. Would be a separate opt-in build. | **None.** candle has no Vulkan/ROCm. ort MIGraphX or OpenVINO only through custom ORT builds. Use CPU. |
| **Transcription runtime** | Parakeet: ort CPU (same as Linux). Whisper: ggml Metal. | Parakeet: ort CPU, parity. Whisper: ggml CPU (slower than Metal). | Deferred: whisper-rs `cuda`, ort `cuda` (pyke cu12/cu13). | Deferred: whisper-rs `vulkan`/`hipblas`. Use CPU. |
| **Diarization runtime** | speakrs CoreML + ort CPU fbank + static OpenBLAS | speakrs `Cpu` (ort CPU) + static OpenBLAS with DYNAMIC_ARCH. Same DER family, slower. | Deferred: speakrs `cuda` (59x real time on a 4090 upstream) | Deferred: speakrs `migraphx` (AMD). Intel: CPU. |

#### (i) Files to change or split

- `crates/speaker-analysis/Cargo.toml:38`: move `coreml` into a `[target.'cfg(target_os="macos")'.dependencies]` speakrs entry, unioned with the base entry.
- `crates/speaker-analysis/src/providers/speakrs.rs:189-192`: `cfg` mode selection (`CoreMl` on macOS, `Cpu` elsewhere).
- `crates/speaker-analysis/src/lib.rs:63-80,330-345`: per-platform file table, with the CPU subset dropping `.mlmodelc` and the CPU-unused ONNX tails. Update the descriptor's display name and description, which say "CoreML" today.
- `apps/desktop/src-tauri/build.rs:45-97`: add a Linux static-Fortran arm, or split into `build/fortran_{macos,linux}.rs`.
- `scripts/openblas-build-env.sh`: Linux branch.
- The CI release workflow (Linux job): `OPENBLAS_DYNAMIC_ARCH=1`, `GGML_NATIVE=OFF`, and `GGML_AVX2=ON` as the baseline to match ORT's v3 floor.
- No change needed in `crates/semantic-search` or `semantic_search_worker.rs`. The worker comment at `:167-171` already expects candle on the CPU outside macOS.
- `SUPPORTS.md:31,32,96,105,308`: update. Lines 31 and 308 wrongly say speakrs "cannot run off Apple Silicon"; it has ort CPU, CUDA and MIGraphX modes (speakrs `src/inference.rs:47-60`).

#### (j) Degradation and UI

- **Settings → Semantic Search:** show the device the worker reports, "CPU" vs "GPU", using the existing `cpu_fallback` bool from `pick_device`. On Linux, add a one-line note: "Runs on your processor; first-time indexing can take a while." Keep the 25% duty pacing.
- **Larger models:** BGE-M3, Arctic-L and Stella (about 435–568M params, `models.rs:443,484`) would be very slow on the CPU. Label them "Not recommended without a GPU" on Linux, or hide them.
- **Transcription:** default to Parakeet int8 on Linux (CPU-native, so it matches macOS). Steer away from Whisper large on the CPU.
- **No AVX2 detected:** disable Parakeet, speakrs and the ort fallback with a typed "unsupported CPU" reason, the same idea as `ProviderUnavailable`. Keep Deepgram and candle embeddings, which don't need AVX2.

#### (k) Benchmark plan (the gate ADR 0037 already requires)

1. **Machines:**
   - x86 laptop (8C, AVX2, for example Ryzen 7 7840U or i7-1360P)
   - x86 desktop with AVX-512 (Zen 4)
   - aarch64 (Ampere/Graviton, or Asahi)
   - one pre-Haswell box to confirm the crash guard
2. **Embeddings:** use `crates/semantic-search/examples/smoke.rs` and `thermal.rs` plus the real corpus harvested by `scripts/semantic_bench/harvest_corpus.py` (2,409 anchors). Measure anchors/s, chunks/s, peak RSS, CPU%, and time to drain 1,000 anchors at duty 3.0. Run it for candle 0.10.2 CPU, candle 0.11.1 CPU, `RAYON_NUM_THREADS` = 4/8/all, and an ort-CPU nomic ONNX prototype.
3. **Pass bar:**
   - drain 1,000 anchors in 30 min or less on the 8-core laptop
   - RSS of 1.2 GB or less
   - no UI jank while OCR runs at the same time
4. **Diarization:** run `scripts/diarization_bench` with the `diarize_to_rttm_speakrs` binary in `Cpu` mode. Measure RTFx, DER against the CoreML baseline, and peak RSS per 180-s chunk.
5. **Parakeet and Whisper:** RTFx on a 5-minute segment for int8 and full Parakeet, and for whisper base and small with `GGML_NATIVE=OFF` and AVX2.
6. **Binary hygiene:**
   - `ldd` on the release binary: expect no `libgfortran`, `libopenblas` or `libonnxruntime`.
   - `objdump -T | grep GLIBC_` for the floor.
   - `objdump -d | grep zmm` to confirm no unguarded AVX-512.

#### (l) Open questions

1. Does pyke's Linux static ORT 1.24.2 need `libc++` (per their docs) or `libstdc++` (per the ort-sys rc.12 default)? What glibc floor does it have? Only a real Linux link can settle this.
2. Is the fbank ONNX in speakrs `Cpu` mode the same graph as in CoreML mode, so DER parity holds? The upstream table has no CPU row.
3. ort's default `tls-native` feature turns on `ureq/native-tls` (OpenSSL) in ort-sys's downloader. Is that only a build-time dependency on Linux, or does it pull `openssl-sys` into the app binary? `openssl` is in `Cargo.lock`, and ADR 0037 claims the app is native-tls-free.
4. Is it acceptable to need AVX2 for Parakeet and diarization? Every Linux desktop CPU from about 2015 on has it.
5. Should an opt-in "NVIDIA build" (CUDA ort + candle cuda + whisper cuda) be a v1.x artifact, or never? That is a product decision.
6. Can the `dev`-tagged MNN prebuilt be pinned or vendored for Flatpak's offline build? This is see §4.4.

##### Sources
- [speakrs README / benchmarks](https://github.com/avencera/speakrs)
- [rust-embedding-bench](https://github.com/jerrythomas/rust-embedding-bench)
- [candle#2877 CPU slowness](https://github.com/huggingface/candle/issues/2877)
- [candle WebGPU request #344](https://github.com/huggingface/candle/issues/344)
- [crates.io candle-core](https://crates.io/api/v1/crates/candle-core), [0.11.1 features](https://crates.io/api/v1/crates/candle-core/0.11.1)
- [ort linking docs](https://ort.pyke.io/setup/linking), [ort prebuilt binaries](https://ort.pyke.io/misc/prebuilt-binaries)
- [ORT CUDA EP](https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html), [ORT 1.23 notes (ROCm EP removed)](https://newreleases.io/project/pypi/onnxruntime/release/1.23.0)
- [PyPI onnxruntime](https://pypi.org/project/onnxruntime/#files), [PyPI onnxruntime-gpu](https://pypi.org/project/onnxruntime-gpu/#files), [Anaconda onnxruntime 1.24.4](https://anaconda.org/anaconda/onnxruntime/files/modal/info/6a17631b4be4dc1619af8e61)
- [NVIDIA CUDA compatibility](https://docs.nvidia.com/deploy/cuda-compatibility/)

### 4.7 On-device LLM


Researched 2026-11-01. Read-only on the repo. "UNVERIFIED" marks a claim I could not confirm against a primary source.

#### TL;DR

- **The brief is wrong about what exists.** The repo has **no Apple Foundation Models, no `InsightProvider` trait and no embedded llama.cpp**. A grep over `*.rs, *.toml, *.ts, *.svelte, *.swift, *.md` for `FoundationModels|InsightProvider|llama[-_]?cpp|llama_cpp` returns nothing.
- **What does exist:** one Rust crate, `crates/ai-runtime`, on `rig-core`/`rig-agent` 0.41 (`crates/ai-runtime/Cargo.toml:12-13`). It talks over HTTP either to a cloud provider (Anthropic, OpenAI, OpenAI-compatible, ChatGPT) or to a **user-installed** local server (Ollama or Llamafile) (`crates/ai-runtime/src/lib.rs:20,95-117`). Nothing is bundled.
- **None of the AI code is gated by `cfg(target_os)`.** The local path is just HTTP to `localhost`, so it already works on Linux as written.
- **Recommendation for Linux v1: keep "bring your own Ollama". Do not embed llama.cpp.** Add one fix: send `num_ctx` (the context window size) on Ollama calls. Effort **S**.

---

#### (a) Current implementation

| Piece | Where |
|---|---|
| Engine types: `CloudProvider` (Anthropic/Openai/OpenAiCompatible/Chatgpt), `LocalKind` (Ollama/Llamafile), `EngineConfig::Local{kind,endpoint,model}` | `crates/ai-runtime/src/lib.rs:95-139` |
| Structured extraction for local engines: `ollama::Client` → `.extractor::<T>()` with `retries(2)`; `llamafile::Client::from_url` | `crates/ai-runtime/src/lib.rs:561-592` (retries const `:79`) |
| Streaming agent loop (Ask AI) for local engines: `max_tokens(REASONING_MAX_TOKENS = 32768)`, dynamic tools | `crates/ai-runtime/src/agent_loop.rs:55, 316-341`; turn cap `MULTI_TURN_CEILING=64` `:64`, `max_turns = clamp(max_tool_calls)+1` `:380` |
| Reachability probe: a 2 s TCP connect | `crates/ai-runtime/src/lib.rs:62, 613-645` |
| Settings resolver and model listing (`/v1/models` for Ollama/Llamafile) | `apps/desktop/src-tauri/src/ai_runtime.rs:251-257, 1008` |
| Default endpoints: Ollama `http://localhost:11434`, Llamafile `http://localhost:8080` | `apps/desktop/src/lib/settings/state/ai-providers.ts:35-38` |
| Onboarding scan of ports 11434 / 8080 / 1234 | `apps/desktop/src/lib/onboarding/AiSetup.svelte:81-87`, `AiScanList.svelte:37-49` |
| Ask AI: one agent loop per turn; default 12 tool calls | `apps/desktop/src-tauri/src/ask_ai.rs:2234, 2526`; `crates/capture-types/src/recording.rs:515-517` |
| Ask AI system prompt: about 10.5 KB of source (roughly 2.5k tokens), plus about 15 tool schemas | `apps/desktop/src-tauri/src/ask_ai.rs:1088-1232` |
| User Context derivation: an `extract` call over up to 80 capture items × 1200 characters, in 2–30 minute windows | `user_context/worker.rs:85-92`; `user_context/derivation.rs:68, 491` |
| Distillation: 60 activities × 600 characters, plus 4k of authored context and 8k of known subjects | `user_context/derivation.rs:709-717, 1020, 1234, 1346` |
| Digest: prompt capped at 24,000 characters (about 6k tokens) | `user_context/digest.rs:218, 1088` |
| Local engine pacing: fixed 300 s tick | `user_context/worker.rs:64-66` |
| Design records: ADR 0028 (rig-core, hybrid local/cloud, the user runs the local server) | `docs/adr/0028-ai-features-call-models-rust-side-via-rig-core.md:9,23,51` |
| SUPPORTS.md already says local engines are "platform-agnostic" | `SUPPORTS.md:46,52,285` |

**One latent gap: we never send `num_ctx`.** rig 0.41's Ollama provider forwards `num_ctx` from `additional_params` into `options` (see its tests at `~/.cargo/registry/src/*/rig-core-0.41.0/src/providers/ollama.rs:1800-1852`), but `grep num_ctx` over `apps/` and `crates/` finds nothing.

#### (b) How it works on macOS

The user installs Ollama or Llamafile themselves, Mnema finds it with the onboarding port scan, and every call goes over HTTP. Ollama uses Metal on Apple Silicon. Its default context window is set by available memory: **4k tokens under 24 GiB of VRAM, 32k for 24–48 GiB, 256k at 48 GiB and above** (https://docs.ollama.com/context-length). So a 16 GB Mac gets a 4k window, and a large Ask AI prompt may already be cut short there. That is UNVERIFIED in practice, and it affects macOS too, not only Linux.

#### (c) Recommended Linux approach — keep bring-your-own Ollama

1. **No code change for the basic path.** Ollama ships an official Linux installer and a systemd unit (https://docs.ollama.com/linux). Arch ships `ollama`, `ollama-cuda`, `ollama-rocm` and `ollama-vulkan` at 0.40.13 (https://archlinux.org/packages/?q=ollama). The latest upstream release is v0.40.13, published 28 Oct (https://github.com/ollama/ollama/releases).
2. **GPU backends on Linux:**
   - **NVIDIA:** CUDA, compute capability 5.0 or newer, driver 550 or newer.
   - **AMD:** ROCm v7, from a separate `ollama-linux-amd64-rocm` tarball.
   - **Vulkan** (AMD/Intel iGPUs and other GPUs): "enabled by default when the backend is installed". It can be turned off with `OLLAMA_VULKAN=0`, and accurate VRAM reporting needs root or `cap_perfmon` (https://docs.ollama.com/gpu).
   - **History:** Vulkan went from experimental in 0.12.6-rc0 to opt-in in 0.12.11 (https://www.phoronix.com/news/ollama-0.12.11-Vulkan), and is now the default.
3. **Small fix (all platforms): send `num_ctx`** on the Ollama extract and agent calls through rig's `additional_params`, at about 16k–32k. Today the call sites are `lib.rs:573-582` and `agent_loop.rs:318-329`. Without this, a laptop with an iGPU or no GPU gets a 4k window, while the derivation prompt can reach about 24k tokens (80 × 1200 characters) and Ask AI's system prompt plus tools is about 5k tokens before any history.
4. **Copy and docs:** add a Linux "install Ollama" hint, and a SUPPORTS.md row naming GPU vendor support.

**Why not embed llama.cpp (the `llama-cpp-2` crate)?** It is active: v0.1.160, 15 Oct 2026, with features `vulkan`, `cuda`, `rocm`, `dynamic-backends` (https://crates.io/crates/llama-cpp-2). But embedding means we would own:

- Vulkan, CUDA and ROCm build matrices and shipping their binaries;
- model downloads (gigabytes);
- memory pressure and crashes inside the app process;
- a new provider kind that macOS does not have.

All of that is to save the user one `curl … | sh`. The ADR 0028 model (the user runs the local server) already works on Linux.

#### (d) Expected Linux performance and the gap

Generation speed is bound by memory bandwidth. Sources are in (k).

| Hardware | 7B/8B Q4 generation (tok/s) | Prompt reading (tok/s) | Feel |
|---|---|---|---|
| Apple M1 (68 GB/s) | 14.2 | 108–118 | baseline Mac |
| Apple M2 / M3 (100 GB/s) | 21.3–21.9 | 146–187 | |
| Apple M1/M2 Pro | 35.5–38.9 | 233–341 | |
| Apple M4 | 24.1 | 221 | |
| Zen 4 laptop CPU only (7840U-class) | ~13 (Mistral 7B Q4_K_M, ngl 0) | ~70 (median, Q8 pp2048, 7840HS; Q4 likely higher) | about M1 speed for generation, but slow to read prompts |
| Radeon 780M iGPU via Vulkan | ~18 (Mistral 7B Q4_K_M, ngl 99) | not measured for 7B (UNVERIFIED) | about M2 speed |
| RTX 3060 via Vulkan | 76–81 | 1816–2013 | far better than any base Mac |
| RTX 4060 laptop / RX 7600 | 58–61 | 1167–2358 | |

**The real Linux gap is prompt reading on CPU, not generation.**

- A 24k-token derivation prompt at about 70 tok/s takes **about 6 minutes per window**.
- The local tick is 300 s (`worker.rs:66`), so a CPU-only laptop may fall behind during busy stretches. This is an estimate, UNVERIFIED.
- Ask AI turns with 12 tool calls re-read the growing context on each call. With prompt caching that is fine; without it, expect tens of seconds before the first token on CPU.
- 3B models are roughly twice as fast but less reliable at tool and structured calls. The code already retries for this (`lib.rs:64-79`).

**Targets the code implies:**

| Setting | Value | Where |
|---|---|---|
| Ask AI local `max_tokens` | 32768 | `agent_loop.rs:55` |
| Ask AI tool calls | default 12, hard cap 64 turns | |
| Extraction retries | 2 | `lib.rs:79` |
| Digest prompt | about 6k tokens | |
| Derivation prompt | up to about 24k tokens | |

**Suggested floor:** 16 GB RAM and an 8B Q4 model. Recommend a GPU or 780M-class iGPU via Vulkan for Ask AI. On CPU only, use a 3B model or a cloud engine.

#### (e) Fallbacks

1. **Llamafile 0.10.x.** It was rebuilt in March 2026; Linux CUDA returned and Vulkan dylibs were added in 0.10.1 (https://blog.mozilla.ai/llamafile-reloaded-whats-new-in-v0-10-0/, https://www.helpnetsecurity.com/2026/03/20/llamafile-0-10-0-released/). It is already a supported kind and needs no code.
2. **Any OpenAI-compatible local server** (llama-server, LM Studio, vLLM) through the Llamafile kind or the OpenAI-compatible kind. Port 1234 is already scanned (`AiSetup.svelte:87`).
3. **A cloud engine.** This is blocked on the Linux key store (§4.8; see `SUPPORTS.md:46,52`).
4. **Last resort, v2:** embedded `llama-cpp-2` with `vulkan` plus `dynamic-backends`, only if user research shows that installing Ollama is a real drop-off point.

#### (f) Risks

1. **Context truncation (4k default under 24 GiB of VRAM).** Ollama quietly drops prompt content, so the model loses its instructions or tools and derivation quality falls with no error. The fix is to send `num_ctx`. This applies to all platforms and matters most on Linux laptops.
2. **CPU-only prompt reading is slow.** User Context can fall behind, and the worker paces at a fixed 300 s. We may need a "local engine is behind" status.
3. **GPU driver variance.** Vulkan needs Mesa or vendor drivers; AMD needs the `render` group; ROCm needs v7. Ollama's VRAM estimate is wrong without `cap_perfmon`. This is all outside our control and shows up as "slow", not "broken".
4. **Small-model tool calling is unreliable.** The ollama extractor uses rig's default forced `submit` tool (`lib.rs:574-582`), unlike the compat path's `tool_choice: auto` plus temperature 0.2 (`lib.rs:540-555`). Some local models may answer in prose, which shows up as `NoData`. The same applies on macOS.

#### (g) Effort

**S.** Sending `num_ctx` (2 call sites plus a test) is about 0.5 day. A Linux install hint in the UI and SUPPORTS.md is about 0.5 day. Manual testing on CUDA, Vulkan and CPU is 1–2 days. Embedding llama.cpp instead would be L.

#### (h) Parity cells

This feature does not depend on the desktop environment: it is HTTP to localhost.

| | macOS | GNOME Wayland | KDE Wayland | wlroots | X11 |
|---|---|---|---|---|---|
| Local engine (Ollama/Llamafile) | [x] Metal | [~] same code, untested | [~] | [~] | [~] |

What matters on Linux is the GPU vendor:

| GPU | Status |
|---|---|
| NVIDIA (CUDA) | [~] best |
| AMD dGPU (ROCm v7 or Vulkan) | [~] |
| AMD/Intel iGPU (Vulkan) | [~] usable for 7B/8B |
| CPU only | [~] slow prompt reading; 3B recommended |

#### (i) Files to move or split

Nothing needs to move: there is no platform-specific code. Edit `crates/ai-runtime/src/lib.rs` (Ollama extract branch) and `crates/ai-runtime/src/agent_loop.rs` (Ollama agent branch) to add `num_ctx`.

Existing size issues, not caused by this port:

| File | Lines |
|---|---|
| `crates/ai-runtime/src/lib.rs` | 991 |
| `crates/ai-runtime/src/agent_loop.rs` | 982 |
| `apps/desktop/src-tauri/src/ai_runtime.rs` | 2446 |
| `apps/desktop/src-tauri/src/ask_ai.rs` | 4644 |

All four are already over the user's 800-line rule. Do not touch them as part of the port.

#### (j) Degradation and UI

- **Ollama not running:** the existing probe and scan show it as not found. Add Linux copy: "Install Ollama: `curl -fsSL https://ollama.com/install.sh | sh`", or the distro package.
- **Slow hardware:** after a measured first probe, suggest a 3B model or a cloud engine. `ai-readiness.ts` already reports latency (`ai-readiness.test.ts:146`).
- **User Context backlog:** surface "local engine is catching up" instead of failing quietly.

#### (k) Benchmarks (sources)

- Apple M-series, LLaMA 7B Q4_0 pp512/tg128 (older llama.cpp build, so newer builds are faster): https://github.com/ggml-org/llama.cpp/discussions/4167
- Vulkan scoreboard, Llama 2 7B Q4_0 (RTX 3060 75.9–80.6 tg; RTX 4060 Mobile 59.5; RX 6600 50.6; RX 7600 58.0): https://github.com/ggml-org/llama.cpp/discussions/10879
- 780M via Vulkan: Mistral 7B Q4_K_M tg128 18.18 (ngl 99) vs 12.97 (ngl 0, CPU), on a UM890 Pro-class mini PC. This is one user report: https://github.com/ggml-org/llama.cpp/discussions/24222
- 7840HS CPU, Mistral 7B Q8_0 pp2048: median about 69 tok/s across public results (range 15–153): https://openbenchmarking.org/test/pts/llama-cpp (aggregated; UNVERIFIED for Q4)
- Ollama context defaults: https://docs.ollama.com/context-length
- Ollama GPU support: https://docs.ollama.com/gpu
- Ollama Linux install: https://docs.ollama.com/linux

**No measured source found** for Intel Arc / Meteor Lake / Lunar Lake iGPU generation speed, or for Q4 8B prompt reading on a laptop CPU. These need our own `llama-bench -p 512 -n 128` runs.

#### (l) Open questions

1. Is the 4k-context truncation already hurting macOS users on 16 GB machines? Check an Ollama debug log for "truncating input prompt".
2. Which `num_ctx` should we send? A fixed 16k or 32k, or derived from the model's `context_window` that we already list (`ai_runtime.rs:1272`)? Bigger windows use more memory.
3. Should the local worker interval adapt to measured throughput instead of the fixed 300 s?
4. Does "VRAM" in Ollama's 4k/32k rule count system RAM on CPU-only or iGPU machines? UNVERIFIED.
5. Should the ollama extractor switch to `tool_choice: auto` plus low temperature, like the compat path?

### 4.8 Database encryption keys and secrets


**Answer in one line:** put all three secrets (database key, vault master key, licence tokens) in the desktop keyring through the freedesktop Secret Service. Use a pure-Rust zbus client: `oo7`, or `keyring` v4 with its zbus Secret Service store. Both the app and `mnema-cli` read the same item, so the CLI needs no special sharing step. When no keyring answers, show a blocking choice instead of failing silently: "set up a keyring" or "store keys in an owner-only file (less secure)". Never fall back silently.

**The trade-off:** on Linux any program running as the same user can read a Secret Service item once the keyring is unlocked. That matches macOS *before* ADR 0057, when an old `security`-created item could be read silently. It is weaker than macOS today, where only the app and the CLI can read the database key.

---

#### (a) Current macOS implementation

The three secrets reach the OS keychain through three separate stores. Everything else goes inside the vault.

| Secret | Code | Keychain item |
|---|---|---|
| SQLCipher capture-index key | `crates/app-infra/src/capture_index_key_store.rs` (712 lines); `capture_index_key_store/shared_group.rs` (144); `capture_index_key_store/resolution.rs` (626) | service `day.mnema.capture-index` (`capture_index_key_store.rs:22`), account = `index_id`. Primary copy: data-protection item in group `RJYMY4RR97.day.mnema.capture-index` (`shared_group.rs:17`). Older copy: a silent item created by `/usr/bin/security` (`capture_index_key_store.rs:362-447`). |
| Vault master key | `crates/app-infra/src/secret_vault.rs:19-81` | service `day.mnema.vault`, account `master-key`, written with `security_framework::passwords::set_generic_password`, so only the app's code signature is on the item's access list |
| Licensing | `crates/app-infra/src/license_token_store.rs:6-16, 190-228` | service `day.mnema.licensing`, accounts `licensegate_key`, `licensegate_receipt`, `licensegate_first_seen` and `licensegate_trial_issuance` |
| Deepgram / AI keys / MCP / ChatGPT tokens | `ai_provider_key_store.rs:1-18`, `mcp_server_secret_store.rs:1-20`, `secret_vault_handle.rs:149-173` | **not in the keychain.** They sit in the vault file `secrets.vault`, encrypted with XChaCha20-Poly1305 (`secret_vault.rs:6-17`). Accounts look like `ai-runtime/transcription.deepgram`. The vault lives in the database base directory (`lib.rs:440`). |

**On non-macOS today, every secret path fails closed:**
- Database key: `capture_index_key_store.rs:449-468` and `:284-293`.
- Vault master key: `secret_vault.rs:83-97`.
- Licensing: `license_token_store.rs:230-247`.
- Hardware ID: `machine_id.rs:48-53`.

**How the CLI gets the database key.** `mnema-cli` calls `Database::initialize_brokered_reader` (`db.rs:153-162`), which resolves the key with `CaptureIndexKeyRole::Reader`. On macOS a reader reads the group item first, then the old item, and never writes (`resolution.rs:1-4`). The app is the `Owner` (`db.rs:122-125`) and is the only process that migrates the key. **The Linux fix therefore lands once in app-infra, and the CLI picks it up automatically.** The CLI never touches the vault (ADR 0057, "Considered options").

**Existing dev and test switches. These are worth reusing:**
- `MNEMA_CAPTURE_INDEX_KEY_DIR` uses `FileCaptureIndexKeyStoreAdapter` (`capture_index_key_store.rs:126-171, 242-246`). **It is not limited to debug builds.** It works in release too.
- `MNEMA_DEV_MASTER_KEY_FILE` uses `FileMasterKeySource` (`secret_vault.rs:99-142`), writes the file 0600 (`:127-136`), and is read only in debug builds (`:321-324`).
- `MNEMA_LICENSE_TOKEN_DIR` uses `FileLicenseTokenStoreAdapter`, which only exists under `cfg(any(test, debug_assertions))` (`license_token_store.rs:54-127`).
- The test guard `MNEMA_TRIPWIRE_NO_KEYCHAIN` is at `secret_vault.rs:336`.

#### (b) What macOS guarantees, and which guarantees the code relies on

1. **Only the app and the CLI can read the database key.** Every other process gets "not found" with no prompt (ADR 0057, Decision). This depends on a Developer ID signature, an embedded provisioning profile and a helper-bundle CLI (ADR 0057, "Verified findings").
2. **The key is readable at a login-item launch, before the user does anything.** The item uses `kSecAttrAccessibleAfterFirstUnlock` (`shared_group.rs:53-60`).
3. **The vault master key is tied to the app's code signature.** Other apps get a prompt (`secret_vault.rs:41-43`).
4. **Keys live outside `saveDirectory`.** So a copied, backed-up or synced capture library cannot be decrypted (ADR 0012, Consequences).
5. **Fail closed.** If the database exists but its key is missing, it is an error and a new key is never minted (`capture_index_key_store.rs:112-123`). If the vault file exists but its key is missing, the result is `Denied` and no key is minted (`secret_vault.rs:342-375`). Corrupt data is never silently reset.
6. **Licensing state survives uninstall,** which defeats a casual reinstall to reset the trial (`license_token_store.rs:8-12`).
7. **No calls block on a prompt in the headless CLI.** Agents call the CLI. A prompt there counts as a broken feature (ADR 0057, Context).

#### (c) Recommended Linux approach

**Use the D-Bus Secret Service (`org.freedesktop.secrets`) through a zbus client.**
- zbus 5.14 is already in `Cargo.lock` on Linux, pulled in by `tauri-plugin-opener` and `tauri-plugin-single-instance`. A zbus client adds **no libdbus system package**.
- Client options:
  - **`oo7`** (0.6, maintained under linux-credentials). It picks D-Bus or a sandbox file backend on its own (https://docs.rs/oo7/latest/oo7/).
  - **`keyring` v4.2** with `zbus-secret-service-keyring-store`. v4 split into `keyring-core` plus one crate per store (https://docs.rs/keyring/latest/keyring/, https://github.com/open-source-cooperative/keyring-rs).
- Pick whichever matches zbus 5 without pulling in a second zbus major version. **UNVERIFIED:** I did not check the zbus version pins of oo7 0.6 or the v4 store crate. Check before choosing.
- Avoid `dbus-secret-service-keyring-store` (needs libdbus-1-dev) and `linux-keyutils-keyring-store`. Keyutils is "completely in-memory and will not persist across reboots" (https://docs.rs/linux-keyutils-keyring-store/latest/linux_keyutils_keyring_store/). That would lose the database key on reboot and brick the database.

**Shape of the change (smallest version):**
- Add one Linux module with three functions: `load(service, account)`, `store(service, account, bytes)` and `delete(service, account)`.
  - Items use attributes `{service, account}` plus a label, in the default collection, so `secret-tool lookup service … account …` finds them.
  - `store` must update in place ("replace"), never delete and recreate.
- Wire it into the existing `#[cfg(not(target_os = "macos"))]` platform functions in the three stores. Change the cfg to `target_os = "linux"` and keep the Windows stubs.
- Item identities are the same on every OS:
  - Database key: `day.mnema.capture-index` / `<index_id>`
  - Vault master key: `day.mnema.vault` / `master-key` (32 raw bytes, or hex-encoded to match the file source)
  - Licensing: `day.mnema.licensing` / `licensegate_*`
- Linux needs **no owner/reader migration**: there is no old item and no group. The current non-macOS `resolve_platform_key_for_role` (`capture_index_key_store.rs:284-293`) already ignores the role and never writes from a reader, because `load` returns `Some` once the app has created the key. Keep that.
- **CLI key sharing needs no extra work.** The app and `mnema-cli` run as the same user on the same session bus and read the same item. The one requirement is that the CLI can reach the session bus (`DBUS_SESSION_BUS_ADDRESS` is set, or derivable from `$XDG_RUNTIME_DIR/bus`).

#### (d) Expected behaviour and the security gap

**Normal case: GNOME, or KDE with Secret Service enabled, keyring unlocked at login through PAM.**
- First launch mints the key silently. Later launches and every CLI call read it silently.
- KDE 6: `ksecretd` provides `org.freedesktop.secrets`; KDE recommends enabling the API (`apiEnabled=true`) and pointing PAM at `ksecretd` (https://wiki.archlinux.org/title/KDE_Wallet, https://github.com/KDE/kwallet).

**The gap: no per-app access control.** Any program running as the same user can read every item in an unlocked collection. GNOME treats the user session as the trust boundary, and CVE-2018-19358 was disputed on those grounds (https://lists.gnome.org/archives/gnome-keyring-list/2011-October/msg00016.html, https://bbs.archlinux.org/viewtopic.php?id=285012).

| Secret | macOS today | Linux |
|---|---|---|
| Database key | only the app and the CLI can read it | any same-user program can read it (`secret-tool lookup service day.mnema.capture-index ...`). Same as macOS before ADR 0057. |
| Vault master key | other apps get a prompt | any same-user program can read it silently. |
| Licensing | in the keychain | same exposure. Low impact: the licence is signed, and the receipt is checked offline. |

**What Linux still protects against:**
- An offline copy of `saveDirectory`: the key is outside it.
- An offline copy of `~`: gnome-keyring encrypts `~/.local/share/keyrings/*.keyring` with the login password.
- Other user accounts.

**Mnema already accepts the same-user boundary elsewhere.** The CLI grant file and the opaque-id MAC secret are plain 0600 files (`brokered_access.rs:45, 1509-1550, 3467`). So the key-store gap does not lower the broker's real security floor below what is already shipped.

**Locked or prompting cases:**
- With autologin, or with a keyring password different from the login password, the login collection stays locked. The first read triggers an unlock prompt through gcr-prompter.
  - This prompt can silently fail under Sway and minimal window managers (https://lists.freebsd.org/pipermail/freebsd-questions/2021-September/294798.html, https://bbs.archlinux.org/viewtopic.php?id=282720).
  - The app can show it, but a headless CLI call from an agent over SSH or cron cannot.
- KeePassXC as provider: with "confirm when passwords are retrieved" on, items are individually locked and clients get `IsLocked` unless they unlock the item first, and each access may prompt (https://github.com/keepassxreboot/keepassxc/issues/9334). This breaks the CLI's "no prompt" rule.

#### (e) Fallbacks, in order

1. **No Secret Service reachable** (no daemon installed, D-Bus activation fails, or no session bus): show a blocking dialog with two choices:
   - **"Set up a keyring"**: install or enable gnome-keyring or KWallet, with a docs link.
   - **"Store keys in an owner-only file (less secure)"**: an explicit choice that is persisted.

   File mode reuses the existing file adapters (`FileCaptureIndexKeyStoreAdapter`, `FileMasterKeySource`, `FileLicenseTokenStoreAdapter`). Rules:
   - Keys go in `$XDG_DATA_HOME/day.mnema/keys/` with the directory at 0700 and files at 0600. **Never in `saveDirectory`**: ADR 0012 forbids fallback keys there.
   - The licensing file adapter must come out of `cfg(debug_assertions)`.
   - The choice is sticky. Never fall back silently, and never auto-migrate between modes without proof the database opens, following ADR 0057's "migrate then delete after proof".

   File mode is the same against same-user malware. Unlike gnome-keyring, it is *not* encrypted at rest, so it relies on full-disk encryption.
2. **Later improvement for file mode:** seal the file with `systemd-creds encrypt --user` (systemd ≥ 256; the key is bound to the TPM2 chip, the UID and the machine-id) (https://man.archlinux.org/man/systemd-creds.1.txt). This adds protection for an offline copy of `~`, with the same same-user exposure. **UNVERIFIED:** whether user-scope decrypt works without polkit on stock distros. Not for V1.
3. **Refuse** (ADR 0012's literal rule): this is the default before the user chooses. Recording cannot start until a key store exists.
4. **Flatpak:** the Secret portal hands a per-app secret to sandboxed apps (https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html), and oo7 uses it automatically. But a host-side `mnema-cli` called by agents cannot reach a secret held inside the sandbox. So Flatpak would need the app to serve the data, which ADR 0057 rejected. Not recommended for V1.

#### (f) Risks

1. **Locked keyring at autostart.** A login-item launch before unlock, or autologin, leads to a prompt or an error.
   - Existing code already refuses to mint a new key when the database exists (`capture_index_key_store.rs:116-118`) or when the vault file exists (`secret_vault.rs:362-367`).
   - **The danger is a fresh install.** If the first launch hits a locked keyring and the user cancels, nothing must be created. Make sure "locked/dismissed" maps to an error, never to "not found, so mint".
2. **The CLI from an agent has no session bus or cannot unlock** (SSH, cron, systemd service, KeePassXC confirm mode). The result is a confusing failure. The error must name the cause ("keyring locked or unreachable"), as ADR 0057 does for the missing entitlement (`shared_group.rs:99-111`).
3. **Weaker isolation than macOS.** Must be stated in SUPPORTS.md and the privacy copy. It is a real downgrade for the database key (ADR 0057's threat model).
4. **Provider quirks.**
   - KWallet/ksecretd in Plasma 6 has reports of hangs and "Could not connect to Secret Service" (https://forum.manjaro.org/t/plasma-6-kwallet-hangs-and-dolphin-sftp-fails-missing-secret-service-backend/178130). Anecdotal.
   - KeePassXC needs a per-item unlock call.
   - The default collection may not exist; creating it prompts.
5. **Build.** `libsqlite3-sys 0.30.1` with `bundled-sqlcipher` on Linux builds SQLCipher against **system OpenSSL**.
   - The non-Apple branch links `dylib=crypto` and uses the default include path unless `OPENSSL_DIR` or `OPENSSL_LIB_DIR`+`OPENSSL_INCLUDE_DIR` are set (`~/.cargo/registry/src/*/libsqlite3-sys-0.30.1/build.rs:143-219`). macOS uses CommonCrypto instead (`:212-215`).
   - **The build needs OpenSSL headers:**
     - Debian/Ubuntu: `libssl-dev`
     - Fedora: `openssl-devel`
     - Arch: `openssl`
   - It also needs a C compiler and `pkg-config`.
   - **Runtime needs `libcrypto.so.3`.** Declare it in the .deb/.rpm (`libssl3`, `libssl3t64` on Ubuntu 24.04+, or `openssl-libs`) and make sure the AppImage bundles it.
   - The cost is low: `native-tls` → `openssl-sys` is already in the lockfile (via reqwest/ureq), so Linux needs libssl anyway.
   - Alternative: a Linux-only `[target.'cfg(target_os="linux")'.dependencies] libsqlite3-sys = { features = ["bundled-sqlcipher-vendored-openssl"] }` links OpenSSL statically. It needs perl and make at build time, plus OpenSSL security updates become our job.
   - `sqlite-vec 0.1.9` is portable. Its `build.rs` only compiles `sqlite-vec.c` with `SQLITE_CORE` and ships its own `sqlite3.h`, with no SIMD flags, so there is nothing Linux-specific (`sqlite-vec-0.1.9/build.rs`). It is registered with `sqlite3_auto_extension` at `db.rs:31-48`.
   - **Separate problem:** `app-infra` depends on `capture-screen`, `ocr`, `capture-writers` and others. The compile check in §2 shows they do once two one-line `cfg` fixes land. Nothing in the *key* code blocks the build: `security-framework` and `core-foundation` are macOS-only (`crates/app-infra/Cargo.toml` target section).
6. **`MNEMA_CAPTURE_INDEX_KEY_DIR` works in release builds** (`capture_index_key_store.rs:242`). On Linux, anything that can set the app's environment (for example `~/.config/environment.d`) can redirect the key to a file it controls. This is the same class of issue the absolute `/usr/bin/security` path guards against (`:364-368`). Consider limiting it to debug builds, or accept it, since a same-user attacker can read the keyring anyway.

#### (g) Effort: **M**

- **Code: about 1–2 days.** One Linux keyring module of about 150 lines, wired into three stores. Plus the file-mode choice: a setting, a dialog and the un-gated licensing file adapter.
- **Testing: about 2–3 days.** Across GNOME, KDE, wlroots, X11, the no-daemon case and the locked-keyring case.
- **Packaging dependencies: part of the packaging slice.**

#### (h) Parity matrix

Labels:
- **full**: same behaviour as macOS
- **degraded**: works, with the gap noted
- **impossible**: cannot work

| Item | macOS | GNOME Wayland | KDE Wayland (Plasma 6) | wlroots (Sway/Hyprland) | X11 (any WM/DE) |
|---|---|---|---|---|---|
| DB key | full (app + CLI only) | degraded: silent, PAM-unlocked, but any same-user program can read it | degraded: same; needs ksecretd Secret Service enabled (on by default in recent Plasma; **unverified per distro**) | degraded: only if gnome-keyring or KeePassXC is installed, running and unlocked through PAM. Otherwise prompts may fail, so file mode or refuse | depends on the desktop: same as GNOME/KDE under those, same as wlroots under i3 and other bare WMs |
| Vault master key | full (signature-bound) | degraded: no app binding | degraded | degraded or file mode | degraded or file mode |
| CLI key sharing | full (access group, helper bundle) | degraded: same item, no grant needed; requires session bus. Headless/SSH: works only if keyring already unlocked | degraded: same | degraded: KeePassXC confirm mode breaks headless | degraded: same |
| Licensing store | full | degraded: works; any user tool (Seahorse) can delete it, which resets the trial locally (the trial is server-issued anyway) | degraded | degraded or file mode (easy to delete) | degraded |
| (related) hardware ID for activation | full | **impossible today**: `machine_id.rs:48-53` stub errors; `/etc/machine-id` is the obvious source (§4.12) | same | same | same |

#### (i) Files to move or split

- `crates/app-infra/src/capture_index_key_store.rs` (712 lines, near the 800-line limit). Move the macOS `security` CLI functions (`:362-447`) into `capture_index_key_store/macos_legacy.rs`, and add `capture_index_key_store/linux.rs`, or a shared `platform_keyring/linux.rs`.
- **New:** `crates/app-infra/src/platform_keyring_linux.rs`, one small module used by all three stores.
- `crates/app-infra/src/secret_vault.rs` (787 lines, at the limit). Move `KeychainMasterKeySource` for each OS into `secret_vault/key_source.rs` before adding Linux.
- `crates/app-infra/src/license_token_store.rs`: replace the non-macOS stubs (`:230-247`) and un-gate the file adapter for file mode.
- `crates/app-infra/Cargo.toml`: add a `[target.'cfg(target_os = "linux")'.dependencies]` entry for the oo7 or keyring store crate.
- `SUPPORTS.md` rows 58 and 59 plus the cloud-key rows (46, 50–52), and the Linux checklist (`:280`).
- New ADR, "Linux key store is Secret Service with explicit file opt-in", amending ADR 0012 and ADR 0057.

#### (j) Degradation and UI

- **Startup check:** "Secret Service available and unlocked?" Three outcomes:
  - Yes: silent.
  - Locked: the app triggers the unlock prompt, and the user sees the keyring dialog.
  - Missing or dismissed: blocking onboarding/settings card: "Mnema needs a keyring to protect your capture index. [How to set one up] [Use a protected file instead — less secure]". The choice is recorded.
- **Settings → Privacy:** show which store is active ("System keyring (GNOME Keyring / KWallet)" or "Protected file"), plus one sentence: "Other apps running as you can read this key on Linux."
- **CLI errors:** "Mnema's keyring is locked or unreachable from this session (no D-Bus session / keyring locked). Unlock it or run from your desktop session." Never "key missing" when the real cause is that the keyring is locked.
- **Cloud AI, Deepgram and MCP rows:** unchanged once the vault unlocks. In refuse mode the vault is `Denied`, so they show the existing denied state (`SecretVaultDenied`).

#### (k) What to test

1. **Unit tests:** reuse the existing adapter-trait tests. Add a Linux adapter test that runs against a throwaway `oo7-daemon` or `gnome-keyring-daemon --unlock` in a private `dbus-run-session` (CI: `dbus-run-session -- sh -c 'echo pw | gnome-keyring-daemon --unlock; cargo test -p app-infra platform_keyring'`).
2. **Mapping:** "locked" and "denied" map to `Err`/`Denied` and never to "not found, so mint". Test on a fresh install and with an existing database.
3. **Store updates in place.** Item count stays at 1 after a re-store.
4. **Distro matrix:** GNOME (Fedora, Ubuntu), KDE Plasma 6 (ksecretd), Sway with gnome-keyring through PAM, Sway with no daemon (fallback dialog), i3 on X11, KeePassXC provider (confirm on and off), autologin (locked collection).
5. **CLI:** from a desktop terminal, over SSH while the desktop is unlocked, over SSH while locked, and from a systemd user service. Check `mnema-cli search` against the app's database.
6. **Reboot persistence:** proves keyutils is not used.
7. **Offline-copy check:** copying `saveDirectory` to another user or machine cannot open the database.
8. **Build:** clean Ubuntu 24.04 and Fedora containers with only `build-essential pkg-config libssl-dev`. Check the produced binary with `ldd`: only `libcrypto.so.3` and `libssl` are expected, and there is no libdbus.

#### (l) Open questions 

1. **No keyring:** offer the owner-only file opt-in, or refuse (ADR 0012's literal reading)? Recommended: the opt-in with a clear warning. The file is outside `saveDirectory`, so ADR 0012's rule still holds.
2. **Do we accept the documented downgrade** of the database key to same-user-readable on Linux? The only stronger option is the app serving the data (the CLI asks the running app), which ADR 0057 rejected for cost.
3. **Should `MNEMA_CAPTURE_INDEX_KEY_DIR` become debug-only** now that a Linux release could hit it?
4. **oo7 or keyring v4?** Decide after checking zbus version compatibility. oo7 is better if Flatpak ever matters.
5. **Is Flatpak in scope?** If yes, the CLI key-sharing model has to change.

### 4.9 Background and lifecycle


**Answer in one line:** put one small Linux "session events" module on **zbus** (logind for suspend/lock, one per-desktop idle reader, GDK for monitor changes, notify-rust for the reminder banner). Most of this subsystem is either already portable (low disk) or has no macOS code to port (App Nap, QoS, launch at login, power/thermal).

Repo paths are relative to the repo. "Uncertain" marks claims I could not confirm from a primary source.

---

#### (a) What exists today on macOS (with file refs)

| Concern | Where | Notes |
|---|---|---|
| Input idle reading | `apps/desktop/src-tauri/src/native_capture_system_idle.rs:7-43` | `CGEventSourceSecondsSinceLastEventType(CombinedSessionState, AnyInput)`. Returns `Option<u64>` ms. The non-macOS stub returns `None` (`:45-48`). |
| Idle consumers | `native_capture/activity.rs:60-68,158,207`; `native_capture/not_recording_reminder.rs:161` | Activity snapshot for the inactivity policy, plus the reminder's "typed in the last minute" check. `activity.rs:66` has a *second* non-macOS stub that returns `None` without calling the idle module. |
| Inactivity policy | `native_capture_inactivity.rs` (1735 lines, mostly platform-neutral) | Sources are `SystemInput`, `ScreenCapture`, `MicrophoneCapture`, `SystemAudioCapture` (`:7-31`). It already handles a missing source: `fallback_idle` (`:211`) is used when idle is unavailable. |
| Will-sleep / did-wake / screens sleep and wake | `native_capture.rs:1469-1528` (`start_system_wake_notifier`); stub `:1530-1531` | Four `NSWorkspace` observers. Will-sleep runs `handle_system_will_sleep` (`native_capture/lifecycle.rs:567-600`, macOS-gated), which stops the screen writer so the tail `.mov` gets its `moov` atom. Did-wake runs `recover_screen_capture_after_system_wake` (`native_capture.rs:~1430`, macOS-gated), as does the missed-wake resync (`~1451`). Screens-sleep hides the webview views (`windows.rs:912-928`) and emits `screens_did_sleep` / `screens_did_wake`. |
| Display reconfiguration | `native_capture.rs:1533-1656` | A `CGDisplayRegisterReconfigurationCallback`. At the end of every reconfiguration it emits `display_configuration_changed` (`:1595`). Only when a display comes online does it run wake recovery (`:1597-1604`). The flag logic is tested in `native_capture/tests.rs:99-130`. |
| Offscreen clamp | `apps/desktop/src/lib/render-idle-clamp.ts`, `render-idle.svelte.ts`; opt-in in `routes/+layout.svelte:97` | Frontend only. Uses `availableMonitors()` and `setPosition`. |
| Render-idle gate | `apps/desktop/src/lib/render-idle.svelte.ts:19-80` | `screensAsleep \|\| visibilityState !== "visible"`. It **fails open**: if the events never arrive, the gate just follows the page's visibility. |
| Not-recording reminder | `native_capture/not_recording_reminder.rs` (306 lines) + `not_recording_reminder/macos_notification.rs` (253 lines) | A 30 s poll thread (`:26,126-139`). The decision logic `ReminderRuntime::observe` (`:47-80`) is pure and tested. Away = `CGDisplay::main().is_asleep()` or `loginwindow` frontmost (`:192-197`); the non-macOS stub `user_away() -> false` is at `:199-202`. Native banner via `UNUserNotificationCenter` with Start/Snooze buttons. |
| Webview cache purge | `webview_cache.rs:31-50`, called from `windows.rs:1311` on blur | Rate-limited to once per 60 s. Calls `WKWebsiteDataStore` to drop `MemoryCache`. No-op on other OSes. |
| Low disk | `native_capture/disk_space.rs` (platform-neutral math), probe `:162-164` = `fs2::available_space` | Checked only at segment boundaries (`segments.rs:999-1003`) and in capture health (`capture_health.rs:446`). |
| Deferred startup | `lib.rs:467-491` (`run_deferred_startup`), spawned on the `mnema-deferred-startup` thread at `lib.rs:956-973` | Platform-neutral. The reminder thread and recording auto-start are spawned from there (`lib.rs:489-491`). |
| **App Nap** | **none** | No `NSProcessInfo beginActivity`, no `NSAppSleepDisabled` in `Info.plist` (grep found nothing). |
| **QoS** | retired | The per-thread background-QoS downclock was removed (ADR 0037); see `semantic_search_worker.rs:28,868`, `semantic_search_query.rs:178-180`. |
| **Power / thermal** | deliberately absent | `semantic_search_worker.rs:188-197`: the worker throttles itself by duty cycle and deliberately does **not** read the thermal state or Low Power Mode. |
| **Launch at login** | **none** | The `autoStart` setting means "start recording when the app launches" (`lib/settings/panels/general/Startup.svelte:17`, `lib.rs:491`), not "start the app at login". No `tauri-plugin-autostart` in `apps/desktop/src-tauri/Cargo.toml:37-90`. |

#### (b) How it behaves on macOS

- Everything is push-based except idle (polled 1 s by the segment loop, `segments.rs:46`; 30 s by the reminder).
- Sleep: will-sleep finalizes the tail. The microphone and system audio keep running (ADR 0021/0052). Wake recovery has two triggers that are safe to fire together: did-wake and the display coming back online.
- Low disk: no polling while healthy. Suspends all sources, resumes with hysteresis (ADR 0040).
- Render-idle exists only to work around a WebKit-on-macOS leak (Apple FB16462982: an IOSurface is stranded on every repaint while the screens are asleep, per `native_capture.rs:130-133`).

#### (c) Recommended Linux approach

**One recommendation:** add a Linux-only `native_capture/linux_session/` module on **zbus 5** (pure Rust, async, no libdbus) with five small pieces. Each feeds the *existing* macOS seams unchanged.

1. **Suspend/resume: logind (works on every desktop).** Hold a *delay* inhibitor (`Inhibit("sleep", …, "delay")`). On `PrepareForSleep(true)`, run the Linux twin of `handle_system_will_sleep`, then drop the inhibitor fd. On `PrepareForSleep(false)`, call `recover_screen_capture_after_system_wake`, emit `system_did_wake`, and take a new inhibitor. Signal semantics: "sent right before (true) or after (false)" sleep; holders "should release their inhibition locks" ([logind D-Bus docs](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html)). The delay cap is `InhibitDelayMaxUSec` from logind.conf (default 5 s from memory, *uncertain*).
2. **Lock state: logind `LockedHint` first, desktop signal as backup.** Watch `PropertiesChanged` on the session object. GNOME Shell sets it ([GNOME bug 764773](https://bugzilla.gnome.org/show_bug.cgi?id=764773)). KDE kscreenlocker added it ([KDE MR 10](https://invent.kde.org/plasma/kscreenlocker/-/merge_requests/10)). COSMIC does not ([upstream note](https://forge.light.kow.is/kowis-projects/computer-unlocker/src/tag/v1.0.2/docs/upstream-issues/cosmic-greeter-lockedhint.md)). swaylock is unknown (*uncertain*). Backup signal: `org.gnome.ScreenSaver.ActiveChanged(b)` ([gsd XML](https://git.stg.centos.org/source-git/gnome-settings-daemon/blob/9ca0cf1f74994e5b17a01c13008127c59de114ef/f/gnome-settings-daemon/org.gnome.ScreenSaver.xml)) or KDE's `org.freedesktop.ScreenSaver.ActiveChanged`. Do **not** use logind's `Lock` signal: it is a *request* to lock, and it does not fire when the shell locks by itself ([gnome-shell #741](https://gitlab.gnome.org/GNOME/gnome-shell/issues/741)). This feeds `user_away()` in the reminder and, optionally, `screens_did_sleep`.
3. **Input idle: one reader per desktop, picked at startup. All must return the same `Option<u64>` from `current_system_idle_ms()`.**
   - **GNOME (Mutter):** `org.gnome.Mutter.IdleMonitor` `/org/gnome/Mutter/IdleMonitor/Core` `GetIdletime() -> t` on the session bus. This is a real counter, a direct match for CoreGraphics ([BOINC/wayland-devel](https://lists.freedesktop.org/archives/wayland-devel/2019-June/040621.html), [Cinnamon commit showing the `t` signature](https://github.com/linuxmint/cinnamon-desktop/commit/d0f8c74a7864786cf04)). Mutter does **not** implement ext-idle-notify (Mutter 51 is listed as unsupported on [wayland.app](https://wayland.app/protocols/ext-idle-notify-v1)).
   - **KDE, sway, Hyprland, niri, COSMIC, labwc, river:** `ext-idle-notify-v1` **v2** `get_input_idle_notification`. It counts only real input and ignores idle inhibitors such as video players, the same meaning as CoreGraphics ([wayland.app](https://wayland.app/protocols/ext-idle-notify-v1): KWin 6.7, Sway 1.11, Hyprland 0.52, COSMIC, niri at v2; Wayfire is v1 only). The protocol fires *events*, not a counter. Turn it into a counter: register with a short timeout T (say 2 s); on `idled` store `last_input = now − T`; on `resumed` clear it; `idle_ms = now − last_input` (or 0 when not idled). That is accurate to within T, far below the minute-scale inactivity timeout and the reminder's 60 s window. Use a separate `wayland-client` connection on its own thread, not GTK's.
   - **X11 session:** XScreenSaver extension `XScreenSaverQueryInfo().idle` via `x11rb` (`screensaver` feature). Known caveat: DPMS can reset or subtract from the value once the monitor powers off ([xprintidle workaround](https://github.com/g0hl1n/xprintidle/blob/master/xprintidle.c), [fdo bug 6439](https://bugs.freedesktop.org/show_bug.cgi?id=6439)). Harmless here, because by then the inactivity pause has already fired.
   - **Do not use** `org.freedesktop.ScreenSaver.GetSessionIdleTime`: KWin errors or returns 0 on Wayland, and GNOME does not implement it ([KDE bug 449488](https://bugs.kde.org/show_bug.cgi?id=449488), [g-s-d MR 316](https://gitlab.gnome.org/GNOME/gnome-settings-daemon/-/merge_requests/316), [discuss.kde.org](https://discuss.kde.org/t/is-it-possible-to-detect-idle-time-on-plasma-6-widgets-on-wayland/30434)). Also do not use XScreenSaver through XWayland on a Wayland session: it only sees input sent to X clients (*uncertain*, follows from XWayland's design; I found no primary source).
4. **Display hotplug: GDK.** Tauri on Linux already runs GTK, so connect `GdkDisplay::monitor-added` / `monitor-removed` and emit `display_configuration_changed`. GDK hides the difference between `wl_output` and RandR, so this works on X11 and on every Wayland compositor with no D-Bus code. (Mutter's `DisplayConfig.MonitorsChanged` also exists, [Chromium client](https://chromium.googlesource.com/chromium/src.git/+/main/remoting/host/linux/gnome_display_config_dbus_client.cc), but is GNOME-only, so it is not needed.) The capture-recovery half belongs to the screen backend (PipeWire via the portal, §4.1): on a PipeWire stream error it should reuse the ADR 0021 `DisplayUnavailable` suspension.
5. **Native reminder banner: `notify-rust` (zbus backend) on `org.freedesktop.Notifications`.** Add actions `start` and `snooze-today`, handle the click on a dedicated thread (`show_and_wait_for_action` / `wait_for_action`, [notify-rust source](https://docs.rs/crate/notify-rust/4.11.3/source/src/xdg/zbus_rs.rs)), and `close()` the handle wherever `clear_reminder` withdraws the macOS banner. Check `GetCapabilities` for `"actions"`; if it is missing, post a plain banner and rely on the existing in-app row.

**Items that need no new mechanism**
- **App Nap:** no Linux equivalent, and none is used today. Nothing to do.
- **QoS:** already retired on macOS. Optional later: `SCHED_IDLE` or `nice` plus `ioprio_set(IOPRIO_CLASS_IDLE)` for OCR, embedding and transcription worker threads via `libc`. Not needed for parity.
- **Power / thermal:** the app deliberately reads none, and the duty-cycle self-throttle is portable. If a "pause indexing on battery" feature is ever wanted, use UPower `OnBattery` on the system bus ([UPower docs](https://upower.freedesktop.org/docs/UPower.html)). Linux has no standard thermal-state D-Bus API (*uncertain*). Skip both.
- **Launch at login:** a *new feature* on both OSes, not a port. When it is built, use `tauri-plugin-autostart`. On Linux it delegates to `auto_launch`, which writes an XDG autostart `.desktop` file, and it uses the AppImage path when `APPIMAGE` is set ([plugin source 2.7.0](https://docs.rs/crate/tauri-plugin-autostart/latest/source/src/lib.rs)). It **fails inside Flatpak**, because the sandbox cannot write `~/.config/autostart` ([crow-translate #428](https://invent.kde.org/office/crow-translate/-/issues/428)); Flatpak builds must call the Background portal `RequestBackground{autostart: true}` instead ([portal docs](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html)).
- **Low disk:** `fs2::available_space` uses `statvfs` on Unix, so it works as-is. **One real Linux bug to fix:** `volume_root` (`disk_space.rs:174-185`) only knows `/Volumes/<name>`. On Linux, a recordings folder on an unplugged USB drive (`/run/media/$USER/X/Mnema`) walks up to `/run/media/$USER` (tmpfs) or `/`, and the app would measure the wrong volume. ADR 0040 says an unknown reading should never block or wrongly pass. Fix by comparing `st_dev` with `MetadataExt::dev()` (stop the walk when the device changes) or by adding `/run/media/*/X`, `/media/*/X` and `/mnt/X` prefixes. The `st_dev` check is the correct one; the prefix list is the lazy one.
- **Render-idle gate:** the bug it works around is specific to WebKit on macOS. No WebKitGTK equivalent is known (*uncertain*, unmeasured). Leave the gate failing open, which it already does. Optionally emit `screens_did_sleep`/`wake` from lock state (item 2) to save CPU while locked.
- **Webview cache purge:** WebKitGTK has `webkit_website_data_manager_clear()` and memory-pressure settings (since 2.34) ([WebKitGTK docs](https://webkitgtk.org/reference/webkit2gtk/stable/class.WebsiteDataManager.html)). Whether WebKitGTK's footprint grows the same way is unmeasured. Port only if a two-day soak shows growth.
- **Deferred startup:** already portable.
- **Offscreen clamp:** on Wayland it cannot work and is not needed: xdg-shell gives clients no global position, so `setPosition` is ignored and the compositor keeps windows on live outputs. It applies only on X11, where the existing frontend code should just work once the event fires.

#### (d) Expected behavior and gaps

- Suspend, lock and hotplug reach full parity on GNOME and KDE. On wlroots, lock detection depends on the locker.
- Input idle reaches full parity on GNOME, KDE and most wlroots compositors. It degrades to `None` on Wayfire v1-only setups or exotic compositors; the existing `fallback_idle` (`native_capture_inactivity.rs:211`) plus screen and audio activity cover that. The reminder then never fires, which is today's behavior.
- "Display asleep" as part of "away": no portable signal on Linux. Mutter has a `PowerSaveMode` property (GNOME-only). Use lock state alone. Because the reminder also needs input in the last minute, an asleep display almost implies no recent input, so the gap is tiny.
- Flatpak: logind is on the **system** bus, so the manifest needs `--system-talk-name=org.freedesktop.login1`. Mutter idle needs `--talk-name=org.gnome.Mutter.IdleMonitor`. The portal `Inhibit.CreateMonitor` gives `screensaver-active` and session-end state but **no** PrepareForSleep ([Inhibit portal docs](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Inhibit.html)).

#### (e) Fallbacks (in order)

1. Idle: Mutter D-Bus or ext-idle-notify v2, then ext-idle-notify v1 `get_idle_notification` (inhibitors count as activity, so video playback keeps capture "active"), then XScreenSaver on a pure X11 session, then `None`.
2. Lock: `LockedHint`, then the `org.gnome.ScreenSaver` / `org.freedesktop.ScreenSaver` `ActiveChanged` signal, then the portal `Inhibit.CreateMonitor` `screensaver-active`, then "not away".
3. Suspend: logind delay inhibitor. With no logind or elogind, the screen backend's stream error plus the ADR 0021 `DisplayUnavailable` path. The tail segment may then be lost, as on macOS before the will-sleep fix.
4. Notifications: notify-rust with actions, then without actions, then the in-app row only.

#### (f) Risks

1. **The delay inhibitor must be released promptly and taken again after resume.** A missed release delays every suspend by `InhibitDelayMaxUSec`; a missed re-take loses the next tail segment. The GStreamer or ffmpeg writer must finalize within that budget, or the tail is truncated.
2. **Idle comes from three different backends.** Each has its own quirks (DPMS resets on X11; event-to-counter conversion on ext-idle-notify; a Mutter-only D-Bus name). Detecting which one to use from `XDG_SESSION_TYPE` and the globals on the Wayland connection can pick wrong on nested or unusual sessions. `None` must stay safe, and it is today.
3. **Lock detection is only as good as the locker.** COSMIC and possibly swaylock do not set `LockedHint`, so typing a password at the lock screen counts as "user active", and the reminder could fire behind the lock screen. The banner is only shown after unlock on most desktops, so the risk is low, but it is real.
- Also: the low-disk `volume_root` bug above is a silent wrong reading on removable drives.

#### (g) Effort

Overall **M**, roughly 600–900 lines of Rust including tests:

| Piece | Size | Estimate |
|---|---|---|
| logind suspend + lock | ~250 lines | S–M |
| idle readers (three backends) | ~300 lines | M |
| GDK hotplug | ~40 lines | S |
| notify-rust banner | ~120 lines | S |
| `st_dev` fix in `volume_root` | ~15 lines | S |
| App Nap, QoS, power | 0 | none |
| autostart, if wanted | plugin + portal branch | S |

#### (h) Parity cells

| Capability | macOS | GNOME Wayland | KDE Wayland | wlroots (sway/Hyprland) | X11 |
|---|---|---|---|---|---|
| Input idle | full (CGEventSource) | full (Mutter `GetIdletime`) | full (ext-idle-notify v2, KWin 6.7) | full (v2 on Sway 1.11 / Hyprland 0.52); Wayfire degraded (v1) | full with a quirk (XScreenSaver, DPMS reset) |
| Lock detection | full (`loginwindow` frontmost) | full (`LockedHint` + `org.gnome.ScreenSaver`) | full (`LockedHint`, ScreenSaver `ActiveChanged`) | degraded (depends on the locker; swaylock `LockedHint` unknown) | degraded (depends on the locker; xss-lock can set it) |
| Suspend/resume | full (NSWorkspace) | full (logind `PrepareForSleep` + delay inhibitor) | full | full (systemd/elogind) | full |
| Display hotplug | full (CG reconfiguration callback) | full (GDK monitor signals) | full | full | full (RandR via GDK) |
| Autostart (launch at login) | not built | full (XDG autostart; Flatpak needs the Background portal) | full | degraded (XDG autostart is honored only if the session runs it, e.g. via `dex`) | full on DE sessions |
| Notifications with actions | full (UNUserNotificationCenter) | full | full | degraded (mako/dunst actions need a menu or click config) | full on DE daemons; varies with a bare daemon |
| Power awareness | not used (deliberate) | not needed (UPower available) | not needed | not needed | not needed |

#### (i) Files to move or split

- `native_capture.rs` is 3648 lines, far over the 800-line rule. Move `start_system_wake_notifier` and the display-reconfiguration code (`:1400-1656`) into `native_capture/system_events/{mod.rs, macos.rs, linux.rs}` so the per-OS code sits in siblings rather than more `cfg` blocks.
- Delete the duplicate stub `native_capture/activity.rs:60-68` and call `system_idle::current_system_idle_ms()` directly. Otherwise a Linux idle reader is silently ignored by the inactivity policy, which is easy to miss.
- `native_capture_system_idle.rs` becomes the dispatcher: macOS body, or `linux_session::idle::current_ms()`.
- `native_capture/not_recording_reminder/linux_notification.rs` sits beside `macos_notification.rs`, and `user_away()` gets a Linux arm.
- `lifecycle.rs:567` `handle_system_will_sleep`: lift the `cfg` once the Linux screen backend exposes a stop-and-finalize call.
- `disk_space.rs:174` `volume_root`: add the `st_dev` floor.

#### (j) Degradation and UI

- Idle `None`: Settings → Inactivity should say "Input idle isn't available on this desktop; pausing uses screen and audio activity only". The idle debug panel already shows source availability (`get_idle_debug`, `activity.rs:204`).
- Reminder without native actions: banner text only, and clicking it opens the app; the in-app row keeps its buttons.
- No logind: one Warn log, "suspend finalize unavailable; tail segment may be lost on sleep".
- Update `SUPPORTS.md` rows 33, 34, 35, 40, 41, 42 and 54 with the Linux column.

#### (k) Hardware tests

- Suspend mid-segment (`systemctl suspend`) on GNOME, KDE and sway: the tail file opens, and the next segment starts on resume with the mic/audio continuity rules intact.
- `systemd-inhibit --list` shows Mnema's delay lock only while recording, and it is gone after `PrepareForSleep(true)`.
- Lock, then type the password: no reminder fires on GNOME or KDE. On sway, record what happens.
- Idle: leave the machine untouched for the inactivity timeout with a video playing; capture pauses (v2 input-only semantics). Repeat on Wayfire for the v1 fallback.
- Unplug and replug an external monitor; on X11, check the clamp.
- Recordings folder on a USB drive, then unplug: health reports "unknown" free space, not the tmpfs or root value.
- Reminder actions on GNOME Shell, Plasma, mako and dunst.
- Two-day soak on WebKitGTK to decide whether the cache purge is needed.

#### (l) Open questions

1. Is Flatpak a target? It decides whether we need the portal paths for Background and Inhibit, and the manifest D-Bus permissions.
2. Is launch-at-login wanted at all? It does not exist on macOS today.
3. Is the logind delay budget enough for the chosen Linux writer to finalize? Measure it.
4. Does swaylock or hyprlock set `LockedHint`? Test, or read their source.
5. Does WebKitGTK have any repaint-surface or memory leak that would justify porting the render-idle gate or the cache purge?

### 4.10 Permissions UX


**Answer in one line:** Linux has no permission database to send people to, so the "Open System Settings" world collapses to one real consent — the screen-sharing portal dialog, which Mnema must ask for once with "remember", then keep alive by saving a fresh restore token after every start — while microphone and system audio need no prompt at all and accessibility is dropped for v1.

**Recommendation:** keep the existing permission *shape* (`CapturePermissionState`, `get_capture_permissions`, `request_capture_permission`, `show_permission_helper`) and turn `show_permission_helper` into the per-platform "fix this permission" seam. On Linux it re-runs the portal picker for `screen` and never opens a window. Microphone and system audio report `granted`, and their rows hide themselves. Add one backend flag so the onboarding relaunch gate doesn't fire on Linux. Register the app ID `day.mnema` through the host portal Registry before any portal call, ask for `persist_mode = 2` with several monitors allowed, and store the rotated restore token after every session start.

Effort: **M**. The frontend reshape is S. The Rust portal-consent seam is M. Most of the cost is the hardware matrix across GNOME, KDE, wlroots and Hyprland.

---

#### (a) Current macOS implementation (file refs)

| Permission | How it's checked | How it's requested | Where "fix it" goes |
|---|---|---|---|
| Screen Recording | `CGPreflightScreenCaptureAccess()` plus an in-process "requested" latch → Granted/Denied/Unknown (`crates/capture-screen/src/lib.rs:3813-3829`). Non-macOS stub → `Unsupported` (`lib.rs:3985-3993`) | `CGRequestScreenCaptureAccess()` (`lib.rs:3831-3834`), via `request_capture_permission("screen")` (`apps/desktop/src-tauri/src/native_capture.rs:2514-2527`) | `x-apple.systempreferences:…Privacy_ScreenCapture` (`native_capture.rs:2549-2569`) |
| Microphone | `AVCaptureDevice authorizationStatus` (`crates/capture-microphone/src/lib.rs:2789-2799`). Non-macOS → `Unsupported` (`:2844-2852`) | `requestAccessForMediaType` with a 20 s wait (`:2801-2842`) | `…Privacy_Microphone` (`native_capture.rs:2559-2561`) |
| Screen & System Audio Recording | **Inferred** tri-state from one persisted evidence row: NotDetermined / AssumedWorking / PossiblyBlocked (`crates/capture-system-audio/src/permission.rs:70-84`, ADR 0052) | A throwaway tap (`native_capture.rs:2466-2481`, `capture-system-audio/src/tap.rs:251`) | Same ScreenCapture pane (`native_capture.rs:2556`) |
| Accessibility (Gecko URLs only) | `AXIsProcessTrusted` (`apps/desktop/src-tauri/src/native_capture_browser_url_ax.rs:62-92`); status command `get_browser_url_accessibility_status` (`native_capture.rs:2597-2633`); non-macOS → `trusted: false` (`:2603-2604`) | `AXIsProcessTrustedWithOptions` prompt (`native_capture.rs:2640-2646`) | `…Privacy_Accessibility` (`native_capture.rs:2651-2661`) |
| (Speech Recognition, Apple provider only) | `SFSpeechRecognizer::authorizationStatus` (`crates/audio-transcription/src/providers/apple_speech.rs:332`) | — | `…Privacy_SpeechRecognition` (`apps/desktop/src-tauri/src/audio_transcription_models.rs:350`) — Apple-only provider, hidden on Linux anyway |

Permission state is one aggregated response: `get_capture_permissions` (`native_capture.rs:2484-2505`), using the enum in `crates/capture-types/src/session.rs:5-…`.

**UI surfaces (all go through `show_permission_helper`):**
- **Rust helper window:** `apps/desktop/src-tauri/src/windows/permission_helper.rs:28-56` checks the kind against `KINDS` (`:20`), opens the pane (`:35-39`), and floats a 460×62 card bottom-centre (`:91-105`) that is ordered front without taking focus (`:107-119`, macOS only). It is registered at `lib.rs:870`, `windows.rs:91,313,337,361`, and the ACL at `capabilities/default.json:5`.
- **Helper page:** `apps/desktop/src/routes/permission-helper/+page.svelte:19-60` polls once a second and closes itself 1.4 s after the grant. Its copy says "Turn on Mnema in the {list} list" (`:66`).
- **Frontend wrapper:** `apps/desktop/src/lib/shell/permission-helper.ts:8-40`.
- **Onboarding:**
  - `routes/onboarding/screens/PermissionsScreen.svelte` (493 lines) shows screen, mic, system audio, plus accessibility only if a Gecko browser is installed (`:91-94`).
  - The **relaunch gate** is set whenever screen turns granted inside this process (`:186-200`). It is enforced by `lib/onboarding/permissions-gate.ts:44-51` and runs `request_app_relaunch` (`:207-210` → `windows.rs:1154`).
  - The system-audio level probe is at `:143-155`.
  - The request and settings dispatch is in `onboarding-permissions.svelte.ts:94-98`.
  - The Finale's "Open settings" button is at `FinaleScreen.svelte:226`.
- **Status-bar flag:** `apps/desktop/src/lib/shell/status-bar.ts:123-126` (`screenPermissionMissing`) and `:199-200` ("Screen Recording off · Open System Settings").
- **Settings → Capture rows:** `lib/settings/panels/capture/permission-rows.ts:12-24` (states granted/ask/denied/hidden; `unsupported` → hidden) and `PermissionRows.svelte:47-54`.
- **System-audio hint:** `lib/settings/state/system-audio-access.svelte.ts`.
- **Gecko access:** `lib/settings/state/gecko-url-access.svelte.ts`.

SUPPORTS.md:44 already notes that on non-macOS "the pane openers are `x-apple.systempreferences:` URLs, so the command errors there before any window shows."

#### (b) How it works on macOS (plain words)

- The OS keeps one switch per app per category.
- Mnema asks once. After a "no", macOS never asks again, which is why every denied path is "open System Settings, and here is a floating card telling you which switch to flip".
- Screen grants only take effect in a fresh process, hence the forced relaunch.
- System audio can't be queried, so it is guessed from whether any sound has arrived.
- Nothing appears on screen while recording, apart from macOS's own purple indicator.

#### (c) Recommended Linux onboarding flow, step by step

1. **Welcome** — unchanged.
2. **Screen (the one real consent).**
   - Copy: "Your desktop will ask which screens Mnema may record. Pick **every** screen you want recorded, and keep **Remember / Allow restoring** ticked."
   - Button "Choose screens" → `request_capture_permission("screen")` → portal `CreateSession` → `SelectSources(types=MONITOR, multiple=true, persist_mode=2, cursor_mode=embedded if offered)` → `Start`.
   - Save the returned `restore_token`, then close the session.
   - The persist semantics are in the spec: modes 0/1/2, the token is single-use, and the app must save the *new* token from each Start ([ScreenCast spec](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)).
   - **No relaunch step.**
   - On a cancel, show "Continue without screen recording", the same as macOS denial semantics, but the button re-asks instead of sending people to settings.
3. **Microphone: remove the prompt step.** Unsandboxed PipeWire apps get no prompt. Show it as granted, with only a device picker or level meter if one exists. Flatpak decides access at install time (the `--socket=pulseaudio`/PipeWire permission), not at runtime ([Flathub discussion](https://discourse.flathub.org/t/updating-the-microphone-access-warning-text-to-be-more-broad-for-clarity/8798)).
4. **System audio: remove the prompt step.** A PipeWire monitor needs no consent. Keep the level probe as a "sound is arriving" check, and add a one-line disclosure: "Linux doesn't ask apps for permission to hear system audio. Mnema records it only while this is on."
5. **Accessibility: remove for v1.** It only existed for Gecko URLs, and the Linux route (AT-SPI) has no permission at all (see (d)).
6. **Add "Recording indicator" (GNOME/KDE only):** "Your desktop shows a screen-sharing icon the whole time Mnema records. That's expected. Clicking **Stop** there pauses screen recording until you choose screens again."
7. **Add "Unlock keyring" (see §4.8):** if the Secret Service collection is locked, prompt here, not at first cloud-AI use.
8. **Add, conditionally, "Quick Recall shortcut" (see §4.11):** on Wayland, bind it through the GlobalShortcuts portal, which shows a desktop dialog on KDE, GNOME 48+ and Hyprland but not on wlroots ([status roundup](https://pypi.org/project/global-shortcut-portal/0.1.2/), [xdpw issue 240](https://github.com/emersion/xdg-desktop-portal-wlr/issues/240)). This is uncertain on GNOME 48 — verify.
9. **Add, conditionally, "GNOME extension" (see §4.2):** only if the active-window metadata slice picks a Shell extension.
10. **Finale** — unchanged, except its fix button re-asks.

**Detecting a revoked or invalid token** (there is no preflight API):
- **At start:** pass `restore_token`. If restore fails, the portal *shows the dialog* rather than returning an error ([spec](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html); GNOME falls back to the dialog when a monitor or window is missing — `find_monitor_by_string()` → `create_screen_cast_dialog()`, [xdp-gnome screencast.c](https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/main/src/screencast.c)).
- So for unattended starts (login, after wake): treat "Start took longer than ~2 s" or "returned no new token" as **NeedsReselection**. Cancel the request, set screen = `Denied`, and raise the status-bar flag, instead of leaving an unanswered dialog on screen.
- **Mid-session:** the user clicks Stop in the GNOME/KDE indicator → the portal `Session::Closed` signal or PipeWire stream error. Map this to "stopped by user", not to transient liveness.

#### (d) Gaps

- **There is no "is it still granted?" query.** State is a guess from whether a token exists and the last Start result. This is the same category of guess as ADR 0052 for system audio.
- **Unsandboxed app identity.** Persistent permissions are stored by `app_id` ([xdp-session-persistence.c L98-170](https://github.com/flatpak/xdg-desktop-portal/blob/main/desktop-portal/xdp-session-persistence.c)). Host apps get an ID inferred from the systemd unit, or an empty one. Use `org.freedesktop.host.portal.Registry.Register("day.mnema")`. It works at most once, must come before any portal call, and the ID must match the `.desktop` basename ([Registry doc](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.host.portal.Registry.html)). The xdg-desktop-portal version that added it is not stated — **verify the distro floor**. The `ashpd` crate wraps it ([ashpd registry.rs](https://docs.rs/crate/ashpd/0.13.10/source/src/registry.rs)).
- **New monitors are not captured.** The stream set is fixed when chosen. A dock with a different set of monitors makes GNOME restore fail, because it matches each monitor by a stored identity string ([screencast.c](https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/main/src/screencast.c)).
- **No API to pre-select "all monitors".** The user has to pick them all.
- **Microphone and system audio have no access control.** No microphone or audio portal has shipped. The discussion is still open, and in Jan 2025 a maintainer called it "not high priority" ([discussion #1142](https://github.com/flatpak/xdg-desktop-portal/discussions/1142)). PipeWire treats its clients as trusted ([gnome-software MR 1797](https://gitlab.gnome.org/GNOME/gnome-software/-/merge_requests/1797)).
- **Accessibility.** AT-SPI turns on when `org.a11y.Status.IsEnabled` is set on the accessibility bus, with no permission prompt ([AT-SPI2 wiki](https://wiki.freedesktop.org/www/Accessibility/AT-SPI2/)). Flipping it switches on accessibility in *every* app, at a performance cost. The often-quoted `gsettings … toolkit-accessibility` key is **unverified** as the switch.
- **Background and autostart.** The Background portal works only for sandboxed apps. Host apps use `~/.config/autostart/*.desktop` ([Background spec](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html); [Geary note](https://lists.gnome.org/archives/commits-list/2022-August/msg08845.html)). Mnema has no launch-at-login today (grep for `SMAppService`/autostart found nothing), so this is not an onboarding step yet.
- **The relaunch latch would misfire.** `PermissionsScreen.svelte:191,199` sets the relaunch gate on *any* in-process transition to granted. On Linux that forces a pointless relaunch.

#### (e) Fallbacks

1. **If persistence fails** (xdph with the box unticked, an old portal, or a broken backend): use `persist_mode = 1` (lasts while the app runs) and show the picker once per app launch. The status-bar flag becomes the "choose screens" entry point.
2. **On X11 sessions:** capture directly through X11 (see §4.1). No consent and no indicator, so the state is `granted` and the rows are hidden.
3. **wlroots/Sway:** `chooser_type=none` plus `output_name` in the xdpw config skips the chooser entirely ([xdpw man page](https://raw.githubusercontent.com/emersion/xdg-desktop-portal-wlr/master/xdg-desktop-portal-wlr.5.scd)). Document this as a setup tip, not an app action.
4. **Hyprland:** document `allow_token_by_default = true` in `xdph.conf`, which pre-ticks "Allow restore token" ([Hyprland wiki](https://wiki.hypr.land/Hypr-Ecosystem/xdg-desktop-portal-hyprland/)).

#### (f) Risks

1. **Silent no-recording after login or redock.** A failed restore shows a dialog nobody is there to answer. Without the timeout-and-flag rule in (c), the session hangs in "starting".
2. **Always-on indicator plus one-click Stop.** GNOME shows a warning-coloured screen-sharing indicator for portal sessions on Wayland ([gnome-shell commit](https://lists.gnome.org/archives/commits-list/2022-August/msg00179.html), [origin](https://lists.gnome.org/archives/commits-list/2018-July/msg03458.html)). KDE shows a "screen is being shared" tray icon ([KDE bug 452980](https://bugs.kde.org/show_bug.cgi?id=452980), [502019](https://bugs.kde.org/show_bug.cgi?id=502019)). Users will stop it, and Mnema must treat that as a deliberate user stop. Whether GNOME 49/50 lets users hide it is **unverified**.
3. **Token bookkeeping.**
   - Tokens are single-use, and each Start returns a new one. A crash between Start and the save, or the onboarding probe and the recorder both spending the same token, means a re-prompt.
   - The token must be written in the same step that consumes it, with one owner (the lifecycle seam).
   - On wlroots and xdph, persistence depends on config or a checkbox. There are reported xdph token-handling bugs ([OBS issue 13635](https://obs-versions.com/community/issues/13635), closed, fix unclear).
4. **Disclosure.** System audio and the microphone are recorded with no OS consent on Linux. That is a privacy-copy duty for Mnema, not something the OS enforces.

#### (g) Effort

- Frontend: **S**.
- Rust: **M**. That covers the portal seam (via `ashpd`), the token store in `app_settings`, Registry registration, the restore timeout and the user-stop mapping.
- QA matrix: **M–L**.
- Overall: **M**.

#### (h) Parity

| Capability | macOS | GNOME Wayland | KDE Wayland | wlroots (Sway / Hyprland) | X11 |
|---|---|---|---|---|---|
| Screen consent persistence | full — TCC, permanent | **degraded** — persist_mode 2 restore; lost when the monitor set changes ([screencast.c](https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/main/src/screencast.c)); xdp-gnome ≥42 ([RH bug](https://bugzilla.redhat.com/show_bug.cgi?id=2174946)) | **degraded** — restore tokens supported; past bugs fixed in 5.25 ([KDE 454128](https://bugs.kde.org/show_bug.cgi?id=454128)); behaviour across reboots unverified for screencast | **degraded** — xdpw ≥0.7 has restore tokens ([xdpw releases](https://www.github.com/emersion/xdg-desktop-portal-wlr/releases)), chooser depends on config; xdph only when the "Allow restore token" box is ticked | full — no consent exists |
| Microphone | full — TCC prompt | full, no prompt (unsandboxed); Flatpak: install-time permission | same as GNOME | same as GNOME | same as GNOME |
| System audio | degraded — inferred tri-state | full, no prompt (PipeWire monitor) | same | same | same (PulseAudio/PipeWire monitor) |
| Accessibility (Gecko URL) | full — AX permission | degraded — AT-SPI, no prompt, global performance cost; deferred | same | same | same |
| Recording indicator | OS purple dot | **forced** orange top-bar indicator with Stop | tray icon with stop menu | none from the portal (Waybar privacy module, if any) | none |

#### (i) Files to move or split

**Rust:**
- `windows/permission_helper.rs`: on Linux, `screen` → re-request the portal and show no window; `microphone`/`systemAudio`/`accessibility` → no-op `Ok`. Keep the macOS body behind `cfg`.
- `native_capture.rs:2484-2569` (permission commands, already ~2.6k lines past the 800 cap): split into `native_capture/permissions.rs` with `permissions/{macos,linux}.rs`.
- New `crates/capture-screen` Linux module `portal_consent.rs` for Registry, session, token and restore-timeout.
- `capture-microphone/src/lib.rs:2844-2852`: return `Granted` instead of `Unsupported` on Linux.
- `capture-system-audio` permission: return `Granted` on Linux.
- Add `grantNeedsRelaunch: bool` to `CapturePermissionsResponse` (`capture-types`) and mirror it in `lib/types/session.ts`.

**Frontend:**
- `PermissionsScreen.svelte:191,199`: gate the latch on `grantNeedsRelaunch`. The moments list at `:91-94` becomes backend-driven (hide mic, system audio and accessibility when they report `granted` with no prompt).
- `permission-helper.ts`: `PERMISSION_HELPER_LIST` copy becomes a platform label.
- `status-bar.ts:199-200` and `PermissionRows.svelte`: change the action label from "Open System Settings" to "Choose screens" (backend- or capability-provided).
- `permission-rows.ts:12-19`: unchanged. Its `denied` → "fix" path already routes through the helper seam.

#### (j) Degradation and UI copy

- **Screen row, denied or cancelled:** "Screen recording is off. **Choose screens**". Remove "Open System Settings".
- **Restore failed (unattended):** status flag "Screen recording paused — your screens changed. **Choose screens**".
- **User clicked Stop in the indicator:** "Screen recording stopped from the desktop's sharing menu. **Resume**".
- **Microphone and system audio rows:** hidden. Settings shows one note: "Linux doesn't ask apps for microphone or system-audio permission. Turn these off here to stop recording them."
- **wlroots or Hyprland, no persistence:** "Your desktop doesn't remember screen choices, so Mnema will ask each time it starts. [How to make it remember]".

#### (k) Hardware tests

1. GNOME 48/49 on Fedora or Ubuntu: grant → restart the app → reboot → unplug and replug a monitor → dock with different monitors. Each time, does a dialog appear? Does the token rotate? Can the indicator be hidden, and does Stop fire `Session::Closed`?
2. KDE Plasma 6.4+: the same matrix, plus the tray stop and the "Full workspace" option (**unverified that it exists**).
3. Sway (xdpw default chooser, then `chooser_type=none`) and Hyprland (xdph with the token box ticked and unticked).
4. Registry: with and without `Register("day.mnema")`, check that the token is still honoured and that another unsandboxed app can't reuse it.
5. Flatpak build: mic and PipeWire socket permissions, and the Background portal dialog.
6. Locked keyring at login with auto-login, and an unattended start after resume.

#### (l) Open questions

1. Should an unattended restore failure auto-cancel (recommended) or leave the dialog up for the user to find?
2. Ship Flatpak, native, or both? This decides microphone UX, the Background portal and how the Registry is needed.
3. Is GNOME's indicator acceptable for an always-on product, or does it push Mnema toward an X11-first or KDE-first launch?
4. Should system-audio and microphone recording on Linux get an explicit in-app consent dialog (like Deepgram's, ADR 0047), since the OS gives none?
5. What is the minimum xdg-desktop-portal version to support (Registry availability)?

---
Not checked: GNOME 49/50 indicator-hiding options, the KDE "Full workspace" source, the exact xdph token UX in late 2026, and what `toolkit-accessibility` actually does. All are marked unverified above. No builds were run.

### 4.11 Tauri shell on Linux

> **Superseded in one place:** this section says only AppImage can self-update. The pinned `tauri-plugin-updater` 2.10.1 also installs `.deb` and `.rpm` (`install_deb`/`install_rpm`, `updater.rs:1049-1059`). §4.12 is right, and deb/rpm get in-app updates with an admin password prompt.

Scope: WebKitGTK, windows (Quick Recall panel, permission helper, developer window, undecorated windows, titlebar), tray, global shortcuts, deep links, updater, time format, Cmd vs Ctrl, WebKit CSS/JS, frame-preview loading, webview memory.
Research date: 2026-11-01. Locked versions: tauri 2.10.3, tray-icon 0.21.3, webkit2gtk crate 2.0.2 (`apps/desktop/src-tauri/Cargo.lock`).

#### The answer in one sentence

Keep stock Tauri v2 on GTK3 + WebKitGTK 4.1, run as a **native Wayland** client, and accept that Linux gets a slightly plainer shell: a tray that only shows a menu, Quick Recall as an ordinary centred floating window, and global shortcuts through the desktop's GlobalShortcuts portal (with plain X11 hotkeys on X11 and a "bind this command yourself" escape hatch everywhere else).

Trade-off in plain words: we give up Spotlight-style "pop up without stealing focus" and pixel-exact placement on Wayland (Wayland does not let apps do that), in exchange for writing almost no new native code. The Mac experience is untouched.

**There is one hard compile blocker today** (see a.1). Everything else is "builds but behaves differently".

---

#### (a) Current macOS implementation, with file refs

##### a.1 Compile blocker on Linux
- `apps/desktop/src-tauri/src/windows.rs:499-503` calls `.title_bar_style(TitleBarStyle::Overlay).hidden_title(true)` with **no `#[cfg(target_os = "macos")]`**. In tauri 2.10.3 both builder methods are `#[cfg(target_os = "macos")]` (`~/.cargo/registry/src/.../tauri-2.10.3/src/webview/webview_window.rs:827-830, 864-867`). The desktop crate will not compile on Linux until this is gated. One-line fix.
- Every other Cocoa call site is already gated: `windows.rs:56-58, 656, 696-910, 1068-1136, 1244`, `windows/quick_recall_frame.rs:47-87`, `windows/permission_helper.rs:107-119`, `webview_cache.rs:44-52`, `clock_format.rs:25-31`, `status_bar.rs:641-692`, `lib.rs:500-535, 997`.

##### a.2 Window catalogue
`windows.rs:194-329` (`AppWindowConfig`): Main is decorated with an overlay titlebar (`:224-236`); Onboarding, CLI Access, Developer, Update, Quick Recall and Permission Helper are all `decorations:false, transparent:true, shadow:true`, with a native rounded corner applied by `apply_macos_rounded_content_view` (`windows.rs:656-688`, called at `:507-510`, `:946-953`).
- Capability ACL lists all seven labels: `apps/desktop/src-tauri/capabilities/default.json:5` — platform-neutral, nothing to change.
- `tauri.conf.json:13` `macOSPrivateApi: true` and `Cargo.toml:36` feature `macos-private-api` — harmless no-ops on Linux.

##### a.3 Quick Recall
- Built hidden, `always_on_top(true)`, `skip_taskbar(true)` (`windows.rs:927-956`); on macOS reclassed into a non-activating `NSPanel` (`windows.rs:690-910`).
- Summon: `quick_recall_frame::anchor_quick_recall` then show, then macOS key / non-mac `set_focus()` (`windows.rs:958-980`). Dismiss: order-out / non-mac `hide()` (`windows.rs:982-1001`). Blur-dismiss with a grace window for the spurious first blur (`windows.rs:31`, `:1003+`).
- Sizing: page measures itself and calls `set_quick_recall_frame(height)`; anchor = centred, top edge 15 % down the work area (`quick_recall_frame.rs:12-31`). macOS does one `setFrame:display:` (`:47-87`); non-mac does `set_size` then `set_position` (`:89-117`).
- Re-focus after mount: `focus_quick_recall_window` (`windows.rs:1440-1450`).

##### a.4 Permission helper
- `windows/permission_helper.rs:28-56`: opens the System Settings pane, then floats a 460×62 card bottom-centre (`:99-110`), ordered front without focus (`:107-124`).
- The pane openers are `x-apple.systempreferences:` URLs (`native_capture.rs:2550-2571`), so off-mac the command errors before a window shows (SUPPORTS.md:44). On Linux the whole helper is the wrong idea — permissions there are per-request portal dialogs (§4.10).

##### a.5 Developer window
- `windows.rs:257-273`, opened with ⌘D (`keyboard_bindings.rs:354`); mini mode uses `setAlwaysOnTop`/`setSize`/`setMinSize` from the frontend (capability `default.json:11-12`).
- Its strip is `WindowStrip.svelte`: three **drawn** traffic lights, the first is the close button, whole strip is `data-tauri-drag-region` (`apps/desktop/src/lib/components/WindowStrip.svelte:15-41`).

##### a.6 Main titlebar
- App draws its own titlebar with a hard-coded **78 px left inset reserved for the macOS traffic lights** (`apps/desktop/src/lib/styles/shell.css:10-17`), drag regions in `lib/shell/Titlebar.svelte:121-167`, `lib/settings/SettingsRail.svelte:184`.

##### a.7 Tray
- `status_bar.rs:435-454`: `TrayIconBuilder` with a template PNG (`:43-52`, `icons/tray-template.png` 32×32 RGBA), `icon_as_template(true)`, `show_menu_on_left_click(true)`, menu-only interaction via `on_menu_event`. Refresh rebuilds the menu and tooltip (`:470-490`).
- "Exclude Current App" is macOS-only; off-mac shows an info dialog (`status_bar.rs:685-692`).

##### a.8 Global shortcuts
- `tauri-plugin-global-shortcut` (`Cargo.toml:41`, `lib.rs:608-612`). Defaults `CommandOrControl+Alt+R/P/M/Space` (`keyboard_bindings.rs:14-17`). Register loop is best-effort and pushes a warning notification on failure (`keyboard_bindings.rs:1040-1063, 1150-1172`).

##### a.9 Deep links
- `tauri-plugin-deep-link` 2.4.9 + `tauri-plugin-single-instance` with the `deep-link` feature (`Cargo.toml:89-90`), scheme `mnema` (`tauri.conf.json:22-27`).
- Setup: `on_open_url` → `dispatch_deep_link`; `get_current()` for cold start; `register_all()` (`lib.rs:891-902`). Five payloads routed by host+path: license activate/claim/renewed, MCP OAuth callback, broker open (`lib.rs:283-345`).
- Single-instance callback ignores `args` and just opens the main window (`lib.rs:573-575`).

##### a.10 Updater
- `tauri-plugin-updater` (`lib.rs:607`), endpoint `https://release.mnema.day/stable/latest.json`, `createUpdaterArtifacts: true`, `targets: "all"` (`tauri.conf.json:28-39`). `updater.check()` at `app_updates.rs:777`. SUPPORTS.md:62: release workflow ships Apple Silicon macOS only.

##### a.11 Time format
- `clock_format.rs:24-31`: macOS reads the hour cycle from `NSDateFormatter`; others return `None` → frontend uses the webview's `Intl`.

##### a.12 Keyboard: Cmd vs Ctrl
- Platform detection is by user agent: `lib/keyboard.ts:96-106` → WebKitGTK UA says Linux → `"other"`.
- `"other"` matches primary when **exactly one** of Meta/Ctrl is down (`keyboard.ts:113-116`) and labels it "Ctrl" (`:137-139`). So bound shortcuts already work and display correctly on Linux.
- **But ~40 hard-coded `⌘` glyphs** in hint text/kbd labels ignore the platform: e.g. `lib/quick-recall/Field.svelte:40-41`, `QuickLookSide.svelte:94-98,156`, `detailStore.svelte.ts:359-367`, `BestMoment.svelte:235`, `chat/ChatList.svelte:88`, `insights/Overview.svelte:692`, `insights/SubjectDetail.svelte:304`, `insights/Context.svelte:269,355`, `debug/DevStrip.svelte:232`, `settings/state/keyboard.svelte.ts:112`.
- **One handler is Meta-only**: DevStrip's copy-diagnostics `event.metaKey && event.shiftKey` (`lib/debug/DevStrip.svelte:114`) — on Linux that is Super+Shift+C, which GNOME/KDE usually grab. Broken on Linux.

##### a.13 WebKit CSS/JS
- `backdrop-filter` in 10 files incl. the always-visible main titlebar (`shell.css:19`) and `kit.css` (6 uses). `-webkit-` prefixes (scrollbar, line-clamp, mask-image, user-select, backdrop-filter) — all supported by WebKitGTK, same engine family.

##### a.14 Frame previews / media
- Asset protocol on, scope empty and widened at runtime with `allow_directory`/`allow_file` (`tauri.conf.json:16-19`, `app_infra/frame_preview.rs:904-952`, `native_capture.rs:657-658, 2900`); frontend uses `convertFileSrc` (`ChatTurn.svelte:95`, `FrameDetailModal.svelte:122`, `Overview.svelte:226`, `charts/Timeline.svelte:106`, `VoiceEnrollment.svelte:56`). Frames via `invoke` + `createImageBitmap` (`FrameDetailModal.svelte:207`, `insights/receipt-frames.ts`). Voice-enrollment playback is an `<audio>` element on an m4a/AAC clip (`VoiceEnrollment.svelte:126-134`).

##### a.15 Webview memory
- `webview_cache.rs`: on window blur, at most every 60 s, purges WKWebView's memory cache (decoded frame previews grew WebContent to 2.2 GB). Off-mac no-op (`:30-50`).
- `windows.rs:895-925`: hides all WKWebViews while displays sleep (stranded IOSurfaces); off-mac no-op.

##### a.16 Exit/lifecycle
- macOS `_exit(0)` after graceful exit; others `app.exit(0)` (`windows.rs:1230-1250`). `RunEvent::Reopen` is macOS-only (`lib.rs:997-1004`) — on Linux there is no dock click, so the tray and single-instance relaunch are the only "bring it back" paths (single-instance already does it).

---

#### (b) How it works on macOS (short)
Quick Recall and the permission helper are NSPanels: they float above everything on every Space, Quick Recall takes keyboard focus without activating Mnema, the helper never takes focus. The app places them exactly. The tray is a template icon that recolours itself for light/dark menu bars and opens a menu on any click. Global shortcuts are Carbon hotkeys. Deep links come through LaunchServices to the running process. Updates swap the `.app`.

---

#### (c) Recommended Linux approach

**One recommendation: stock Tauri GTK3/WebKitGTK 4.1, native Wayland, a thin `linux` sibling for the four places that matter, everything else via the existing non-mac fallbacks.**

1. **Make it compile:** gate `windows.rs:499-503` with `#[cfg(target_os = "macos")]`. (S)
2. **Webview:** require WebKitGTK 4.1 (Tauri v2's only Linux webview; prerequisites list `libwebkit2gtk-4.1-dev` / `webkit2gtk-4.1` / `webkit2gtk4.1-devel` and `libayatana-appindicator3-dev` — https://v2.tauri.app/start/prerequisites/). Do **not** set `WEBKIT_DISABLE_DMABUF_RENDERER` unconditionally — Tauri's guide says that "disables a faster path for everyone"; apply only on detected NVIDIA after `__NV_DISABLE_EXPLICIT_SYNC=1` is tried (https://v2.tauri.app/develop/debug/linux-graphics/). Ship a documented env-var override and log the renderer at startup. GTK4/WebKitGTK 6 is **not** shipped by Tauri; a GTK4 port is an open help-wanted issue (https://goodfirstissue.org/tauri-apps/tauri/issues/12562) — don't wait for it.
3. **Global shortcuts:** keep `tauri-plugin-global-shortcut` on X11 (its `global-hotkey` backend is "Linux (X11 Only)" — https://github.com/tauri-apps/global-hotkey). On Wayland, add a small Linux backend over the XDG **GlobalShortcuts portal** using the `ashpd` crate (`GlobalShortcuts::new → create_session → bind_shortcuts`, https://docs.rs/crate/ashpd/latest/source/src/desktop/global_shortcuts.rs). GNOME 48+ and KDE implement it (https://release.gnome.org/48/developers/, https://dec05eba.com/2024/03/29/wayland-global-hotkeys-shortcut-is-mostly-useless/); wlroots' portal does not (https://github.com/emersion/xdg-desktop-portal-wlr/issues/240). Portal shortcuts are *proposed* — the desktop shows its own dialog and the user may pick different keys, so the Settings UI must show "assigned by your desktop" instead of an editable accelerator.
   - **Universal fallback:** make every global action reachable as a deep link/CLI arg (`mnema://quick-recall`, `mnema://toggle-recording`) routed through single-instance, and tell users to bind it in their DE's custom-shortcut settings (works on Sway/Hyprland/i3/anything). Needs `lib.rs:573-575` to stop ignoring `args` — though with the `deep-link` feature the plugin already forwards URLs to `on_open_url` (https://v2.tauri.app/plugin/deep-linking/), so only plain CLI flags need new code.
4. **Quick Recall:** use the existing non-mac path (`windows.rs:927-1001`): undecorated, always-on-top *requested*, `skip_taskbar`, `set_focus()` on summon, `hide()` on blur. On Wayland, `set_position` is ignored and always-on-top is not honoured (no protocol for either — https://github.com/tauri-apps/tauri/issues/3117, https://gitlab.freedesktop.org/wayland/wayland/-/issues/183), so the compositor places it (GNOME centres new dialogs-ish windows; KDE uses its placement policy). Make the bar grow downward by sizing only, and skip the anchor math when on Wayland. Later, optional: layer-shell placement on KDE/wlroots via `gtk-layer-shell` (GTK3, maintenance mode; not supported on GNOME — https://github.com/wmww/gtk-layer-shell).
5. **Permission helper:** don't open it on Linux. Return `unsupported` from `show_permission_helper` (`permission_helper.rs:28`) and let the capture subsystem's portal flow own permission UX.
6. **Tray:** keep `TrayIconBuilder` + menu. On Linux the click event is never emitted and `show_menu_on_left_click` is unsupported; the icon may be invisible without a menu (https://v2.tauri.app/reference/javascript/api/namespacetray/) — Mnema is already menu-only, so it fits. Replace the template icon with a **light/dark-safe** Linux icon (template recolouring is macOS-only; a black glyph disappears on dark panels — *unverified what colour `tray-template.png` is*). GNOME needs the AppIndicator extension (https://github.com/ubuntu/gnome-shell-extension-appindicator); make the app fully usable without a tray (main window reachable by relaunch, already wired via single-instance).
7. **Titlebar/chrome:** on Linux, drop the 78 px traffic-light inset (`shell.css:17`) behind a platform class, keep main window `decorations:true` (native GTK/SSD titlebar — accept a double bar, or set `decorations:false` and reuse `WindowStrip`'s drawn close button). For undecorated windows: `shadow` is "Linux: Unsupported" (tauri-2.10.3 `webview_window.rs:713-716`) and there is no native corner rounding, so draw the rounded corner + shadow in CSS inside the transparent window.
8. **Deep links:** keep config; `register_all()` writes the scheme handler at runtime because "by default the deep link is only registered when your app is installed"; for AppImage the link "is invalidated" if the file moves (https://v2.tauri.app/plugin/deep-linking/). Ship `.deb`/`.rpm` with `MimeType=x-scheme-handler/mnema;` in the `.desktop` file (Tauri's bundler does this from the deep-link config — *flag: verify in a built .deb*). Single-instance uses D-Bus on Linux and needs explicit D-Bus names under Flatpak/Snap (https://v2.tauri.app/plugin/single-instance/).
9. **Updater:** only AppImage is updatable by `tauri-plugin-updater` (https://v2.tauri.app/plugin/updater/). For `.deb`/`.rpm`, run the check as "notify only" and link to the download/APT repo. Add an `install_kind` switch in `app_updates.rs` near `:777`.
10. **Time format:** on GNOME read `org.gnome.desktop.interface clock-format` (`24h`/`12h`) in `clock_format.rs` under `cfg(target_os="linux")`; otherwise keep `None` (Intl/locale). (S; *KDE source of truth unverified* — likely locale `LC_TIME`.)
11. **Keyboard labels:** replace hard-coded `⌘` strings with `formatShortcut(..., platform)` (already exists, `keyboard.ts:133-140`); fix `DevStrip.svelte:114` to use `matchShortcut`. Default `CommandOrControl+Alt+Space` maps to Ctrl+Alt+Space — fine on Linux.
12. **Webview memory:** port the blur purge with `webkit_website_data_manager_clear(..., WEBKIT_WEBSITE_DATA_MEMORY_CACHE)` (flag exists since 2.16 — https://webkitgtk.org/reference/webkit2gtk/stable/flags.WebsiteDataTypes.html) via `with_webview` in `webview_cache.rs`. Measure first; WebKitGTK may not show the IOSurface leak at all. Skip the display-sleep hide unless measured.
13. **Media:** WebKitGTK plays `<audio>`/`<video>` through GStreamer; AAC (`.m4a`) needs `gst-plugins-good` + `gst-libav` (https://wails.io/fr/docs/guides/linux, https://forum.manjaro.org/t/webkit2gtk-doesnt-work-well-with-m3u8-streams/168103). Add them as package dependencies (deb/rpm) and verify the AppImage bundles them.

---

#### (d) Expected behaviour and gap

| Area | Linux behaviour after (c) | Gap vs macOS |
|---|---|---|
| Quick Recall | Summons, focuses its search field, hides on blur | Activates Mnema (steals focus); Wayland: compositor picks position, may not stay on top or follow to other workspaces; size change may jump (`quick_recall_frame.rs:89-91`) |
| Permission helper | Not shown | None needed — Linux uses portal prompts |
| Developer window | Opens, pins on X11 | Wayland: "keep on top" is a no-op; mini mode works |
| Tray | Menu on right-click (and left on KDE/most SNI hosts) | No click events; GNOME needs an extension; no auto light/dark icon |
| Global shortcuts | X11: same as mac. GNOME 48+/KDE Wayland: desktop dialog to approve keys | wlroots: none — user binds `mnema://…` themselves |
| Deep links | Work for deb/rpm; AppImage only after `register_all` and breaks if moved | Small |
| Updater | AppImage self-updates; deb/rpm notify-only | No silent update for deb/rpm |
| Rendering | Same pages, same CSS | NVIDIA blank-window risk; `backdrop-filter` may be slow on software paths |
| Time format | GNOME setting honoured; else locale | KDE user override maybe missed |

---

#### (e) Fallbacks
1. **Force XWayland** (`GDK_BACKEND=x11` in `main()` before GTK init): restores `set_position`, always-on-top and X11 hotkeys — **but X11 hotkeys under XWayland only fire while an X11 window has focus**, unless KDE's "Legacy X11 apps can read keystrokes" is turned on (off by default, a security cost) (https://github.com/bk138/gromit-mpx/issues/121, https://lxr.kde.org/source/plasma/kwin/src/kcms/xwayland/ui/main.qml). Good for placement, useless for shortcuts on GNOME. Offer as an opt-in env knob, not the default.
2. **Layer-shell Quick Recall** on KDE/wlroots (`gtk-layer-shell` on the GTK window from `window.gtk_window()`): true overlay + exact placement + keyboard grab. Not on GNOME. M effort.
3. **Disable Quick Recall global summon** and rely on tray "Quick Recall" + user-bound `mnema://quick-recall`.
4. **Tray-less mode:** if no StatusNotifierWatcher on D-Bus, keep main window as the control surface (don't hide-to-tray on close).
5. **Renderer:** auto-apply `WEBKIT_DISABLE_DMABUF_RENDERER=1` only when NVIDIA proprietary is detected and the user opts in; last resort `WEBKIT_DISABLE_COMPOSITING_MODE=1`.

#### (f) Risks
1. **Global shortcuts on Wayland are fragmented**: portal on GNOME 48+/KDE only, user-chosen keys, async approval dialog, older portals expect shortcuts in `CreateSession` instead of `BindShortcuts` (https://bugs.kde.org/show_bug.cgi?id=474741). A user-reported fork of the plugin crashed the portal on `unregister_all()` (https://stardance.hackclub.com/projects/16334/devlogs/13944 — *anecdotal*). New native code with no upstream Tauri support.
2. **WebKitGTK GPU/driver bugs** (blank windows, Error 71 on NVIDIA) and generally slower WebKitGTK (https://github.com/tauri-apps/wry/issues/890). Frame-heavy timeline/scrubbing and 13 `backdrop-filter` uses may stutter on software compositing — *unmeasured*.
3. **Quick Recall loses its "launcher" feel on Wayland**: can't position, can't force on-top, focus-steal semantics differ per compositor; blur-dismiss timing (`windows.rs:31`, grace window) was tuned for AppKit and may misfire under GTK focus events.
4. AppImage + WebKitGTK + GStreamer bundling is fragile (mesa/EGL mismatch, missing AAC decoder) — *flag: not verified on a real AppImage*.
5. Transparent undecorated windows show black corners on X11 without a compositor; CSS shadow eats into the window rect.

#### (g) Effort
- Compile fix + chrome/labels/permission-helper off + tray icon swap + updater notify-only + clock (GNOME): **S** (~2–3 days).
- GlobalShortcuts portal backend + Settings UI for "assigned by desktop" + `mnema://` action links: **M** (~1–1.5 weeks).
- Quick Recall Wayland polish (+ optional layer-shell on KDE/wlroots): **M**.
- Webview memory purge port + measurement: **S**.
- **Total shell + frontend: M (≈3 weeks one person)**, excluding packaging/CI.

#### (h) Parity cells

| Feature | macOS | GNOME Wayland | KDE Wayland | wlroots (Sway/Hyprland) | X11 (any WM) |
|---|---|---|---|---|---|
| Tray | full | degraded — needs AppIndicator extension (preinstalled on Ubuntu, *not* Fedora — *flag*); menu only | degraded — SNI native; menu only, no click events | degraded — needs waybar/tray host; menu only | degraded — needs SNI/XEmbed host; menu only |
| Global shortcuts | full | degraded — GNOME 48+ portal, user approves/rebinds keys; <48 impossible (DIY binding) | degraded — portal, user approves keys | impossible natively (wlr portal lacks it; Hyprland has own protocol) → DIY `mnema://` binding | full (X11 grab) |
| Quick Recall panel | full (non-activating NSPanel) | degraded — normal focused window, compositor placement, on-top not guaranteed | degraded — same; layer-shell could make it full-ish | degraded — same; layer-shell could make it full-ish | degraded — positioned + on-top, but activates app (no non-activating mode) |
| Window positioning | full | impossible (compositor decides) | impossible without layer-shell / KWin script | impossible without layer-shell | full |
| Deep links | full | full for deb/rpm; AppImage degraded (path-bound) | same | same (needs `xdg-open` + mimeapps) | same |
| Updater | full | degraded — AppImage only; deb/rpm notify-only | same | same | same |
| Webview rendering | full | degraded — WebKitGTK; NVIDIA DMABUF risk; slower filters | same | same | same; transparency needs a compositor |

#### (i) Files to move/split
- `windows.rs` is **2006 lines** (over the 800-line rule). Natural split for this port: move the macOS NSPanel block (`windows.rs:690-925`) to `windows/macos_panel.rs`, dock/terminate helpers (`:1068-1136, 1244-1250`) to `windows/macos_app.rs`, and put Linux bits (Wayland detection, no-op permission helper) in `windows/linux.rs`. Leave platform-neutral config/ACL/tests in `windows.rs`.
- `keyboard_bindings.rs` (1420 lines): put the portal backend in a new `keyboard_bindings/linux_portal.rs`; the registration loop at `:1040-1063` dispatches to it.
- `status_bar.rs` (1067): add `tray_icon()` per-platform (`:43-52`); no split needed for the port.
- `app_updates.rs` (1641): add an install-kind check; splitting is separate work.
- Frontend: add `data-platform` on `<html>` from `detectKeyboardPlatform()` in `routes/+layout.svelte`, use it in `shell.css:17` and `WindowStrip.svelte`.

#### (j) Degradation + UI
- Settings → Shortcuts on Wayland: show "Your desktop manages these shortcuts" + button "Choose keys…" (re-trigger portal bind); on wlroots/no-portal show the `mnema://…` commands to copy into the compositor config.
- Quick Recall: no UI change; document that it opens as a normal window.
- Tray missing: one-time notice in the main window "Install the AppIndicator extension to see Mnema in the top bar" when no StatusNotifierWatcher is on the session bus.
- Updates (deb/rpm): "Version X is available — update with your package manager / download" instead of Install button.
- Developer window: hide "Keep on top" toggle on Wayland (or label "may be ignored by your desktop").
- Permission helper: never shown; every "Open System Settings" button must be hidden on Linux (36 `x-apple`/"System Settings" references in `apps/desktop/src` — §4.10 has the replacement copy).
- SUPPORTS.md rows to update: Tauri shell, Quick Recall, Permission helper, Developer window, Status bar, Time format, Global shortcuts, Release/updater, MCP OAuth (deep link).

#### (k) What to test
1. `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` on Linux (after the cfg fix) — expect next failures from capture crates, not the shell.
2. Matrix: Ubuntu 24.04/26.04 GNOME Wayland, Fedora GNOME Wayland, KDE Plasma 6 Wayland, Sway, an X11 session; Intel/AMD + NVIDIA proprietary.
3. Each window opens, drags (drag regions), closes, transparent corners look right; Quick Recall summon → focus in search field → blur hides → re-summon keeps Ask AI session.
4. Tray menu items all work; icon visible on light and dark panels.
5. Global shortcuts: register, fire while another (native Wayland) app is focused, survive Settings rebinding and app restart, failure path shows the warning (`keyboard_bindings.rs:1150`).
6. Deep links from a browser: `mnema://license/activate?key=…`, MCP OAuth callback, broker open — both cold start and running instance (D-Bus single-instance).
7. AppImage update round-trip; deb/rpm notify-only.
8. Timeline scrubbing for 30 min: WebKitWebProcess RSS growth (to decide if the purge port is needed); frame previews load via `asset://` URLs.
9. Voice-enrollment `<audio>` plays m4a on a clean install.
10. Time format "System" on GNOME with 12h/24h toggled.

#### (l) Open questions
1. Which distros/DEs are in scope for v1 Linux? (Recommend: GNOME 48+ and KDE 6 Wayland; X11 best-effort; wlroots "DIY shortcuts".)
2. Package formats: AppImage (self-update) vs deb/rpm (proper desktop integration, GStreamer deps) vs Flatpak (portal-first, but sandbox breaks single-instance D-Bus and capture). Determines updater story.
3. Is the Quick Recall launcher a must-have on Linux, or acceptable as "normal window via shortcut"? If must-have, budget layer-shell (not GNOME).
4. Is it OK that portal shortcuts may end up bound to keys the user chose, not ours?
5. Does WebKitGTK's memory grow like WKWebView under scrubbing? (Measure before porting `webview_cache.rs`.)
6. Colour of `icons/tray-template.png` (unverified — no image tool used); need a Linux-specific icon either way.

### 4.12 Packaging and distribution

> **Updated 2026-11-01 after the owner decisions (§10):** the build floor is Ubuntu 24.04, x86_64 only, and the CLI owns the `mnema` command. The text below has been changed to match.

**Answer in one sentence:** ship a **.deb + .rpm built on Ubuntu 24.04 (x86_64)**, downloaded from the existing R2 bucket and kept current by the **Tauri updater Mnema already uses** (the pinned plugin 2.10.1 can already install deb/rpm updates). Add an **AUR `mnema-bin`** that repackages the .deb as the second channel. Keep **AppImage** as a fallback only, and **defer Flatpak and Snap**.

Why this one: deb/rpm run as ordinary host processes, so every Linux capability Mnema needs works without sandbox holes: PipeWire portals, AT-SPI, browser native-messaging hosts, Secret Service, the CLI on `PATH` and the Unix socket. Keeping the in-app updater also keeps the paid **Update Window** check, which today exists only in the updater (`apps/desktop/src-tauri/src/app_updates.rs:417-427`). A public apt/dnf repo or Flathub would hand every user every build and skip that check.

---

#### (a) Current macOS implementation (file refs)

| Piece | Where |
|---|---|
| Bundle config: `targets: "all"`, `createUpdaterArtifacts: true`, `externalBin: ["binaries/mnema-cli"]`, macOS-only entitlements/Info.plist, no `linux` section | `apps/desktop/src-tauri/tauri.conf.json:37-55` |
| Updater: minisign pubkey + stable endpoint `https://release.mnema.day/stable/latest.json` | `tauri.conf.json:29-34` |
| Preview channel endpoint | `apps/desktop/src-tauri/src/app_updates.rs:21` |
| Update Window gate (only `Licensed` builds dated after `update_through` are declined; "never a hard lock") | `app_updates.rs:408-445` |
| Incompatible-update copy says "this Mac" | `app_updates.rs:535` |
| Sidecar build (target-triple suffix, handles `.exe`, nothing Linux-specific) | `scripts/prepare-mnema-cli-sidecar.sh:31-104` |
| Release staging (DMG / `.tar.gz` / `.sig` / `latest.json`, uses `shasum`) | `scripts/stage-macos-release-artifacts.sh:16-44` |
| Release workflows (macOS only) | `.github/workflows/macos-release.yml`, `macos-release-promote.yml` |
| Release process, R2 bucket layout, channels | `docs/release-process.md:9-33` |
| ADRs: tauri-action builds draft releases; opt-in preview channel | `docs/adr/0017-*.md`, `docs/adr/0018-*.md` |
| Machine fingerprint: macOS `gethostuuid(2)`; **non-macOS returns an error** | `crates/app-infra/src/machine_id.rs:14-31, 47-52` |
| Device label: `sysctl hw.model`; non-macOS returns an error | `machine_id.rs:58-104, 106-111` |
| Activation: fingerprint error → "skipped, treat as Activated" | `apps/desktop/src-tauri/src/licensing/activation.rs:175-184` |
| Trial: no fingerprint → **no trial issued and no stamp written** | `apps/desktop/src-tauri/src/licensing/trial.rs:166-182` |
| Status: `machine_supported=false` turns every non-terminal state into `Activated` | `licensing.rs:231-266`, `licensing/adapter.rs:177-185`, test `adapter.rs:505` |
| licensegate hashing: plain salted SHA-256 of any string the caller supplies | `~/.cargo/registry/.../licensegate-0.2.0/src/lib.rs:41-53, 158-168` |
| Broker socket: `<config_dir>/cli-access.sock`, parent dir set to 0700, Linux config dir = `$XDG_CONFIG_HOME` or `~/.config`, plus the identifier | `apps/desktop/src-tauri/src/broker_authorization_channel.rs:177-200, 1009-1038` |
| CLI socket path (mirrors the app) | `crates/cli/src/main.rs:1180-1185, 1208-1224` |
| CLI starts the app on Linux with `xdg-open mnema` | `crates/cli/src/main.rs:1187-1205` (line 1199) |
| CLI "install to PATH" = **symlink** `~/.local/bin/mnema` → bundled sidecar next to `current_exe()` | `apps/desktop/src-tauri/src/app_infra.rs:57-58, 4322-4371, 4470-4501` |
| The GUI binary is also called `mnema` (Cargo package name; no `mainBinaryName`) | `apps/desktop/src-tauri/Cargo.toml:2`, `crates/cli/Cargo.toml:7-9` |
| Deep-link scheme `mnema://` + `register_all()` at startup | `tauri.conf.json:22-26`, `lib.rs:902` |
| SQLCipher on Linux dynamically links the system `libcrypto` (`bundled-sqlcipher` without vendored OpenSSL) | `crates/app-infra/Cargo.toml:37`; libsqlite3-sys `build.rs` "dylib=crypto" branch |
| Other native libs: `ort =2.0.0-rc.12` (ONNX Runtime), whisper-rs, tesseract-rs, ocr-rs (paddle), speakrs/OpenBLAS (macOS CoreML) | `crates/audio-transcription/Cargo.toml:23,37-40`, `crates/ocr/Cargo.toml:8-19`, desktop `Cargo.toml:18-27` |
| Project license AGPL-3.0; package description "A Tauri App", authors "you" | `LICENSE:1`, `apps/desktop/src-tauri/Cargo.toml:4-5` |
| SUPPORTS rows: Licensing (macOS-only fingerprint), Broker Channel `[~]` Linux, CLI sidecar `[~]`, Release `[ ]` | `SUPPORTS.md:59-62, 236, 282-289` |

#### (b) How it works on macOS

`tauri-action` builds a DMG and an `.app.tar.gz`, and signs the `.tar.gz` with the minisign updater key (`.sig`). It also writes `latest.json`. The promote workflow copies all of these to R2 and rewrites `latest.json` to point at the R2 files. Builds are ad-hoc signed and not notarized (`docs/release-process.md:3`). The `mnema-cli` sidecar lives inside the bundle. Settings can symlink it to `~/.local/bin/mnema`. The CLI talks to the running app over a per-user Unix socket. Licensing hashes the hardware UUID (salted with the license id or the product slug) and sends that hash to licensegate once per machine.

#### (c) Recommended Linux approach

**Primary: .deb + .rpm, built on Ubuntu 24.04, direct download plus the in-app Tauri updater.**
- Add a `bundle.linux` section, or a separate `tauri.linux.conf.json`. Set `targets` to `["deb","rpm","appimage"]` (drop `all`). Add deb `depends` for `libwebkit2gtk-4.1-0`, `libgtk-3-0`, `libayatana-appindicator3-1` (tray) and `libssl3` (SQLCipher's libcrypto), plus the PipeWire and portal packages that the other subsystems settle on. Tauri recommends building on the oldest base you support because of glibc ([Tauri deb guide](https://v2.tauri.app/distribute/debian/)). Tauri v2 needs webkit2gtk-4.1, which makes Ubuntu 22.04 the lowest possible floor ([Tauri prerequisites](https://v2.tauri.app/jp/start/prerequisites)); Mnema chose 24.04 for PipeWire and portal maturity (§10 decision 1).
- Tauri puts the main binary and the `externalBin` sidecar in **`/usr/bin`** in the .deb ([bundler debian.rs](https://raw.githubusercontent.com/tauri-apps/tauri/dev/crates/tauri-bundler/src/bundle/linux/debian.rs)). The CLI is therefore on `PATH` with no symlink. That only works once the name clash below is fixed.
- **Updater:** tauri-plugin-updater **2.10.0+** installs Deb, Rpm and AppImage updates ([changelog](https://v2.tauri.app/release/updater/all-versions/)), and this repo is already on **2.10.1** (`Cargo.lock`). For deb/rpm it runs `pkexec dpkg -i` or `pkexec rpm -U`, falling back to zenity/kdialog plus `sudo -S`, then to terminal `sudo` ([updater.rs](https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/plugins/updater/src/updater.rs)). The user sees one admin-password prompt per update. The existing minisign key and the stable/preview feeds carry over unchanged (ADR 0018).
- **Signing:** Linux has nothing like notarization. Keep the minisign `.sig` for in-app updates. GPG-sign the .rpm (`TAURI_SIGNING_RPM_KEY` / `_PASSPHRASE`, [Tauri RPM](https://v2.tauri.app/distribute/rpm/)) and publish a GPG-signed `SHA256SUMS` for manual downloads. An apt/dnf repo is not needed at first.
- **Licensing fingerprint:** on Linux, `hardware_uuid()` returns `/etc/machine-id` (fallback `/var/lib/dbus/machine-id`). `hardware_model()` returns `/sys/class/dmi/id/sys_vendor + product_name`, which anyone can read (checked on this machine: `-r--r--r--`). licensegate already salts and hashes whatever string it is given (`licensegate/src/lib.rs:41-53`), so no crate change is needed. That matches systemd's advice to never send the raw id and to hash it with an app-specific key ([machine-id(5)](https://www.freedesktop.org/software/systemd/man/latest/machine-id.html)).

**Secondary: AUR `mnema-bin`** (a PKGBUILD that unpacks the .deb). This is a few lines of upkeep and reaches Arch users. Updates come through the AUR helper. Turn the in-app updater off there or point it at the AUR, see (d).

**Fallback: AppImage.** It is supported by the updater with no admin prompt. It has known problems, see (d) and (f).

#### (d) Gaps

1. **Name clash: `mnema` is both the GUI and the CLI.** The .deb installs the GUI as `/usr/bin/mnema` and the CLI as `/usr/bin/mnema-cli`. The in-app "install CLI" step then symlinks `~/.local/bin/mnema` (`app_infra.rs:4366-4371`). Which program `mnema` runs depends on `PATH` order. Fix: on Linux, set `mainBinaryName` to something like `mnema-desktop`, and either ship the CLI as `/usr/bin/mnema` or keep the `~/.local/bin` link.
2. **CLI launches the app the wrong way on Linux.** `xdg-open mnema` (`crates/cli/src/main.rs:1199`) asks the desktop to open a *file* named `mnema`. It should be `gtk-launch day.mnema`, `xdg-open mnema://` (once the scheme is registered) or the binary path.
3. **AppImage CLI symlink breaks.** `bundled_mnema_cli_path()` resolves next to `current_exe()`, which for an AppImage is a temporary `/tmp/.mount_*` directory that changes on every launch. The `~/.local/bin/mnema` link goes stale as soon as the app exits. AppImage needs a small wrapper script that runs `"$APPIMAGE"` in CLI mode, or a copy refreshed on each launch.
4. **Licensing is silently open on Linux today.** With no fingerprint there is no trial issuance and no stamp (`trial.rs:166-171`), and status is forced to `Activated` (`adapter.rs:177-178`). Read literally, that means unlimited free capture. Needs a Linux `machine_id.rs` branch; the rest of the licensing code is cross-platform.
5. **Update Window is enforced only by the updater.** Any channel with outside updates (apt repo, AUR, Flathub) skips `build_out_of_window`. The app still shows the out-of-window notice for the running build (`app_updates.rs:435-445`), but it is a notice, never a lock.
6. **Updater platform keys.** `latest.json` needs a `linux-x86_64` entry. With deb, rpm and AppImage all built, Tauri decides which artifact each installed bundle type gets. Confirm the exact per-bundle key names against tauri-action output before merging feeds (**uncertain**; [updater docs](https://v2.tauri.app/plugin/updater/) only show `linux-x86_64`). The Windows research doc already warns that parallel jobs overwrite `latest.json` (`docs/windows/storage-access-release-research.md:208`).
7. **Native library audit.** Check with `ldd` whether `ort` rc.12 links ONNX Runtime statically on Linux x64 (**uncertain**). Declare `libssl3` in deb depends (or switch to `bundled-sqlcipher-vendored-openssl` on Linux). The speakrs/OpenBLAS default feature is macOS CoreML. Turning it off on Linux belongs to the diarization subsystem, but it decides whether gfortran is needed in CI.
8. **Deb/rpm metadata.** The description "A Tauri App" and authors "you" end up in the control file. Add an AppStream `metainfo.xml` via `bundle.linux.deb.files` ([Tauri Flatpak guide](https://v2.tauri.app/distribute/flatpak/)).
9. **Copy:** "this Mac" (`app_updates.rs:535`); the release docs and staging script use `shasum` (Linux has `sha256sum`).

#### (e) Fallbacks

- **AppImage only:** one file, no admin prompt on update, natively supported by the updater ([updater docs](https://v2.tauri.app/plugin/updater/)). Costs: gap 3 (CLI link), FUSE2 missing by default on Ubuntu 24.04 (`libfuse2t64`, [AppImage FUSE wiki](https://github.com/AppImage/AppImageKit/wiki/FUSE)), and the WebKitGTK/Wayland problems in (f).
- **Self-hosted GPG-signed apt + dnf repo:** natural for sysadmins. It skips the Update Window gate, so do it only if the business accepts that the gate becomes the honor system on Linux.
- **Flatpak (self-hosted remote, later Flathub):** see (h) and (f). Needs a 3-component app ID.
- **Snap:** no. Strict confinement blocks AT-SPI and native messaging, and classic confinement is granted only by manual review that has pushed back on screen-capture tools ([Snapcraft forum example](https://forum.snapcraft.io/t/classic-confinement-request-for-aurora-screenshots/50704)).

#### (f) Risks

1. **Flatpak identity break.** Flatpak itself rejects IDs with fewer than 3 components ("Names must contain at least 2 periods", [flatpak#4535](https://github.com/flatpak/flatpak/issues/4535)), and Flathub requires ≥3 ([Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements)). `day.mnema` cannot be a Flatpak ID. A Flatpak build means a new identifier (e.g. `day.mnema.Mnema`), which changes the config dir, socket path, keyring service names and the CLI's `APP_IDENTIFIER` constant.
2. **Flatpak sandbox versus core features.** AT-SPI reading of other apps needs `--talk-name=org.a11y.Bus`, because the default a11y proxy is filtered ([KDE bug 452132](https://bugs.kde.org/show_bug.cgi?id=452132)). Flathub rejects broad static permissions where a portal exists ([requirements](https://docs.flathub.org/docs/for-app-authors/requirements)). Secret Service becomes an app-only portal secret ([opensource.com](https://opensource.com/article/19/11/secrets-management-flatpak-applications)), so a host-side CLI cannot read the vault. Inside the sandbox `XDG_CONFIG_HOME` is `~/.var/app/<id>/config`, so the host CLI and the app disagree on the socket path. Flatpak has no way to publish a second command to the host `PATH`, so it needs a `flatpak run --command=mnema-cli …` wrapper ([wrapper pattern](https://gist.github.com/zocker-160/00da2e644e8b3afc1615e9133bc0ec4b)). The project is AGPL, so Flathub requires a build entirely from source with offline cargo/npm sources (OpenBLAS, whisper.cpp and ONNX Runtime included). Flathub's monetisation policy also requires that reviewers can test the full functionality ([requirements](https://docs.flathub.org/docs/for-app-authors/requirements)).
3. **WebKitGTK and the graphics stack.** Tauri AppImages bundle the build host's WebKit/GTK/Wayland libraries and show blank windows or EGL failures on newer distros and Wayland/NVIDIA ([Armbian fix](https://github.com/armbian/imager/pull/170), [dev.to write-up](https://dev.to/fengzeng/fixing-wayland-crashes-in-tauri-appimage-linuxdeploy-gtk-issue-1381)). deb/rpm use the system WebKit and avoid most of this, but a 24.04-built .deb running on Fedora/Arch still uses each distro's WebKit (fine; the version varies).
4. **Machine-id is weak against trial abuse.** Root can regenerate it (`rm /etc/machine-id && systemd-machine-id-setup`) and get a new trial, and cloned VM images may share an id if the image was not cleared ([machine-id(5)](https://www.freedesktop.org/software/systemd/man/latest/machine-id.html)). The DMI `product_uuid` would be sturdier, but it is root-only (0400 on this kernel 6.18; the 2023 patch to relax it is [not confirmed merged](https://lkml.iu.edu/hypermail/linux/kernel/2306.2/00406.html)). This is acceptable: the app is AGPL, so anyone can already rebuild it without the gate. Machine-id is not the weakest link. An OS reinstall gives a new id, which uses one of the 3 device slots; the existing in-app "Free up my devices" reset covers that.
5. **The admin-password prompt on every deb/rpm update** may annoy an always-on app. If it does, AppImage (no admin needed) is the escape hatch.
6. ~~The Ubuntu 22.04 baseline ages out.~~ **Resolved:** the floor is 24.04 (§10 decision 1), whose standard support runs to 2029, and whose portal and PipeWire 1.0 support restore tokens.

#### (g) Effort

- deb + rpm bundle config, Linux CI job, R2 promote of Linux artifacts, merged `latest.json`: **M**
- Linux `machine_id.rs` branch (machine-id + DMI product name) + one test: **S**
- CLI fixes (binary name, `xdg-open` launch, AppImage wrapper): **S**
- AUR PKGBUILD: **S**
- AppImage fallback hardening (strip bundled Wayland libs, FUSE note): **S–M**
- Flatpak (new ID, sandbox permissions, CLI wrapper, from-source build, Flathub review): **L**

Recommended path total: **M**.

#### (h) Parity cells

| Feature \ Format | AppImage | deb | rpm | Flatpak | AUR (-bin) |
|---|---|---|---|---|---|
| Auto-update (in-app, keeps Update Window) | full (no admin) | full (pkexec prompt) | full (pkexec prompt) | degraded: store updates, gate skipped; Tauri updater unusable | degraded: AUR helper, gate skipped |
| Capture works (PipeWire portal, mic, system audio) | full | full | full | full via portals + `--socket=pipewire/pulseaudio`; compositor D-Bus (e.g. `org.kde.KWin`) needs extra talk-names; GNOME Shell extension can't be installed from inside | full |
| AT-SPI (read other apps) | full | full | full | degraded: needs `--talk-name=org.a11y.Bus`; Flathub may refuse | full |
| Native messaging host (our browser extension) | degraded: per-user manifest must point at a stable wrapper, AppImage path can move | full (system manifests via `deb.files`) | full | degraded: needs `--filesystem` for browser manifest dirs + `flatpak run` wrapper | full |
| …to a sandboxed browser (Flatpak/Snap Firefox/Chrome) | degraded for all formats: depends on browser support of `xdg-native-messaging-proxy` ([flatpak list, Jun 2025](https://lists.freedesktop.org/archives/flatpak/2025-June/002391.html)) | same | same | same | same |
| CLI on PATH | degraded: needs wrapper (gap 3) | full (`/usr/bin`) after name fix | full | degraded: wrapper script; host CLI can't read sandbox secrets | full |
| Keyring (Secret Service) | full (if a keyring daemon runs) | full | full | degraded: Secret portal = app-only; CLI must run in-sandbox | full |
| Broker Unix socket | full | full | full | degraded: socket under `~/.var/app/…`; host CLI path mismatch | full |
| Licensing fingerprint (`/etc/machine-id`) | full | full | full | full (host machine-id is bind-mounted, [Flatpak wiki](https://github.com/flatpak/flatpak/wiki/Sandbox)) | full |

No cell is "impossible". Flatpak is degraded in 6 of 8 rows.

#### (i) Files to add or split

- `apps/desktop/src-tauri/tauri.linux.conf.json`: `bundle.targets`, `linux.deb.depends`, `linux.rpm.depends`, `linux.deb.files` (metainfo, native-messaging manifests), `mainBinaryName`, Linux icon PNGs.
- `apps/desktop/src-tauri/linux/day.mnema.metainfo.xml`. The `.desktop` file is generated by Tauri; a template is only needed for `MimeType=x-scheme-handler/mnema` if the deep-link plugin doesn't add it (verify).
- `crates/app-infra/src/machine_id.rs`: add a `#[cfg(target_os = "linux")]` branch (stays under 800 lines).
- `crates/cli/src/main.rs`: Linux launch command. (The file is already over 3,000 lines, so do not add to it; a later split is a separate task.)
- `apps/desktop/src-tauri/src/app_infra.rs`: AppImage-aware CLI install (wrapper instead of symlink when `$APPIMAGE` is set). The file is ~5,600 lines, so put the new logic in a small `cli_install/linux.rs`.
- `.github/workflows/linux-release.yml` (or a matrix leg) + `scripts/stage-linux-release-artifacts.sh` (`sha256sum`, GPG detached sig).
- `.github/workflows/*-promote.yml`: upload Linux artifacts and merge `latest.json`.
- `packaging/aur/PKGBUILD`.
- `docs/release-process.md`, `SUPPORTS.md` rows 59-62 / 236 / 283 / 289; `app_updates.rs:535` copy.

#### (j) Degradation and UI

- **deb/rpm update:** before calling the updater, show "Updating needs your administrator password" so the pkexec dialog isn't a surprise.
- **AUR / other package-manager installs:** detect this (install path not owned by our updater, or a build-time flag) and change Settings → Updates to "Updates come from your package manager". Keep the out-of-window notice.
- **AppImage CLI:** the "Install CLI" button writes a wrapper. Status shows "points at <AppImage path>; re-run if you move the file".
- **No keyring daemon** (minimal WMs): that belongs to the key-store subsystem, but packaging should add a "Recommends: gnome-keyring | kwalletmanager" hint.
- **Licensing:** if `/etc/machine-id` is missing or unreadable, keep today's behaviour of not locking anyone out. Log a Warn with no id value.

#### (k) What to test

1. Install the .deb on clean Ubuntu 24.04 and Debian 13, and the .rpm on Fedora 42+. The app launches, the tray works, and `ldd` shows no missing libs.
2. Update v→v+1 through the in-app updater for deb, rpm and AppImage on both stable and preview feeds. Confirm the pkexec prompt, the restart, and that a `.sig` mismatch is refused.
3. Update Window: a Licensed key with `update_through` before the build date gets the update declined.
4. `which mnema` and `mnema-cli` after a deb install, plus the "Install CLI" button. No `PATH`-order ambiguity.
5. With the app not running, a CLI call starts it (the fixed launch), and the socket shows up in `~/.config/day.mnema/` with the parent directory at 0700.
6. Licensing: a trial is issued on Linux. The activation hash is stable across reboot and app reinstall. Regenerating machine-id produces a new hash (documented). The device label is "Vendor Model".
7. AppImage on GNOME Wayland, KDE Wayland and NVIDIA: no blank webview. `~/.local/bin/mnema` still works after the app exits.
8. The AUR package builds in a clean chroot (`makepkg`, `namcap`).
9. `bash scripts/prepare-mnema-cli-sidecar.sh release` produces `mnema-cli-x86_64-unknown-linux-gnu`.

#### (l) Open questions

All five were settled on 2026-11-01 (§10):
1. **Update Window on package managers:** package channels ship; an uncovered build shows a permanent banner with Renew and a safe Roll back, never a lock (decision 9, ADR 0064).
2. **`mnema` command:** the CLI owns it; the GUI binary is `mnema-desktop` on Linux (decision 12).
3. **Architecture:** x86_64 only in v1 (decision 12).
4. **Flatpak app ID:** moot, no Flatpak in v1 (decision 12).
5. **Machine-id trial resets:** accepted for v1, no server change (decision 10).

### 4.13 CI

> **Updated 2026-11-01 after the owner decisions (§10):** the container is `ubuntu:24.04` and v1 is x86_64 only, so the arm64 jobs below are kept for later but marked deferred. Also, `app-infra` and `cli` in the allowlist need the two one-line fixes from §2 first.

Research date: 2026-11-01. Repo is **public** (`gh repo view` → `isPrivate:false`), so standard hosted runners (x64 and arm64) are free; larger runners are not.

**Recommendation in one sentence:** add one new `linux-build.yml` that builds inside an `ubuntu:24.04` container on `ubuntu-24.04` hosts (arm64 hosts later). Start it as a PR job that tests only the crates that already work on Linux, and widen it as the port lands. Add a nightly headless smoke job and a tag-triggered tauri-action release job later. The trade-off: the container step needs a little more setup than plain `runs-on: ubuntu-22.04`, but it keeps the glibc 2.35 floor after GitHub retires the 22.04 runner image.

---

#### (a) Current CI — what exists

| Workflow | Runner | What it does | Linux relevance |
|---|---|---|---|
| `.github/workflows/type-check.yml` | `macos-15` (:16) | bun check + frontend/web tests, `brew` Fortran (:81), sidecar (:95), stub licenses (:104), `cargo check --workspace --all-targets --locked` (:107), semantic-search tests (:110, :119 with `metal`) | The PR gate. It is macOS-only today. Sets `MNEMA_TRIPWIRE_NO_KEYCHAIN=1` (:30). |
| `.github/workflows/macos-release.yml` | `macos-15` (:24) | Tag → tauri-action@v0 (:176) → entitle/notarize/re-sign → rebuild DMG + updater tar → **patches `latest.json` with `(.platforms[].signature) = $sig` (:304)** → stage (:316) | That :304 jq step would overwrite a Linux platform's signature if Linux shares the same draft release's `latest.json`. |
| `.github/workflows/macos-release-promote.yml` | `ubuntu-latest` (:47) | Draft → R2 publish. It requires a DMG / `.app.tar.gz` (:99-101), rewrites `.platforms` URLs (:141), and verifies every `.platforms[].url` (:213) | Already on Linux. Needs Linux asset checks (`.AppImage`, `.deb`, `.rpm`, `.sig`) added. |

Scripts and config the Linux CI touches:
- `scripts/ci-install-fortran.sh` is Homebrew-specific (`brew install gcc` :27, `-print-file-name=libgfortran.dylib` :61). On Linux, `apt-get install gfortran` replaces it entirely, and no shim is needed (Ubuntu ships an unversioned `gfortran`).
- `scripts/openblas-build-env.sh:23` hardcodes `libgfortran.dylib` and :18 tells you to `brew install gcc`. On Linux, the `.dylib` lookup returns the bare name, so `dirname` gives `.`, which is the same silent failure that ci-install-fortran.sh:15-17 describes. It needs a `.so` branch, or Linux CI should not source it (Ubuntu's gfortran lib dir is already on the default linker path).
- `scripts/prepare-mnema-cli-sidecar.sh` already works on Linux: it takes the triple from `rustc -vV` (:32-33) and uses an empty exe suffix (:39-42). The only macOS-only branch is universal (:73).
- `apps/desktop/src-tauri/build.rs:45-48`: the static-Fortran force-link runs **only for `target_os = "macos"`**. On Linux the binary links `libgfortran.so.5` (and on x86_64 `libquadmath.so.0`) dynamically. That is acceptable if the .deb/.rpm declares the dependency and the AppImage bundles it, but it must be a deliberate choice.
- `turbo.json:25` `passThroughEnv`: any new `MNEMA_*` variable the Linux build needs must be added there.
- `apps/desktop/src-tauri/tauri.conf.json:37-38`: `createUpdaterArtifacts: true`, `targets: "all"`. On Linux this yields deb + rpm + AppImage, plus a signed AppImage for the updater.

##### Blockers that stop Linux CI from compiling the workspace today
1. **speakrs `coreml` feature is a hard compile error off macOS.** `crates/speaker-analysis/Cargo.toml:38` enables `features = ["coreml", "openblas-static"]` unconditionally, and speakrs 0.4.2 has `#[cfg(all(feature = "coreml", not(target_os = "macos")))] compile_error!(...)` (`~/.cargo/registry/src/*/speakrs-0.4.2/src/lib.rs:202-203`). The desktop crate turns it on by default (`apps/desktop/src-tauri/Cargo.toml:19, 27`). So `cargo check --workspace` fails on Linux until `coreml` is moved under a `[target.'cfg(target_os="macos")']` entry.
2. The desktop crate has **597** `cfg(target_os = "macos")` sites and 1,250 tests. Whether it compiles on Linux is a question for the port work in §4.1–§4.12. CI can only gate on it after that work lands.
3. SQLCipher on Linux links the system `libcrypto` (dynamic). `libsqlite3-sys-0.30.1/build.rs:205-217` uses CommonCrypto only on Apple and otherwise does `rustc-link-lib=dylib=crypto`, so CI needs `libssl-dev`. The shipped package then depends on `libssl3`, which is fine on 22.04+.

---

#### (b) Recommended Linux CI design

##### Runners (sources checked 2026-11)
- Standard hosted Linux labels: `ubuntu-22.04`, `ubuntu-24.04`, `ubuntu-latest`, `ubuntu-26.04`, plus `-arm` variants of each. All are 4 vCPU / 16 GB / 14 GB SSD on public repos — https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- **ubuntu-22.04 images are being retired.** Notice: "will begin deprecation on September 17th … fully unsupported by April 17th" (on an Aug-2026 image, so most likely Sep 2026 → Apr 2027; **the year is inferred, not stated**) — https://github.com/actions/runner-images/releases/tag/ubuntu22%2F20260810.260. Ubuntu 26.04 is in public preview (same page).
- Public-repo standard runners are free — https://docs.github.com/en/billing/concepts/product-billing/github-actions. For a private repo: Linux x64 2-core $0.006/min, arm64 $0.005/min, macOS $0.062/min; a 4-core GPU runner is $0.052/min and is never free — https://docs.github.com/en/enterprise-cloud@latest/billing/reference/actions-runner-pricing
- GPU runners are **not recommended**. Nothing in CI needs CUDA (candle-CPU on Linux per SUPPORTS.md, and ort CPU), and they are paid even on public repos.

##### glibc baseline
- Tauri says to build "using the oldest base system you intend to support", with Ubuntu 22.04 / Debian 12 as examples. linuxdeploy cannot cross-compile ARM AppImages, so ARM must build on ARM runners — https://v2.tauri.app/distribute/appimage/
- **Choice:** `container: ubuntu:24.04` (glibc 2.39) on `ubuntu-24.04` hosts (§10 decision 1). Pinning the container, not the runner label, keeps the floor fixed when GitHub moves `ubuntu-latest`.
- **Catch:** jammy's PipeWire is **0.3.48** (noble 1.0.5, 26.04 1.6.2) — https://launchpad.net/ubuntu/+source/pipewire. If the capture design needs PipeWire ≥ 1.0 APIs, the floor has to move to 24.04 (glibc 2.39). That is open question 1.

##### Job list
| Job | Trigger | Runner | Does |
|---|---|---|---|
| `linux-check` | every PR | x64, container 24.04 | `bun run check`; `cargo check` + `cargo test` on an **allowlist** of Linux-clean crates (it grows as the port lands; ends as `--workspace --all-targets --locked`) |
| `linux-check-arm` (deferred: after v1) | push to `main` + nightly | arm64, container 24.04 | Same as `linux-check`, with no cache save on PRs (protects the cache budget) |
| `linux-smoke` | nightly + `workflow_dispatch` + PR label `linux-smoke` | x64, ubuntu-24.04 VM (needs systemd-less session daemons, so no container) | Tier-2 headless tests (see d) |
| `linux-release` | `v*` tag + dispatch | x64 (arm64 after v1), container 24.04 | tauri-action → deb/rpm/AppImage + `.sig`, `OPENBLAS_DYNAMIC_ARCH=1`, glibc symbol guard, upload to the **same draft release** |

##### Exact apt package list (inside `ubuntu:24.04`)
```
# base (container has none of these)
sudo git ca-certificates curl wget file unzip xz-utils zstd pkg-config build-essential
# Tauri v2 (https://v2.tauri.app/start/prerequisites/ + pipeline extras patchelf/xdg-utils)
libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev patchelf xdg-utils
# native source builds: tesseract+leptonica (cmake, tesseract-rs build.rs), whisper.cpp (cmake+bindgen),
# MNN headers (ocr-rs bindgen), OpenBLAS (gfortran; libgfortran-11-dev ships the static .a)
cmake ninja-build clang libclang-dev gfortran
# Linux port deps — ONLY once the matching port slice lands (confirm against §4.1 and §4.8)
libpipewire-0.3-dev libspa-0.2-dev libdbus-1-dev libasound2-dev
libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev   # only if GStreamer is chosen for decode/encode
libsecret-1-dev                                         # only if a libsecret-binding crate is chosen (keyring's secret-service backend needs libdbus-1-dev instead)
# smoke-job runtime only (not in build jobs)
xvfb dbus-x11 gnome-keyring pipewire wireplumber pipewire-pulse
```
Not needed as system packages: tesseract/leptonica. `tesseract-rs-0.2.0/build.rs:12-14` downloads leptonica 1.84.1 and tesseract 5.3.4 from GitHub and builds them with cmake. ONNX Runtime is not needed either: ort-sys downloads prebuilt Linux x64/aarch64 binaries (`ort-sys-2.0.0-rc.12/build/download/dist.txt:11-12`). Neither is MNN: ocr-rs downloads prebuilt `linux-x86_64` / `linux-aarch64` (`ocr-rs-2.2.2/build.rs:232-251`).

Note that the Tauri docs disagree with each other: prerequisites lists `libayatana-appindicator3-dev`, while the GitHub pipeline example lists `libappindicator3-dev` (https://v2.tauri.app/distribute/pipelines/github/). Use ayatana; it is the maintained one on 22.04+.

##### Draft `linux-build.yml` skeleton (not committed)
```yaml
name: Linux Build

on:
  pull_request:
  push:
    branches: [main]
    tags: ["v*"]
  schedule:
    - cron: "17 3 * * *"
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: linux-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

env:
  CARGO_TERM_COLOR: always
  CARGO_INCREMENTAL: "0"            # incremental artifacts bloat cache, useless in CI
  CARGO_PROFILE_DEV_DEBUG: "0"      # shrinks target/ a lot; tests still get panics + line info off
  MNEMA_TRIPWIRE_NO_KEYCHAIN: "1"   # same tripwire as type-check.yml:30 (test jobs only)

jobs:
  check:
    if: github.ref_type != 'tag'
    strategy:
      fail-fast: false
      matrix:
        include:
          - { runner: ubuntu-24.04,     arch: x64,   save: true }
          # arm64 deferred until after v1 (§10 decision 12):
          # - { runner: ubuntu-24.04-arm, arch: arm64, save: false }   # PR: restore only
    runs-on: ${{ matrix.runner }}
    container: ubuntu:24.04          # glibc 2.39 floor (§10 decision 1)
    timeout-minutes: 90
    steps:
      - name: Install system packages
        run: |
          apt-get update
          DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
            sudo git ca-certificates curl wget file unzip xz-utils zstd pkg-config build-essential \
            libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev \
            patchelf xdg-utils cmake ninja-build clang libclang-dev gfortran
      - uses: actions/checkout@v4
      - uses: oven-sh/setup-bun@v2
        with: { bun-version: 1.3.5 }
      - uses: actions/cache@v4
        with:
          path: |
            ~/.bun/install/cache
            node_modules
            apps/desktop/node_modules
          key: linux-${{ matrix.arch }}-bun-${{ hashFiles('bun.lock', 'package.json', 'apps/desktop/package.json') }}
      - run: bun install --frozen-lockfile
      - run: bun run check
      - run: bun --cwd=apps/desktop test
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: linux-${{ matrix.arch }}-dev
          cache-directories: ~/.tesseract-rs          # tesseract-rs builds leptonica+tesseract here
          save-if: ${{ matrix.save && github.ref == 'refs/heads/main' }}
      # Phase 0 allowlist: crates with no macOS-only deps. Grow this; end state is the workspace line below.
      - name: Test Linux-clean crates
        run: >
          cargo test --locked
          -p capture-types -p capture-runtime -p capture-metadata -p capture-vad
          -p secret-redaction -p ai-runtime -p semantic-search -p app-infra -p cli
      # Phase 1 (after speakrs coreml is target-gated and the desktop crate compiles):
      # - run: ./scripts/prepare-mnema-cli-sidecar.sh debug --locked
      # - run: printf 'Placeholder\n' > apps/desktop/THIRD_PARTY_LICENSES.md
      # - run: cargo check --workspace --all-targets --locked
      # - run: cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --locked

  smoke:
    if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' || contains(github.event.pull_request.labels.*.name, 'linux-smoke')
    runs-on: ubuntu-24.04            # VM, not container: session daemons need a real user session
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
            libxdo-dev libssl-dev cmake ninja-build clang libclang-dev gfortran \
            xvfb dbus-x11 gnome-keyring pipewire wireplumber pipewire-pulse libpipewire-0.3-dev libspa-0.2-dev
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { shared-key: linux-x64-smoke, save-if: "${{ github.ref == 'refs/heads/main' }}" }
      - name: Headless session tests
        run: |
          dbus-run-session -- bash -euxc '
            echo -n ci | gnome-keyring-daemon --unlock --components=secrets
            pipewire & wireplumber & sleep 2
            pw-cli info 0 >/dev/null                  # fail fast if the graph is not up
            xvfb-run -a cargo test --locked --features linux-smoke -- --ignored linux_smoke_
          '
        env:
          MNEMA_TRIPWIRE_NO_KEYCHAIN: ""              # this job exercises the real Secret Service on purpose

  release:
    if: github.ref_type == 'tag' || github.event_name == 'workflow_dispatch'
    permissions: { contents: write }
    environment: macos-release                        # reuse TAURI_SIGNING_* secrets (rename env later)
    strategy:
      fail-fast: false
      matrix:
        include:
          - { runner: ubuntu-24.04,     arch: x86_64 }
          # - { runner: ubuntu-24.04-arm, arch: aarch64 }   # deferred until after v1
    runs-on: ${{ matrix.runner }}
    container: ubuntu:24.04
    timeout-minutes: 120
    env:
      OPENBLAS_DYNAMIC_ARCH: "1"     # same reason as macos-release.yml:40 — runner CPU != user CPU
      MNEMA_LICENSE_PUBLIC_KEY: ${{ secrets.MNEMA_LICENSE_PUBLIC_KEY }}
      MNEMA_LICENSE_KID: ${{ secrets.MNEMA_LICENSE_KID }}
      MNEMA_LICENSE_PK_TOKEN: ${{ secrets.MNEMA_LICENSE_PK_TOKEN }}
      VITE_LICENSE_CHECKOUT_URL: ${{ secrets.VITE_LICENSE_CHECKOUT_URL }}
      VITE_RENEWAL_CHECKOUT_URL: ${{ secrets.VITE_RENEWAL_CHECKOUT_URL }}
    steps:
      # ... same apt + checkout + bun + rust steps as `check` (no rust-cache save) ...
      # ... verify-desktop-release-version.sh, licensing-secret gate, CRL floor fetch,
      #     generate-third-party-licenses.sh — copied from macos-release.yml ...
      - run: ./scripts/prepare-mnema-cli-sidecar.sh release --locked
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          projectPath: apps/desktop
          tagName: ${{ github.ref_name }}
          releaseDraft: true
          includeUpdaterJson: true
      - name: glibc floor guard
        run: |
          max=$(objdump -T target/release/mnema | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)
          echo "max glibc symbol: $max"
          [ "$(printf '%s\nGLIBC_2.35\n' "$max" | sort -V | tail -1)" = "GLIBC_2.35" ]
```
Caveats on the skeleton. `linux_smoke_` and `--features linux-smoke` are placeholders; those tests don't exist yet. The `release` job and `macos-release.yml` write into the same draft `latest.json`, which is a race (see risks).

---

#### (c) Caching plan
- **Rust:** use `Swatinem/rust-cache@v2`. It caches `~/.cargo` (registry/cache/git) and the dependency part of `target/`, and it does **not** cache `~/.cargo/registry/src`. It prunes stale and incremental artifacts. Use one `shared-key` per arch/profile, and `save-if` only on `main` so PRs restore without writing — https://github.com/Swatinem/rust-cache
- **OpenBLAS from source:** its output sits in `target/<profile>/build/openblas-src-*/out`, a dependency build-script output, so rust-cache keeps it. No separate cache is needed. Release builds use `OPENBLAS_DYNAMIC_ARCH=1` and are deliberately **not cached** (tags are rare, and caching them would push PR caches out).
- **tesseract/leptonica:** `tesseract-rs-0.2.0/build.rs:28-33` writes to `~/.tesseract-rs` on Linux, outside `target/`. Add it via `cache-directories`.
- **MNN prebuilt:** cached under `~/.cargo/registry/src/*/ocr-rs-2.2.2/3rd_party/prebuilt` (`ocr-rs-2.2.2/build.rs:260`). rust-cache **excludes** registry/src, so it re-downloads on every cold build. It is small; accept it, or set `MNN_LIB_DIR` to a cached pinned copy. The tag it pulls is `"dev"` (`build.rs:7`), which is unpinned (risk).
- **Bun:** same `actions/cache` block as type-check.yml:41-50, with the key prefixed by arch.
- **sccache:** this is the fallback, not the default. Each job is one big compile, so rust-cache's whole-target restore beats per-object sccache hits. Revisit only if PR jobs keep missing the cache.
- **Budget:** 10 GB per repo is free, with LRU eviction across all caches. Above that you pay $0.07/GB-month, and only if you raise the limit — https://docs.github.com/en/billing/concepts/product-billing/github-actions
  - Estimates (unverified, no builds run): macOS dev target ~4–6 GB (already there). Linux x64 dev target with `CARGO_PROFILE_DEV_DEBUG=0` ~2.5–4 GB, tesseract cache ~0.3 GB, bun ~0.4 GB. Adding arm64 is another ~3–4 GB, which **exceeds 10 GB**, so caches would evict each other.
  - Two ways out: save only x64 (as in the skeleton), or raise the repo cache limit to ~25 GB, about $1/month.

##### CI minutes (estimates, 4-vCPU public runner; flag: not measured)
| Job | Cold | Warm |
|---|---|---|
| linux-check x64 (Phase 0 allowlist) | 25–35 min (OpenBLAS isn't in it unless speaker-analysis is; candle + SQLCipher + cmake deps dominate) | 6–10 min |
| linux-check x64 (Phase 1 full workspace + desktop tests) | 45–60 min (OpenBLAS from source ~8–12, tesseract+leptonica ~6–8, whisper.cpp ~3) | 12–18 min |
| linux-check arm64 | ~same as x64 | n/a (no save on PR) |
| linux-release ×2 | 60–90 min each (DYNAMIC_ARCH builds every kernel; Tauri docs say a "typical" ARM build is ~10 min, but this app's native stack is far heavier) | — |
| smoke | 20–30 min | 10–15 min |

Cost is $0 while the repo is public. If the repo went private: roughly 30 PRs/month × ~15 min ≈ 450 min ≈ $3/month on x64. That is negligible next to the macOS gate at ~10× the per-minute price.

---

#### (d) Test tiers

**Tier 1 — unit tests, every PR, Linux x64 container.** Counts below are `#[test]`/`#[tokio::test]` hits and `cfg(target_os="macos")` sites per crate:
- Already platform-neutral (0 macOS cfgs): capture-types 58, capture-runtime 16, capture-metadata 19, capture-vad 21, secret-redaction 19, ai-runtime 20, semantic-search 65 (17 `#[ignore]` need model weights; the CPU path is exactly what Linux ships).
- Likely fine (few cfgs, non-mac stubs exist): app-infra 926 tests / 52 cfgs. The vault master-key source has a non-mac "unsupported" stub (`crates/app-infra/src/secret_vault.rs:83-95`), and tests pin a scratch vault, so they should pass. cli 72 / 2.
- speaker-analysis without the `speakrs` feature (`default = []`) is testable now. With `speakrs`, it is blocked by the coreml compile_error until the feature is target-gated.
- After the port: desktop crate (1,250 tests / 597 cfgs), audio-transcription (62/27), ocr (15/24, tesseract/paddle paths are the Linux providers), capture-* crates (mostly macOS-gated today, with Linux backends to come).
- Frontend: `bun run check`, `bun --cwd=apps/desktop test`, `bun --cwd=apps/web test` are OS-independent, so running them once on Linux as well is cheap redundancy. Optional: drop them from the macOS job later to save mac minutes.

**Tier 2 — headless integration, nightly or on a label, ubuntu-24.04 VM:**
- **Secret Service master-key source:** the pattern is `dbus-run-session` + unlocked `gnome-keyring-daemon` (as in zalando/go-keyring CI, https://github.com/zalando/go-keyring/actions/runs/9301160510/workflow; SecretStorage docs recommend `dbus-run-session` for headless tests, https://github.com/mitya57/secretstorage).
- **PipeWire mic and system-audio smoke:** run `pipewire` + `wireplumber` inside the same dbus session, create a null sink or virtual source, feed it a tone, and assert the writer produces a non-silent m4a. **I found no complete public GitHub Actions example of this.** The closest is GNOME's move to running PipeWire in a dbus session in CI (https://gitlab.gnome.org/GNOME/gnome-remote-desktop/-/merge_requests/13). Expect flakiness; keep it nightly.
- **App launch smoke:** `xvfb-run -a` the release binary with a scratch `MNEMA_SAVE_DIRECTORY` / `MNEMA_APP_CONFIG_DIR`, wait for the window, then quit. WebKitGTK under Xvfb may need `WEBKIT_DISABLE_COMPOSITING_MODE=1` (unverified).
- **Not feasible in CI:** Wayland screen capture via xdg-desktop-portal ScreenCast. It needs a compositor, a portal backend and an interactive consent dialog. At most, test the X11 path under Xvfb if the port implements one.
- **glibc floor guard** on release artifacts (in the skeleton).

**Tier 3 — manual, on real hardware (release checklist):**
- Wayland portal capture on GNOME and KDE, real mic, system audio, Bluetooth device switch.
- Sleep/resume, multi-monitor, permission flows.
- AppImage/deb/rpm install on clean Ubuntu 24.04/26.04, Debian 13, Fedora 42+ and Arch.
- Real arm64 hardware (after v1).
- Updater round trip, from an old AppImage to a new one.

---

#### (e) Risks
1. **The workspace cannot compile on Linux yet.** The speakrs `coreml` feature (`crates/speaker-analysis/Cargo.toml:38`, speakrs `lib.rs:202`) and hundreds of macOS-gated sites in the desktop crate block it. CI can only grow as fast as the port. Mitigation: the allowlist job lands first, so regressions in shared crates are caught now.
2. **OpenBLAS without `OPENBLAS_DYNAMIC_ARCH=1` on x86_64.** It gets tuned to the Azure runner's CPU (possibly AVX-512), which can crash with an illegal instruction on users' older CPUs. This is the same class of bug as the M4→M1 note in CLAUDE.md. It must be set in the Linux release job.
3. **Shared `latest.json` race and clobber.** Mac and Linux release jobs both write the draft release's `latest.json`. `macos-release.yml:304` overwrites **every** platform's signature with the mac tarball's signature, which silently breaks Linux auto-update. Fix it to patch only the `darwin-*` keys, and serialize the jobs (Linux uploads after mac, or a final merge job). The promote workflow (:99-101, :141, :213) also needs Linux assets added.
4. **Cache budget thrash.** macOS plus two Linux arches exceed the free 10 GB, so caches evict each other and every PR goes cold (~45–60 min).
5. **Network-fetched native deps at build time:** tesseract/leptonica from GitHub zips, MNN `"dev"` (unpinned) from zibo-chen/MNN-Prebuilds, ORT from cdn.pyke.io. A flaky download fails the build, and an unpinned MNN tag can change under you. The prebuilt ORT/MNN glibc floors are unknown, so the guard step will catch it if they exceed 2.35.
6. ~~**Runner retirement:**~~ moot: the floor is a pinned `ubuntu:24.04` container, not a runner label.
7. **Dynamic `libgfortran.so.5` / `libssl3` / `libquadmath`** runtime dependencies on Linux (`build.rs:45-48` is macOS-only; SQLCipher links `libcrypto`). The .deb/.rpm must declare them (`tauri.conf.json` `bundle.linux.deb.depends`), and the AppImage must bundle them.
8. **Headless PipeWire tests are flaky** with no proven Actions recipe. Keep them nightly and non-blocking at first.

#### (f) Effort
- Phase 0 (allowlist PR job, x64, caching): **S**, about 1 day.
- Phase 1 (full workspace check + desktop tests): **S** of CI work, but **blocked** on porting slices (speakrs feature gating, desktop compile).
- Release job (deb/rpm/AppImage, signing, latest.json merge, promote-workflow Linux assets, glibc guard): **M**, about 3–4 days, mostly the release/promote plumbing.
- Tier-2 smoke (keyring + PipeWire + Xvfb launch): **M**, with flake-hunting expected.
- Overall: **M**.

#### (g) Open questions
1. ~~22.04 or 24.04 floor?~~ **Decided:** 24.04 (§10 decision 1).
2. ~~arm64 in v1?~~ **Decided:** no, x86_64 only, so the cache-budget problem goes away for now (§10 decision 12).
3. Should Linux reuse the `macos-release` environment for TAURI_SIGNING secrets, or should it be renamed to `release`?
4. Should the AppImage statically link the Fortran runtime (a new Linux branch in `build.rs`, e.g. `-static-libgfortran`; note that libquadmath does not exist on aarch64), or rely on the package dependency / AppImage bundling?
5. ~~Raise the cache limit or drop arm64 cache saves?~~ Moot until arm64 returns.
6. Once Linux runs the frontend tests, should the mac gate stay as is, or drop duplicated work to save mac minutes (relevant only if the repo goes private)?

## 5. Parity matrix

Labels: **full** (same as macOS), **degraded** (works, with the gap noted), **impossible**. Each cell assumes the recommendation in §4 has been built. "wlroots" means Sway, Hyprland, niri, labwc and similar. Features that don't depend on the desktop at all (OCR, ML, local LLM, packaging) take the same value in all four Linux columns.

| Feature | macOS | GNOME Wayland | KDE Wayland | wlroots | X11 |
|---|---|---|---|---|---|
| Screen capture | full: ScreenCaptureKit, silent after the one-time permission | degraded: portal dialog once, then restore token; screen-sharing indicator always visible; may re-ask after a monitor change | degraded: same; KDE ≥ 6.6.6 fixes a token bug | degraded: chooser behaviour depends on config; token support varies (xdph only if its box is ticked) | full: no consent needed (`xcap`) |
| Multiple monitors | degraded: first display only (`capture-screen/src/lib.rs:3314`) | full (parity): the user picks one monitor | full (parity) | full (parity) | full (parity) |
| Hiding Mnema's own windows | degraded: filtered later, not at capture | degraded: frames tagged while a Mnema window has focus | degraded, or full if KWin's per-window hide can be scripted (unverified) | Hyprland full (`noscreenshare`); Sway degraded | full: black out own window areas |
| Per-app screen exclusion | full: ScreenCaptureKit app filter | degraded: pause while in front with the Mnema extension (hiding at capture is impossible); unavailable without it | degraded: pause while in front; full if the KWin spike works | Hyprland full; Sway degraded (pause while in front) | degraded: black out windows by position |
| Video writing and previews | full: AVFoundation, hardware H.264 | full: OpenH264 on the CPU | full | full | full |
| Frontmost app and window title | full | degraded: only with Mnema's Shell extension plus a log-out | full: KWin script | full: wlr window list (PID through IPC on Sway/niri) | full: `_NET_ACTIVE_WINDOW` |
| Firefox/Zen URL | full: Accessibility API | full: AT-SPI `DocURL` | full | full if at-spi2-core is installed | full |
| Chromium-family URL | full: AppleScript | degraded: accessibility must be on when the browser starts (one restart); may also need a flag (unverified) | degraded | degraded | degraded |
| OCR | full: Vision | degraded: PaddleOCR on CPU, slower, English-only bundle until PP-OCRv6 | degraded | degraded | degraded |
| Microphone | full: permission prompt | full: no prompt (unsandboxed) | full | full (PulseAudio/ALSA fallback) | full |
| System audio | full: process tap | full: linked app streams; degraded to whole-output capture on PulseAudio-only systems | full | full | full |
| Per-app audio exclusion | full | degraded: link-based, a heuristic around effect nodes; impossible on PulseAudio-only systems | degraded | degraded | degraded |
| Transcription | full: Apple Speech, Whisper on Metal, Parakeet, Deepgram | degraded: no Apple Speech; Whisper on CPU; Parakeet and Deepgram unchanged | degraded | degraded | degraded |
| Speaker detection | full: CoreML, ~529× real time | degraded: same models on CPU, speed unmeasured | degraded | degraded | degraded |
| Semantic search | full: candle on Metal | degraded: candle on CPU, backfill ~5–8× slower (estimate) | degraded | degraded | degraded |
| Local LLM (Ollama/Llamafile) | full: Metal | full: same code; speed depends on GPU vendor | full | full | full |
| Secrets (database key, vault) | full: readable by app + CLI only | degraded: any same-user program can read them; unlocked at login via PAM | degraded: needs ksecretd's Secret Service enabled | degraded: keyring may be absent, then the explicit file opt-in | depends on the desktop running |
| Input idle | full | full: Mutter `GetIdletime` | full: ext-idle-notify v2 | full (Wayfire degraded, v1 only) | full (screen-saver extension; resets on display power-off) |
| Lock detection | full | full: logind `LockedHint` | full | degraded: depends on the locker | degraded: depends on the locker |
| Suspend/resume (finish tail segment) | full | full: logind delay hold | full | full | full |
| Notification with actions | full | full | full | degraded: mako/dunst action setup | full on desktop notification daemons |
| Tray | full | degraded: needs the AppIndicator extension (Ubuntu preinstalls it, Fedora doesn't); menu only | degraded: menu only | degraded: needs a bar with a tray | degraded: needs a tray host |
| Global shortcuts | full | degraded on GNOME 48+ (portal; user approves keys); **impossible on 46–47**: user binds `mnema://` links | degraded: portal | impossible natively: user binds `mnema://` links | full |
| Quick Recall launcher | full: non-activating panel | degraded: normal focused window, desktop decides placement | degraded (layer-shell could improve it) | degraded (layer-shell could improve it) | degraded: placed and on top, but takes focus |
| Permission flow | full | degraded: portal consent + indicator | degraded | degraded | full: none needed |
| In-app updates | full | full for deb/rpm (admin password prompt) and AppImage; AUR uses its own updates | same | same | same |
| Licensing activation | full | full once `/etc/machine-id` is wired (**open today**) | same | same | same |

**By desktop:** KDE Plasma 6 Wayland is the closest to macOS. GNOME Wayland is the most common Linux desktop but carries two hard gaps: per-app screen exclusion, and app tracking without an extension. X11 is easiest technically but is being removed by GNOME and KDE.

## 6. Proposed architecture

**Recommendation: keep each crate's existing public API as its seam and put per-OS code in sibling modules. Don't build a new cross-cutting `platform` crate of trait objects.** The codebase already works this way:
- `capture-screen` exposes free functions (`start_capture_session_with_options`, `rotate_…`, `stop_…`, `screen_display_available`).
- `capture-system-audio` exposes `SystemAudioCaptureSession` and `SegmentHooks`.
- the key stores already have adapter traits.
- 122 non-macOS stubs already sit behind those seams (§2).

The unmerged Windows port uses the same shape. A central trait layer would duplicate those seams and make every call site change for no new capability.

**Rules:**
1. **Ungate before adding.** Much of what is behind `cfg(target_os = "macos")` is plain Rust:
   - frame equivalence and activity fingerprints;
   - VAD resampling and the mic format-stability state machine;
   - privacy refresh;
   - most of `segments.rs`, `lifecycle.rs` and `runtime.rs`, which are gated only because the session types they hold are.

   Move this code out of the gates first. That's most of Phase 0, and it is also what produced the compile errors in §2.
2. **Per crate, `platform/{macos,linux}.rs` (or `macos.rs`/`linux.rs` siblings), selected by `cfg`.**
3. **Runtime choice only where Linux has several backends,** using a small enum chosen at startup rather than `dyn` traits:
   - screen capture: `Portal | X11`
   - active window: `KWin | GnomeExtension | Wlr | X11 | None`
   - input idle: `Mutter | ExtIdleNotify | XScreenSaver | None`
   - global shortcuts: `Plugin (X11) | Portal | DeepLinkOnly`
4. **One capability report for the frontend.** Add a `get_platform_capabilities` command so the UI renders from data instead of `isMac` or user-agent checks. Some of these fields already exist piecemeal: `ScreenCaptureSupport` (`capture-screen/src/lib.rs:395-399`) and `pick_device`'s `cpu_fallback`. Fields:

   | Field | Values |
   |---|---|
   | `screenConsent` | `Tcc \| PortalDialog \| None` |
   | `privacyExclusion` | `Full \| FocusGated \| Masked \| Unavailable` |
   | `activeWindow` | backend name, plus `needsSetup` |
   | `globalShortcuts` | `Accelerator \| DesktopManaged \| ManualBinding` |
   | `updater` | `InApp \| InAppNeedsAdmin \| PackageManager` |
   | `keyStore` | `Keychain \| SecretService \| File` |
   | `tray` | `bool` |
   | `grantNeedsRelaunch` | `bool` |
5. **Harvest the Windows branch's shared pieces into `main` first.** `origin/windows-key-stores-and-final-gaps` (184 files, last commit 2026-07-13, not merged) already contains:
   - a frame-rate scheduler, `crates/capture-screen/src/frame_schedule.rs`;
   - a `crates/media-decode` crate;
   - the speakrs CPU-mode file subset.

   Landing those on `main` once means both ports share them, instead of Linux re-solving them.
6. **Split as you go.** Most files the port touches are already over the 800-line rule (sizes below).

**Files to split or add, by subsystem:**

| Today (lines) | Split into / add | Why |
|---|---|---|
| `crates/capture-screen/src/lib.rs` (5,376) | `frame_index.rs`, `activity.rs`, `equivalence.rs` (ungated), `frame_export.rs`, `platform/macos/{sck_session, avfoundation_session, content_filter, validate, stream_delegates}.rs`, `platform/linux/{portal_session, pipewire_stream, x11_session, privacy, liveness}.rs` | §4.1 |
| `crates/capture-writers/src/lib.rs` (2,146) | `macos/{video_writer, audio_writer, audio_decode, trim}.rs`, `video_writer_openh264.rs`, `video_reader.rs`, `opus_writer.rs`, `decode.rs` (shared non-macOS decode) | §4.1, §4.5 |
| `apps/desktop/src-tauri/src/app_infra/frame_preview.rs` (3,892) | `frame_preview/{cache, dto, scrub, video_extract_macos, video_extract_linux}.rs` | §4.1 |
| `crates/capture-microphone/src/lib.rs` (2,852) | `activity.rs`, `vad_feed.rs`, `format_stability.rs`, `finalize.rs`, `macos.rs`, `linux.rs` (cpal) | §4.5 |
| `crates/capture-system-audio/src/` | add `pipewire/{graph, identity, session}.rs` behind the existing `SystemAudioCaptureSession`/`SegmentHooks` seam | §4.5 |
| `apps/desktop/src-tauri/src/native_capture_metadata.rs` (838) | `native_capture/active_window/{mod, macos}.rs`, `active_window/linux/{kwin, gnome, wlr, sway_ipc, niri_ipc, x11}.rs`; new `native_capture_browser_url_atspi.rs` | §4.2, §4.3 |
| `crates/capture-metadata/src/lib.rs` (1,051) | `browser_catalog.rs` with per-OS ids | §4.3 |
| `apps/desktop/src-tauri/src/native_capture.rs` (3,648) | `native_capture/system_events/{mod, macos, linux}.rs`, `native_capture/permissions/{mod, macos, linux}.rs`, `app_catalog/{mod, macos, linux}.rs` | §4.2, §4.9, §4.10 |
| `native_capture/privacy.rs` | remove the whole-file macOS gate; only the ScreenCaptureKit filter step stays macOS-only | §4.2 |
| `native_capture_system_idle.rs`, `native_capture/activity.rs:60-68` | idle dispatcher plus a new `native_capture/linux_session/`; **delete the duplicate stub** in `activity.rs` | §4.9 |
| `crates/app-infra/src/` | new `platform_keyring_linux.rs`; move `capture_index_key_store.rs:362-447` into `capture_index_key_store/macos_legacy.rs`; `secret_vault/key_source.rs`; Linux branch in `machine_id.rs` | §4.8, §4.12 |
| `crates/ocr/src/lib.rs` (1,719) | `apple_vision.rs`, `models.rs`; delete dead `app-infra/src/processing/{apple_vision, engine}.rs` | §4.4 |
| `crates/speaker-analysis` | target-split speakrs dependency; `cfg` for the CPU mode; per-OS file manifest | §4.6 |
| `apps/desktop/src-tauri/src/windows.rs` (2,006) | `windows/macos_panel.rs`, `windows/macos_app.rs`, `windows/linux.rs` | §4.11 |
| `keyboard_bindings.rs` (1,420) | `keyboard_bindings/linux_portal.rs` | §4.11 |
| `apps/desktop/src-tauri/build.rs`, `scripts/openblas-build-env.sh` | Linux static-Fortran branch; `.so` lookup | §4.6 |
| new | `tauri.linux.conf.json`, `linux/day.mnema.metainfo.xml`, `packaging/aur/PKGBUILD`, `.github/workflows/linux-build.yml`, `scripts/stage-linux-release-artifacts.sh`, GNOME extension source (e.g. `apps/gnome-extension/`) | §4.2, §4.12, §4.13 |

**ADRs this needs:**
- **Linux privacy exclusion promise:** amends ADR 0006.
- **Linux key store:** Secret Service with explicit file opt-in, amends ADRs 0012/0057.
- **Linux audio format:** Opus, amends the m4a invariant in CLAUDE.md and ADR 0052.
- **Linux codec:** H.264 via OpenH264 vs AV1.
- **Every Linux browser uses the Accessibility strategy:** extends ADR 0039.
- **ort embedding backend on Linux:** only if candle on the CPU fails the benchmark, amends ADR 0037.

## 7. What degrades or is dropped on Linux, and how the UI shows it

**One rule from `SUPPORTS.md`: never downgrade silently.** Every row below comes from the capability report (§6). Settings shows a degraded capability in plain words, and onboarding only asks for what the platform actually needs.

| Feature | On Linux | UI handling |
|---|---|---|
| Apple Vision OCR | dropped | PaddleOCR is the default. The picker lists only providers available on this machine. Never switch a user's provider silently (ADR 0004). |
| Apple Speech | dropped | Hidden. Parakeet int8 is the default. Whisper medium and larger are labelled "slow on CPU" unless a GPU build is detected. |
| Per-app screen exclusion | GNOME: unavailable. KDE/Hyprland: possibly full. Elsewhere: "pause while in front" | Wording per capability. **Full:** as macOS. **FocusGated:** "Excluded apps pause screen recording while they're in front. If one is visible behind another window, it is recorded." **Masked (X11):** "Excluded windows are blacked out." **Unavailable:** "This desktop doesn't let apps hide other apps from recordings. Use Pause or Delete Recent Capture." Hide the tray's "Exclude Current App" when the front app is unknown. |
| Per-app audio exclusion | heuristic; impossible on PulseAudio-only systems | If filtering is on and the privacy list isn't empty, system audio is turned off on PulseAudio-only systems with "Can't filter per-app audio on this system". |
| OS consent for mic and system audio | doesn't exist | Hide those permission rows. Show one disclosure: "Linux doesn't ask apps for microphone or system-audio permission. Turn these off here to stop recording them." |
| Permission helper window and "Open System Settings" | removed | One action, "Choose screens", which reopens the portal picker. No relaunch after granting. |
| Restore failed at an unattended start | new state | Cancel the dialog after about 2 s. Status bar: "Screen recording paused — your screens changed. **Choose screens**". |
| User pressed Stop in the desktop indicator | new state | "Screen recording stopped from the desktop's sharing menu. **Resume**". Never retry silently, which would just re-prompt. |
| Multiple monitors | one picked monitor (same as macOS today) | "Records the monitor you picked" plus "Change monitor…". |
| Accessibility onboarding step | removed for v1 | Replaced by an opt-in "Read browser URLs" toggle that explains what it changes system-wide. For Chromium browsers started earlier: "Restart Chrome/Brave/… to capture URLs." |
| App tracking on GNOME | needs the extension | "App tracking needs the Mnema GNOME extension — Install (requires log out)". Without it, timeline groups show "Unknown app" and recording still works. |
| Quick Recall | normal window, not a non-activating panel | No UI change; documented. |
| Global shortcuts | desktop-managed, or bound by hand | "Your desktop manages these shortcuts — Choose keys…". Where no portal exists, show `mnema://…` commands to copy into the desktop's shortcut settings. |
| Tray | menu only; GNOME needs an extension | One-time notice when no tray host is running. The app stays fully usable without a tray. |
| "Keep on top" (developer window) | ignored on Wayland | Hide the toggle on Wayland. |
| Updates | admin password for deb/rpm; package manager for AUR | Show "Updating needs your administrator password" before the prompt. For package-manager installs: "Updates come from your package manager". The Update Window notice stays. |
| Secret isolation | same-user readable | Settings → Privacy names the store and adds: "Other apps running as you can read this key on Linux." If no keyring: a blocking choice between "Set up a keyring" and "Use a protected file (less secure)". |
| Large embedding models | very slow on CPU | Mark "Not recommended without a GPU", or hide them. |
| CPUs without AVX2 | Parakeet and speakrs crash | Startup check. Turn them off with a typed "unsupported CPU" reason. |
| Recording indicator | always visible on GNOME/KDE | Onboarding: "Your desktop shows a screen-sharing icon the whole time Mnema records. That's expected." |
| Flatpak/Snap | not offered in v1 | — |
| Webview cache purge, render-idle gate | not ported unless a soak test shows growth | Render-idle already falls back safely when its events never fire. |

## 8. Phased plan

**The plan now lives in its own file: [`docs/linux-port-plan.md`](linux-port-plan.md).** It was moved there on 2026-11-02 and updated for the owner decisions in §10, so an agent picking up a slice reads one short file instead of this report. Each slice gives its goal, what to read, steps, what not to do, files, verify commands and dependencies.

| Phase | Outcome | Slices |
|---|---|---|
| 0 | Builds and launches on Linux; nothing waits on a decision | P0.1–P0.4 |
| 1 | Records the screen and makes it searchable | P1.1 media stack, P1.2 portal capture (with the desktop-Stop behaviour), P1.3 session events, P1.4 key store, P1.5 OCR default (benchmark first), P1.6 onboarding, P1.7 shell polish, P1.8 embedding gate |
| 2 | Parity features | P2.1 front-app backends, P2.2 privacy exclusion, P2.3 browser URLs (opt-in), P2.4 mic + Opus, P2.5 system audio, P2.6 CPU transcription, P2.7 shell parity, P2.8 Ollama `num_ctx`, **P2.9 COSMIC front-app backend** (new) |
| 3 | Packaging, licensing and release | P3.1 licence fingerprint, P3.2 bundle config + `mnema-desktop`, P3.3 release pipeline, P3.4 AUR, P3.5 docs, **P3.6 uncovered-build banner + safe rollback** (new, every platform) |

Slices that can start right away, in parallel: P0.1, P0.2, P2.8, and the banner part of P3.6.

## 9. Benchmark plan

**A VM is enough for:**
- builds;
- unit and headless tests (D-Bus, keyring, PipeWire null sinks, Xvfb launch);
- portal consent and restore-token behaviour on a GNOME or KDE VM with virtio-gpu;
- onboarding flows.

**A VM is not enough for anything about speed or power:** GPU paths, compositor cost, battery and thermals all need real hardware.

**Hardware matrix:**
- an 8-core AVX2 laptop (e.g. Ryzen 7 7840U or i7-1360P) running GNOME and KDE Wayland;
- a 4-core Intel laptop;
- an AVX-512 desktop (Zen 4) with an NVIDIA GPU on the proprietary driver;
- one older pre-Haswell machine, to confirm the AVX2 check works;
- an M-series Mac as the reference.

**Linux measurement tools (the macOS harness uses macmon/powermetrics/vmmap):**
- `turbostat` or `powerstat` (RAPL) for package power;
- `pidstat -u -r -h 1` for each process;
- `smem` (PSS) or `/proc/<pid>/smaps_rollup` for memory;
- `perf stat` for wakeups.

Always include `gnome-shell` / `kwin_wayland`, `pipewire`, `wireplumber`, `xdg-desktop-portal*` and `WebKitWebProcess`. On macOS, much of the cost lands in other processes (`scripts/bench-day/collect.sh:24-27`), and the same holds on Linux.

| # | Benchmark | Measure | Target / decision | Section |
|---|---|---|---|---|
| 1 | Restore-token durability | re-prompts across reboot, logout, 20× lock/unlock, suspend/resume, monitor hotplug, portal restart; on GNOME 46/48/50, KDE 6.4/6.8, Sway + xdpw, Hyprland + xdph | 0 after the first grant on GNOME/KDE | §4.1, §4.10 |
| 2 | Capture cost | whole-system CPU and package power at 0.5 / 2 / 10 fps × 720p / 1080p / native; shared memory vs DMA-BUF; Intel / AMD / NVIDIA | ≤ 5 % of one core at defaults | §4.1 |
| 3 | Frame-rate floor | frames delivered on a static screen; does the compositor honour < 1 fps? | decides whether we drop frames ourselves | §4.1 |
| 4 | Encode | OpenH264 ms/frame and MB per 5-minute segment against the macOS bitrate presets; rav1e for comparison | decides the codec fallback | §4.1 |
| 5 | Preview latency | exact preview p50/p95; scrub batch per 5-minute segment | within the 5 s / 2 s timeouts | §4.1 |
| 6 | OCR | ~100 real frames (1080p / 1440p / 4K; light and dark; IDE, terminal, browser, chat, sheets; 9–11 px text; CJK, Cyrillic, Arabic). Character error rate, word recall, p50/p95 latency, CPU-seconds per frame, peak RSS, frames OCR'd per minute under the real budget over 30 minutes. MNN at 1/2/4 threads; v5-mobile vs v6 tiny/small; Tesseract fast/best ± inversion. Reference: Vision on a Mac | p50 ≤ 1.0 s; recall ≥ 90 % of Vision-accurate | §4.4 |
| 7 | Embeddings | anchors/s, peak RSS, CPU %, time to process 1,000 at duty 3.0; candle 0.10.2 vs 0.11.1; `RAYON_NUM_THREADS` 4/8/all; ort prototype | ≤ 30 min per 1,000, ≤ 1.2 GB | §4.6 |
| 8 | Speaker detection | real-time factor, error rate vs CoreML on VoxConverse dev, RSS per 180 s chunk | ≥ 10× real time, error rate within 1 point | §4.5, §4.6 |
| 9 | Transcription | Parakeet int8/fp32 and Whisper base/small/medium real-time factor; CPU vs Vulkan | Parakeet ≥ 15× | §4.5 |
| 10 | Audio capture | 1-hour soak: CPU, wakeups, xruns; exclusion drill matrix | ≤ 1 % of one core; 0 leaks | §4.5 |
| 11 | Session events | suspend mid-segment ×10 (tail file opens); delay hold visible only while recording; idle with video playing | 10/10 | §4.9 |
| 12 | Browser URL | read latency cold and warm; Firefox/Chrome CPU and RSS with accessibility off vs on; cost to other apps | ≤ 400 ms per read; accessibility cost acceptable? | §4.3 |
| 13 | Webview | WebKitWebProcess RSS over 30 minutes of scrubbing; timeline frame rate with `backdrop-filter` | ≤ 300 MB growth; decides whether to port the cache purge | §4.11 |
| 14 | Local LLM | `llama-bench -p 512 -n 128` for 8B Q4 on CPU, 780M-class iGPU via Vulkan, RTX 3060-class | decides the "recommended hardware" copy | §4.7 |
| 15 | Binary hygiene | `ldd`, `objdump -T` glibc floor, `objdump -d \| grep zmm` | no undeclared libraries; glibc ≤ 2.39 | §4.6, §4.13 |

## 10. Open questions and unknowns

### Owner decisions (settled 2026-11-01)

All twelve questions were answered in a Q&A with the owner. The ADRs named below record the ones that change architecture or a promise; the rest live here. Where a subsystem's "(l) Open questions" list in §4 overlaps one of these, the decision here wins. §1, §4.1, §4.2, §4.12 and §4.13 have been edited to match; the other subsystem lists were left as written.

1. **Build floor: Ubuntu 24.04** (glibc 2.39, PipeWire 1.0).
   - Build deb and rpm in an `ubuntu:24.04` container. §4.12 and §4.13 have been updated to match.
   - **Supported distros:** Ubuntu 24.04+, Linux Mint 22+, Pop!_OS 24.04+, Fedora 42+, Debian 13+, and Arch (rolling).
   - **Arch** goes through an AUR `mnema-bin` package that repackages the .deb. Release CI bumps it. The in-app updater is off there.
   - Pop!_OS 24.04 ships the COSMIC desktop by default, so supporting Pop!_OS means supporting COSMIC (see 12).
2. **Video: H.264 via OpenH264.**
   - Sign the Via LA H.264 licence (free under 100k units a year) before the Linux release, after a lawyer reviews it.
   - AV1 (rav1e/dav1d) stays the fallback if the licence step stalls.
   - Linux media work starts now; only the encoder crate choice waits on the licence.
3. **Privacy exclusion: ship "pause while an excluded app is in front"**, with plain wording for each desktop's limit. It amends ADR 0006 for Linux; see ADR 0060.
   - **Full:** KDE 6.6+ and Hyprland, if the per-window hiding spike works.
   - **Pause while in front:** GNOME with the Mnema extension, KDE otherwise, and Sway/wlroots and COSMIC.
   - **Masked:** X11.
   - **Unavailable:** anywhere the front app is unknown, for example GNOME without the extension.
   - **Correction to §1/§4.1:** GNOME is "pause while in front" once the extension is on. It is only *impossible to hide at capture time*.
4. **The always-visible screen-sharing indicator is accepted.**
   - A Stop from the desktop is a normal event, not an error. It ends the screen only; the microphone and system audio keep recording. The app shows "Screen recording stopped by your desktop. Audio is still recording." with **Resume screen** and **Pause everything** (ADR 0060).
5. **Audio: Opus in Ogg on Linux, m4a/AAC on macOS** (ADR 0061).
   - SegmentPlanner gets a per-platform extension constant.
   - Fix the ~38 files that match `m4a`, the MIME map, and the Deepgram Content-Type.
   - Decode via symphonia + libopus + rubato.
   - The CLAUDE.md m4a line and ADR 0052 get a Linux note.
6. **No keyring running: a blocking choice, never silent** (ADR 0062).
   - The choice is "Set up a keyring" or "Store keys in a file only you can read (less secure)". The file lives in `$XDG_DATA_HOME/day.mnema/keys/` (0700/0600) and never in saveDirectory.
   - The prompt says the file isn't encrypted on disk.
   - The licensing file adapter leaves `cfg(debug_assertions)`.
   - TPM sealing is deferred.
7. **Browser URLs: one opt-in "Read browser URLs" setting**, off by default, with no extension in v1 (ADR 0063; ADR 0013 stands).
   - It explains the accessibility change first. Turning it off undoes exactly what Mnema changed.
   - Firefox/Zen get full parity.
   - Chromium is supported only if the hardware test passes without `--force-renderer-accessibility`; otherwise it is "not available on Linux".
8. **GNOME extension: bundle and side-load in v1**, and publish to extensions.gnome.org later.
   - Install takes one click plus a log-out.
   - Settings states the gap without it.
   - Porting the extension to each new GNOME release goes on the release checklist.
9. **Update Window on package managers: banner, no lock, plus a safe rollback** (ADR 0064).
   - Package channels serve the latest build to everyone, as Sublime Text, JetBrains and Sketch do.
   - A build newer than the owner's window keeps recording and shows a permanent "This version isn't covered" banner with Renew and Roll back. The licence is refreshed once before it shows.
   - Rollback uses the built-in updater with a downgrade comparator (`tauri-plugin-updater` 2.10.1 `version_comparator`). On Arch it shows a `pacman -U` command instead.
   - Rollback is offered only when the covered build knows every DB migration on this machine, because an older build refuses to start on newer migrations (`crates/app-infra/src/db.rs:20`).
10. **Trial abuse via a `/etc/machine-id` reset: accepted for v1.**
    - No server-side per-IP limit, because it would refuse honest users behind shared addresses.
    - Watch Linux trials against Linux sales in the licensegate dashboard.
11. **PaddleOCR: hidden for an unknown reason** (`apps/desktop/src-tauri/src/ocr_models.rs:493-497`, since `ae825586`).
    - The owner doesn't recall why, so treat the reason as unknown.
    - Benchmark PaddleOCR against Tesseract on real Linux screenshots first. Default to the winner and keep the other selectable.
12. **Scope.**
    - **x86_64 only in v1.** aarch64 comes later on demand; the arm64 jobs in §4.13 are deferred.
    - **No Flatpak in v1**, and the design doesn't plan for it.
    - **COSMIC is supported in v1** with its own front-app backend (`cosmic-toplevel-info`). The known limit: COSMIC doesn't set `LockedHint`.
    - **The CLI owns the `mnema` command.** The GUI binary is `mnema-desktop` on Linux (Linux-only `mainBinaryName`), and packages install the CLI as `mnema` on `PATH`.

### Needs hands-on testing (no amount of reading settles these)

- **Restore tokens:** how they behave across reboot, monitor changes and lock on each desktop; whether a restore attempt while locked burns the token (one unverified report).
- **COSMIC portal:** whether `xdg-desktop-portal-cosmic` honours `persist_mode=2` restore tokens at all. If not, COSMIC users get a share prompt on every launch.
- **OCR default (decision 11):** PaddleOCR against Tesseract on real Linux screenshots, using the repo's OCR benchmark harness.
- **Whether GNOME 49/50 lets users hide the screen-sharing indicator.**
- **KDE per-window hiding:** whether `ExcludeFromCapture` (6.6+) applies to monitor casts and can be set from a script; the same for Hyprland `noscreenshare` with xdph.
- **Chromium URLs:** whether the web document's `URI` appears with only start-up accessibility, or needs `--force-renderer-accessibility`.
- **Flatpak audio:** whether a Flatpak client may link other apps' PipeWire nodes (only matters if Flatpak is ever in scope).
- **Effect nodes:** how reliably EasyEffects, echo-cancel and filter-chain nodes can be told apart from real app streams.
- **Lock signal:** whether swaylock and hyprlock set logind `LockedHint`.
- **logind sleep delay** (`InhibitDelayMaxUSec`) against the time the Linux writer needs to finish a segment.
- **Prebuilt binaries:** the glibc floor of pyke's static ONNX Runtime and the MNN prebuilt; whether pyke's static ORT needs `libc++` or `libstdc++`.
- **Real Linux CPU speeds** for PaddleOCR, speakrs, Whisper and candle embeddings. None were measured anywhere; every number in this report for these is an estimate.
- **WebKitGTK memory:** whether its footprint grows under scrubbing like WKWebView's.
- **Tray icon colour** on dark panels; whether a built `.deb` registers `mnema://`; AAC playback in a clean install if m4a stays.
- **Ollama context truncation:** whether it already affects 16 GB Macs. Check for "truncating input prompt" in its logs.

---

## Appendix: how this report was made

- **Research:** 15 parallel research passes, each read-only on the repo: an inventory, the macOS baseline, and one per subsystem.
- **Compile checks:** run on a scratch copy of the repo with a separate build directory, so no build script or lockfile touched the working tree.
- **Hand checks:** surprising claims were re-checked by hand before inclusion — the licensing bypass, the speakrs `compile_error!`, the ungated title-bar call, the ggml native default, and the updater's deb/rpm support (`tauri-plugin-updater-2.10.1/src/updater.rs:1049-1059`).
- **One conflict between passes resolved:** the shell pass said only AppImage can self-update. The updater source shows `install_deb`/`install_rpm`, so §4.12 is right and §4.11's updater row is corrected here.
- **Dates:** external facts are as of late 2026. Crate versions were read from crates.io on 2026-11-01 and change quickly.
