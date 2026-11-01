# Mnema on Linux: implementation plan

_Updated 2026-11-02. It reflects the twelve owner decisions recorded in [`linux-port-report.md`](linux-port-report.md) §10 and ADRs 0060–0064._

The report holds the evidence; this file holds the work. Each slice below is meant for one agent or engineer working alone. A slice gives its goal, what to read, the steps, what not to do, the files involved, how to verify it, and what it depends on. Section numbers like "§4.1" point into the report. Effort is S (days), M (1–2 weeks) or L (3+ weeks), taken from the report's §4.

## Rules for every slice

- Follow `CLAUDE.md` (it is a symlink to `AGENTS.md`). Never log captured content or secrets.
- Keep every code file at 800 lines or fewer. If a file you touch is already over (for example `app_updates.rs`, 1,641 lines), put new logic in a new module; don't grow the file.
- macOS behaviour must not change unless the slice says so. Keep the macOS CI green.
- Update `SUPPORTS.md` in the same change whenever Linux behaviour changes.
- Never delete or edit an applied DB migration.
- New `MNEMA_*` environment variables go into `turbo.json` `passThroughEnv`.

## Decisions every slice assumes

| Topic | Decision | Record |
|---|---|---|
| Distros | Ubuntu 24.04+, Mint 22+, Pop!_OS 24.04+ (COSMIC), Fedora 42+, Debian 13+, Arch via AUR | report §10.1 |
| Architecture | x86_64 only in v1 | §10.12 |
| Desktops | KDE Plasma 6.4+ (reference), GNOME 46+ with the bundled extension, COSMIC, plus wlroots and X11 on a best-effort basis | §1, §10.12 |
| Video | H.264 via OpenH264, with the Via LA licence signed before release; AV1 is the fallback | §10.2 |
| Audio | Opus in Ogg on Linux, m4a on macOS | ADR 0061 |
| Privacy exclusion | Capability ladder: Full / pause while in front / Masked / Unavailable | ADR 0060 |
| Desktop Stop | Ends the screen only; audio keeps recording; the message says so | ADR 0060 |
| Secrets | Secret Service, with an explicit owner-only file fallback that is never silent | ADR 0062 |
| Browser URLs | One opt-in "Read browser URLs" setting; no extension | ADR 0063 |
| Update window | Permanent banner, no lock, rollback only when safe | ADR 0064 |
| Command name | The CLI is `mnema`; the GUI binary is `mnema-desktop` on Linux | §10.12 |
| Not in v1 | Flatpak, Snap, aarch64 | §10.12 |

## Phase overview

| Phase | Outcome | Slices |
|---|---|---|
| 0 | Builds and launches on Linux; nothing waits on a decision | P0.1–P0.4 |
| 1 | Records the screen and makes it searchable | P1.1–P1.8 |
| 2 | Parity features | P2.1–P2.9 |
| 3 | Packaging, licensing and release | P3.1–P3.6 |

Slices that can start right away, in parallel: P0.1, P0.2, P2.8, and the banner part of P3.6.

---

## Phase 0: Builds and launches on Linux

Exit criteria for the whole phase:
- `cargo check --workspace --all-targets --locked` and the desktop crate check pass in an `ubuntu:24.04` container.
- `cargo test` passes on the Linux-clean crates (about 1,240 tests).
- `bun run check` passes.
- A debug build opens its main window on a GNOME VM and a KDE VM, with capture showing "not supported on this platform yet".
- The macOS CI stays green.

### P0.1 Fix `cfg` drift (S)
- **Goal:** the library crates and the desktop crate compile on Linux.
- **Read:** report §2 (compile probe and the 14 desktop errors).
- **Steps:**
  - Ungate `mod equivalence` and `normalize_candidate_text`.
  - Gate `title_bar_style`/`hidden_title` with `#[cfg(target_os = "macos")]`.
  - Move the plain helpers shared by `lifecycle.rs`, `segments.rs`, `runtime.rs`, `privacy.rs` and `output` out of their macOS gates, or add `not(macos)` stubs, until `mnema` type-checks.
  - Repeat until the next wave of errors is clear. 14 errors is a lower bound.
- **Don't:** change macOS behaviour, or add Linux capture code.
- **Files:** `crates/capture-screen/src/lib.rs:20,61`, `crates/ocr/src/lib.rs:730`, `apps/desktop/src-tauri/src/windows.rs:499-503`, `apps/desktop/src-tauri/src/native_capture/*`.
- **Verify:** `cargo check -p capture-screen -p ocr -p app-infra -p cli` on Linux; desktop crate `cargo check` on Linux with `webkit2gtk-4.1` installed (run `bash scripts/prepare-mnema-cli-sidecar.sh debug` first); the full macOS `type-check.yml` is green.
- **Depends:** nothing.

### P0.2 Native ML builds (S)
- **Goal:** speakrs, Whisper and OpenBLAS build portably.
- **Read:** §4.6.
- **Steps:**
  - Move speakrs' `coreml` feature into a macOS-only dependency entry.
  - Choose `ExecutionMode::Cpu` off macOS and add a CPU file manifest.
  - Add a Linux static-Fortran branch to `build.rs`, and a `.so` branch to `scripts/openblas-build-env.sh`.
  - Set `GGML_NATIVE=OFF` and `OPENBLAS_DYNAMIC_ARCH=1` in release environments. Add them to `turbo.json` if they pass through turbo.
- **Don't:** turn on CUDA, ROCm or Vulkan.
- **Files:** `crates/speaker-analysis/Cargo.toml:38`, `crates/speaker-analysis/src/providers/speakrs.rs:189-192`, `crates/speaker-analysis/src/lib.rs:63-80`, `apps/desktop/src-tauri/build.rs:45-47`, `scripts/openblas-build-env.sh`, `turbo.json`.
- **Verify:** `cargo check -p speaker-analysis --features speakrs` and `cargo check -p audio-transcription --features local-whisper,parakeet-onnx` on Linux; `ldd` on the binary shows no `libgfortran`.
- **Depends:** nothing.

### P0.3 Linux CI allowlist job (S)
- **Goal:** catch regressions in shared crates now.
- **Read:** §4.13 (job list, apt list, YAML draft; the arm64 jobs there are deferred).
- **Steps:**
  - Add `linux-build.yml` with a `check` job in `container: ubuntu:24.04` on `ubuntu-24.04` (x86_64 only). It runs `bun run check` and `cargo test` on the allowlist. Add crates as P0.1 and P0.2 land.
  - Fix `macos-release.yml:304` so it patches only `darwin-*` signatures in `latest.json`.
- **Don't:** add the release job or any arm64 job yet.
- **Files:** `.github/workflows/linux-build.yml` (new), `.github/workflows/macos-release.yml`.
- **Verify:** the workflow is green on a PR; the macOS release dry-run is unchanged.
- **Depends:** P0.1, to widen the allowlist.

### P0.4 Shared pieces from the Windows branch (M)
- **Goal:** one copy of the non-Apple building blocks.
- **Read:** `git diff main...origin/windows-key-stores-and-final-gaps -- crates/capture-screen/src/frame_schedule.rs crates/media-decode`; report §6 (capability report).
- **Steps:** land `frame_schedule.rs`, `media-decode` (only if it fits the Opus/H.264 decode plan in §4.5), and the capability-report type on `main`.
- **Don't:** merge Windows-only capture code.
- **Files:** `crates/capture-screen/src/frame_schedule.rs`, `crates/media-decode/` (new on main), `crates/capture-types` (capability report).
- **Verify:** macOS tests are green; the new crates build on Linux.
- **Depends:** P0.1.

---

## Phase 1: Records the screen and makes it searchable

Exit criteria. These are proposed targets measured against the macOS baseline in report §3; check them on the hardware in §9.

- **Reliability:**
  - 8 hours of recording at defaults (720p, 0.5 fps, 60 s segments) on GNOME 46+, KDE 6.4+ and COSMIC.
  - No permission prompts after the first one, through 20 lock/unlock cycles, 3 reboots and 3 suspends with the same monitors. On COSMIC, record whether restore tokens work at all.
  - The tail segment survives suspend: the file opens and has a video track in 10 out of 10 runs.
- **Cost:**
  - Capture overhead at defaults (Mnema plus compositor plus PipeWire, not counting OCR) is ≤ 5 % of one core on average on a 4-core laptop.
  - Screen disk use is within ±25 % of macOS at the same settings (about 168 MB video plus 47 MB OCR index per day, `apps/desktop/src/lib/onboarding/disk-estimate.ts:1-30`).
- **OCR:**
  - Median ≤ 1.0 s per 1080p frame on an 8-core x86 laptop.
  - Average OCR CPU no higher than the macOS 20 % duty of its 2 threads.
  - Word recall ≥ 90 % of Vision-accurate on the §9 frame set.
  - p95 time from capture to searchable ≤ 2 min.
- **Embeddings (only if enabled):** 1,000 items in ≤ 30 min at ≤ 1.2 GB memory; otherwise use the ort fallback.
- **Webview:** WebKitWebProcess memory grows ≤ 300 MB over 30 minutes of timeline scrubbing.

### P1.1 Media stack (M)
- **Goal:** replace AVAssetWriter and AVAssetImageGenerator with an H.264 path.
- **Read:** §4.1 "Video writing" and "Frame preview"; §10 decision 2.
- **Steps:**
  - `openh264` encoder plus `mp4-atom` writer, keeping the `.mov` file name.
  - Build the frame index from the writer's own sample table.
  - Exact and scrub preview decode in the Linux functions of `frame_preview.rs`.
  - Keep the encoder behind one small trait so AV1 (rav1e/dav1d) can replace it if the licence step stalls.
- **Don't:** use GStreamer or FFmpeg; don't rename files.
- **Files:** `crates/capture-writers/`, `apps/desktop/src-tauri/src/app_infra/frame_preview.rs:1853-1907`.
- **Verify:** unit tests encode synthetic BGRA, reopen the file, check sample times and decode the exact frame. Preview latency stays within the 5 s / 2 s timeouts (`frame_preview.rs:33,40`).
- **Depends:** P0.1. The Via LA licence must be signed (after a lawyer's review) before release, not before coding.

### P1.2 Portal + PipeWire capture (M)
- **Goal:** live screen frames on Wayland, with X11 as a fallback.
- **Read:** §4.1 (c), §4.10 (c); ADR 0060 decision 5 (desktop Stop).
- **Steps:**
  - Register `day.mnema` with the portal Registry before any other portal call.
  - `SelectSources` with monitor sources and `persist_mode=2`. Rotate and save the restore token after every start.
  - Shared-memory BGRx frames on a PipeWire thread; CPU downscale; JPEG export; equivalence and activity fingerprints.
  - Liveness from the session `Closed` signal, stream state and logind `LockedHint`, mapped to the ADR 0021 suspension.
  - **Desktop Stop:** when the portal session closes because the user pressed Stop, stop the screen only, commit the segment, and keep the microphone and system audio running. Show "Screen recording stopped by your desktop. Audio is still recording." with **Resume screen** (re-runs the portal start with the saved token) and **Pause everything**. Never retry on your own.
  - `xcap` backend for X11.
- **Don't:** use DMA-BUF in v1; don't restore while the screen is locked; don't treat a desktop Stop as an error.
- **Files:** `crates/capture-screen/` (new Linux backend), `apps/desktop/src-tauri/src/native_capture/lifecycle.rs`, `native_capture/runtime.rs`.
- **Verify:** the restore-token durability matrix in §9 (GNOME, KDE and COSMIC); segment rotation tests; pressing Stop leaves audio segments growing.
- **Depends:** P1.1, P1.3 (lock state).

### P1.3 Session events (M)
- **Goal:** sleep, lock, idle and display hotplug.
- **Read:** §4.9.
- **Steps:**
  - A zbus `linux_session` module: logind delay hold, `LockedHint`, idle readers (Mutter, ext-idle-notify v2, XScreenSaver), GDK monitor signals.
  - COSMIC doesn't set `LockedHint`. Fall back to the idle reader there and document the limit.
  - Delete the duplicate idle stub.
  - Fix `volume_root` with an `st_dev` check.
- **Don't:** add power or thermal logic.
- **Files:** `apps/desktop/src-tauri/src/native_capture/activity.rs:60-68`, `apps/desktop/src-tauri/src/native_capture/disk_space.rs:174`, new `linux_session` module.
- **Verify:** suspend mid-segment; `systemd-inhibit --list`; idle with video playing.
- **Depends:** P0.1.

### P1.4 Linux key store (M)
- **Goal:** the database key, vault master key and licence store live in the Secret Service.
- **Read:** §4.8; ADR 0062.
- **Steps:**
  - One `platform_keyring_linux.rs` on oo7 or the keyring v4 zbus store. Pick whichever needs no second zbus major version.
  - Wire it into the non-macOS branches of the three stores.
  - When no keyring answers, show the blocking choice: "Set up a keyring" (docs link) or "Store keys in a file only you can read (less secure)". The prompt says the file isn't encrypted on disk. The choice persists.
  - File mode writes to `$XDG_DATA_HOME/day.mnema/keys/` (directory 0700, files 0600), never inside the save directory.
  - Ungate the licensing file adapter from `cfg(debug_assertions)` for file mode.
- **Don't:** use keyutils; never treat "locked" as "missing" (that would mint a new key); never fall back silently; skip TPM sealing.
- **Files:** `crates/app-infra/src/platform_keyring_linux.rs` (new), `crates/app-infra/src/secret_vault.rs`, `crates/app-infra/src/capture_index_key_store.rs`, `crates/app-infra/src/license_token_store.rs`.
- **Verify:** `dbus-run-session` + `gnome-keyring-daemon --unlock` tests; the key survives a reboot; the CLI over SSH while locked gives the named error; file-mode permissions are 0700/0600.
- **Depends:** P0.1.

### P1.5 OCR default, benchmark first (S)
- **Goal:** choose the Linux OCR default from measurements, not assumptions.
- **Read:** §4.4; §10 decision 11 (the reason PaddleOCR was hidden is unknown).
- **Steps:**
  - Run `cargo run -p ocr --features paddle-rs --example paddle_ocr_benchmark` and the Tesseract equivalent on real Linux screenshots (§9 frame set).
  - Make `default_ocr_provider()` per platform and default to the winner. Keep the other selectable: add PaddleOCR to `is_desktop_selectable_ocr_provider` if it wins or ties.
  - Hide Vision off macOS. Pin or vendor MNN. Cache `TesseractAPI` per language.
- **Don't:** change the macOS default; don't unhide PaddleOCR on macOS without a separate decision.
- **Files:** `apps/desktop/src-tauri/src/ocr_models.rs:493-497`, `apps/desktop/src/lib/settings/panels/intelligence/Ocr.svelte:83`, `crates/ocr/src/tesseract.rs:179`, `crates/ocr/Cargo.toml`.
- **Verify:** benchmark numbers written into the PR description against the Phase 1 OCR targets; the settings picker shows the right engines on Linux.
- **Depends:** P0.1. Needs a real Linux laptop.

### P1.6 Onboarding and permissions (M)
- **Goal:** one consent step on Linux.
- **Read:** §4.10; ADR 0060 (indicator copy).
- **Steps:**
  - A `grantNeedsRelaunch` flag in `CapturePermissionsResponse`; gate `PermissionsScreen.svelte:191,199` on it.
  - `show_permission_helper` re-asks the portal on Linux; mic and system audio report `granted`.
  - Onboarding copy: "Your desktop shows a screen-sharing icon the whole time Mnema records. That's expected."
- **Don't:** keep any `x-apple.systempreferences:` path reachable on Linux.
- **Files:** `crates/capture-types/` (`CapturePermissionsResponse`), `apps/desktop/src/routes/onboarding/screens/PermissionsScreen.svelte`, `apps/desktop/src-tauri/src/windows/` (`show_permission_helper`).
- **Verify:** an onboarding run on GNOME, KDE and COSMIC VMs; `bun run check`.
- **Depends:** P1.2.

### P1.7 Minimum shell polish (S)
- **Goal:** the app looks and behaves right on Linux.
- **Read:** §4.11 (c) 1, 5–7, 11.
- **Steps:**
  - A platform class on `<html>`; drop the 78 px inset on Linux.
  - Replace hard-coded `⌘` with `formatShortcut`; fix `DevStrip.svelte:114`.
  - A tray icon that works on light and dark panels; CSS corners and shadow for undecorated windows.
- **Don't:** add GlobalShortcuts yet.
- **Files:** `apps/desktop/src/routes/+layout.svelte`, `apps/desktop/src/lib/debug/DevStrip.svelte`, `apps/desktop/src-tauri/src/status_bar.rs`, `apps/desktop/src-tauri/icons/`.
- **Verify:** a visual check on GNOME, KDE and COSMIC, in light and dark themes.
- **Depends:** P0.1.

### P1.8 Embedding benchmark gate (S, or M if it fails)
- **Goal:** decide whether semantic search ships enabled on Linux.
- **Read:** §4.6 (k).
- **Steps:** run `crates/semantic-search/examples/{smoke,thermal}.rs` on the harvested corpus. If it misses the bar, prototype the ort CPU backend and write its ADR.
- **Don't:** ship semantic search enabled by default without passing the bar.
- **Files:** `crates/semantic-search/`.
- **Verify:** the numbers against the Phase 1 embedding target.
- **Depends:** P0.2.

---

## Phase 2: Parity features

Exit criteria:
- **Screen exclusion:** across 100 scripted alt-tabs, no frame is captured while an excluded app is in front, on every supported desktop (GNOME with the extension, KDE, COSMIC, Sway/Hyprland). On KDE/Hyprland, if the spike succeeds, excluded windows never appear at all.
- **Audio exclusion:** the drill matrix passes with zero leaks (browser, Flatpak app, EasyEffects active, Bluetooth profile switch, PipeWire restart).
- **Audio capture cost:** mic plus system audio ≤ 1 % of one core.
- **Speed and accuracy:**
  - Parakeet ≥ 15× real time on an 8-core laptop.
  - Speaker detection ≥ 10× real time, with error rate within 1 point of CoreML on VoxConverse dev.
  - Active-app labels correct on ≥ 95 % of frames in a scripted app-switch test on each supported desktop.
  - Firefox URL captured within 2 s of a tab change, with the opt-in on.

### P2.1 Active-window backends: KDE, GNOME, wlroots, X11 (L)
- **Goal:** know the front app and its `.desktop` id on every supported desktop except COSMIC (that's P2.9).
- **Read:** §4.2; ADR 0060 decisions 3–4.
- **Steps:**
  - A KWin script over D-Bus.
  - The Mnema GNOME Shell extension, bundled for v1. Settings shows "App tracking needs the Mnema GNOME extension: Install (requires log out)". One click copies it to `~/.local/share/gnome-shell/extensions` and enables it. Without it, Settings says plainly that there's no app tracking or exclusion on GNOME.
  - The wlr window list plus Sway/niri/Hyprland IPC for the PID; `x11rb` for X11.
  - One trait, with the backend picked at startup. If a source stops reporting mid-session, the capability drops to Unavailable.
  - Store the `.desktop` id in the existing `bundle_id` fields (no migration). Icons come from the icon theme. Add Linux ids to the sensitive-app catalog.
  - Add "bump the GNOME extension's `shell-version`" to the release checklist in `docs/release-process.md`.
- **Don't:** rename database columns; don't publish to extensions.gnome.org yet.
- **Files:** new `apps/desktop/src-tauri/src/native_capture/front_app/` module (one file per backend), `apps/desktop/src-tauri/resources/gnome-extension/` (new), `crates/app-infra/src/user_context/capture_source.rs`.
- **Verify:** the scripted app-switch test on GNOME (with and without the extension), KDE, Sway and X11; unit tests for `.desktop` id resolution.
- **Depends:** P0.1, P0.4 (capability report).

### P2.2 Privacy exclusion on Linux (M)
- **Goal:** ADR 0060's capability ladder, stated plainly in Settings.
- **Read:** §4.1 "Live privacy exclusion", §7; ADR 0060.
- **Steps:**
  - A capability enum (Full / FocusGated / Masked / Unavailable), detected per session and exposed through the capability report.
  - Pause while an excluded app is in front, through a **new** transient, screen-only `CaptureSuspensionKind` variant that behaves like `DisplayUnavailable`: it never gives up and resumes on its own.
  - Settings wording per rung (report §7 strings). Unavailable points to Pause and Delete Recent Capture, and the tray's "Exclude Current App" is hidden when the front app is unknown.
  - Spike KWin `ExcludeFromCapture` and Hyprland `noscreenshare` against monitor casts; promote those desktops to Full only if the spike passes.
  - Tag frames as self-capture while a Mnema window has focus.
- **Don't:** reuse `CaptureSuspensionKind::PrivacyFilter` (it means "a filter apply failed" and escalates to a manual restart, `native_capture/runtime.rs:36-39`); don't claim Full anywhere the spike didn't pass.
- **Files:** `apps/desktop/src-tauri/src/native_capture/privacy.rs`, `native_capture/runtime.rs`, `apps/desktop/src/routes/settings/+page.svelte` (privacy section), `apps/desktop/src-tauri/src/status_bar.rs`.
- **Verify:** the 100 alt-tab script on each desktop; a unit test that the new suspension resumes when the excluded app leaves the front.
- **Depends:** P2.1, P2.9 (for COSMIC), P1.2.

### P2.3 Browser URLs via AT-SPI, opt-in (M)
- **Goal:** browser URLs on Linux behind one opt-in setting.
- **Read:** §4.3; ADR 0063.
- **Steps:**
  - `native_capture_browser_url_atspi.rs` (~300 lines, `atspi` 0.30).
  - A "Read browser URLs" setting, off by default. Before writing anything, explain that it turns on desktop accessibility and that Chrome-type browsers need one restart.
  - Record exactly what Mnema flipped (`IsEnabled`, `toolkit-accessibility`, `environment.d`) and undo exactly that when the setting is turned off.
  - Firefox and Zen: full parity. Chromium family: enable only if the hardware test shows the URL appears without `--force-renderer-accessibility`; otherwise show "not available on Linux".
  - Add Linux keys to the browser catalog.
- **Don't:** add a browser extension (ADR 0013 stands); don't change accessibility settings before the user opts in.
- **Files:** `apps/desktop/src-tauri/src/native_capture_browser_url_atspi.rs` (new), the browser catalog, the settings page.
- **Verify:** the Firefox 2 s tab-change test; turning the setting off restores the original values; the Chromium hardware test result goes in the PR.
- **Depends:** P2.1.

### P2.4 Microphone and Opus audio format (M)
- **Goal:** microphone capture and Opus/Ogg segments on Linux.
- **Read:** §4.5 (c) 1, 3, 4; ADR 0061.
- **Steps:**
  - A cpal 0.18 microphone.
  - An Opus/Ogg writer, trim, and a shared decoder (symphonia + libopus + rubato to 16 kHz).
  - A per-platform segment extension constant in SegmentPlanner. Fix the ~38 files that hard-code `m4a`, the MIME map, and the Deepgram `Content-Type` (derive it from the extension).
- **Don't:** change macOS output (m4a/AAC stays).
- **Files:** `crates/capture-microphone/`, `crates/capture-writers/`, SegmentPlanner, the files from `grep -rl m4a crates apps/desktop/src-tauri/src`.
- **Verify:** record, trim and decode round-trip tests; Deepgram sends `audio/ogg`; macOS tests unchanged.
- **Depends:** P0.4.

### P2.5 System audio via PipeWire links (L)
- **Goal:** system audio with per-app exclusion parity.
- **Read:** §4.5 (c) 2, (f) 1.
- **Steps:**
  - An always-running virtual target. Link only leaf `Stream/Output/Audio` nodes. Exclude by `pipewire.sec.pid` and process tree.
  - Rebuild on core error; zero watchdog.
  - On PulseAudio-only systems with a non-empty privacy list, turn system audio off with "Can't filter per-app audio on this system".
  - Write a Linux version of `docs/agents/system-audio-drills.md`.
- **Don't:** link filter, effect or loopback nodes.
- **Files:** `crates/capture-system-audio/` (new Linux backend), `apps/desktop/src-tauri/src/native_capture/system_audio.rs`.
- **Verify:** the drill matrix in the Phase 2 exit criteria.
- **Depends:** P2.4.

### P2.6 Transcription and speaker detection on CPU (S–M)
- **Goal:** Whisper, Parakeet and speakrs run on Linux.
- **Read:** §4.5 (c) 5–6, §4.6.
- **Steps:** connect the shared decoder to the Whisper, Parakeet and speakrs stubs and to `system_audio_speech_activity.rs:157`; add an AVX2 check; tune the speakrs chunk size; hide Apple Speech.
- **Don't:** enable GPU backends.
- **Files:** `crates/audio-transcription/`, `crates/speaker-analysis/`, `crates/app-infra/src/processing/system_audio_speech_activity.rs`.
- **Verify:** the Parakeet and speaker-detection speed targets on the §9 hardware.
- **Depends:** P0.2, P2.4.

### P2.7 Shell parity (M)
- **Goal:** shortcuts, Quick Recall, notifications and clock format.
- **Read:** §4.11 (c) 3–4, 8–10, 12; §4.9 (c) 5.
- **Steps:**
  - A GlobalShortcuts portal backend plus `mnema://` action links; single-instance forwards arguments.
  - Quick Recall sizing on Wayland.
  - A notify-rust reminder with actions.
  - The GNOME clock-format setting.
- **Don't:** use X11 key grabs on Wayland.
- **Files:** `apps/desktop/src-tauri/src/lib.rs` (plugins), `apps/desktop/src-tauri/src/windows/`, `apps/desktop/src/lib/` (shortcut formatting).
- **Verify:** shortcuts on GNOME 48+, KDE and COSMIC; `mnema://` links on desktops without the portal.
- **Depends:** P1.7.

### P2.8 Send `num_ctx` to Ollama (S, every platform)
- **Goal:** stop silent prompt truncation on local Ollama.
- **Read:** §4.7.
- **Steps:** pass `num_ctx` through rig `additional_params`.
- **Don't:** change cloud providers.
- **Files:** `crates/ai-runtime/src/lib.rs:573-582`, `crates/ai-runtime/src/agent_loop.rs:318-329`.
- **Verify:** `cargo test -p ai-runtime`; Ollama logs no "truncating input prompt" on a long chat.
- **Depends:** nothing. It can ship before the port.

### P2.9 COSMIC front-app backend (S–M)
- **Goal:** front-app tracking on COSMIC, so pause-while-in-front works on Pop!_OS.
- **Read:** §4.2 item 4; §10 decision 12; awatcher's COSMIC watcher (https://github.com/2e3s/awatcher) as prior art.
- **Steps:**
  - Bind `ext-foreign-toplevel-list-v1` plus COSMIC's `cosmic-toplevel-info` state extension for the "activated" state, on its own `wayland-client` connection and thread.
  - Implement the same front-app trait as P2.1 and register it for COSMIC at startup.
  - Resolve the `.desktop` id from `app_id`; get the PID through `/proc` if the protocol has none, or leave it empty.
- **Don't:** rely on `ext-foreign-toplevel-list-v1` alone (it has no "activated" state); don't touch the other backends.
- **Files:** `apps/desktop/src-tauri/src/native_capture/front_app/cosmic.rs` (new); the `cosmic-protocols` crate (new dependency; check its licence and pin it).
- **Verify:** the scripted app-switch test on a COSMIC machine (≥ 95 % correct labels); the 100 alt-tab exclusion script on COSMIC.
- **Depends:** P2.1 (the trait). Needs a COSMIC test machine.

---

## Phase 3: Packaging, licensing and release

Exit criteria:
- **Installs:** clean installs on Ubuntu 24.04/26.04, Debian 13, Fedora 42+, Pop!_OS 24.04 and Arch (AUR).
- **Updates:** an in-app update from version v to v+1 works for deb and rpm on the stable and preview channels. A tampered `.sig` is refused.
- **Update window:** a build newer than the owner's window shows the banner and keeps recording; Roll back appears only when the migration check allows it, and it works.
- **Licensing:** a trial is issued on Linux, and the activation hash stays the same across reboot and reinstall.
- **Binary hygiene:** `ldd` shows only declared dependencies, and the newest glibc symbol is ≤ 2.39.

### P3.1 Licence fingerprint (S). Must land before ANY public Linux build, preview included
- **Goal:** trials and activation work on Linux. Today non-macOS builds are silently unlicensed.
- **Read:** §4.12 (c), (d) 4; §10 decision 10.
- **Steps:**
  - Linux `hardware_uuid()` reads `/etc/machine-id`, falling back to `/var/lib/dbus/machine-id`.
  - `hardware_model()` reads DMI `sys_vendor` + `product_name`.
  - If the id is unreadable, keep the "never lock out" behaviour and log a Warn with no id value.
  - Remove the non-macOS "force Activated" branch.
- **Don't:** send the raw machine-id (licensegate already salts and hashes it); don't add server-side rate limits (decision 10).
- **Files:** `crates/app-infra/src/machine_id.rs:14-127`, `apps/desktop/src-tauri/src/licensing/adapter.rs:175-178`, `apps/desktop/src-tauri/src/licensing/trial.rs:166-171`.
- **Verify:** `cargo test -p app-infra machine_id`; trial issuance on a VM; the hash is stable across reboot.
- **Depends:** P0.1.

### P3.2 Bundle config and CLI naming (S–M)
- **Goal:** correct Linux packages, with the CLI owning `mnema`.
- **Read:** §4.12 (c), (d) 1–3, 8; §10 decision 12.
- **Steps:**
  - `tauri.linux.conf.json`: targets `deb`, `rpm` (AppImage only as a fallback); dependencies on WebKitGTK 4.1, ayatana-appindicator, libssl3, PipeWire and the portal packages; AppStream metainfo.
  - Linux-only `mainBinaryName: "mnema-desktop"`, so the `.desktop` launcher runs `mnema-desktop`.
  - Packages install the CLI as `/usr/bin/mnema`. Hide the "Install CLI" button on Linux.
  - The CLI launches the app with `gtk-launch day.mnema` or the binary path, not `xdg-open mnema`.
- **Don't:** ship Flatpak or Snap; don't change macOS bundle names.
- **Files:** `apps/desktop/src-tauri/tauri.linux.conf.json` (new), `apps/desktop/src-tauri/src/app_infra.rs:4355-4371` (CLI install path), `crates/cli/src/main.rs:1189-1199`, the settings CLI section.
- **Verify:** after a deb install, `which mnema` is the CLI and `mnema-desktop` opens the app; `mnema search` works against a running app; `mnema://` is registered.
- **Depends:** P0.1.

### P3.3 Release pipeline (M)
- **Goal:** tagged releases produce signed Linux packages alongside macOS.
- **Read:** §4.13 `release` job (x86_64 only).
- **Steps:**
  - tauri-action in an `ubuntu:24.04` container, x86_64 only.
  - Merge `latest.json` after the macOS job.
  - Add Linux assets to the promote workflow; a glibc floor check; GPG-signed `SHA256SUMS`; rpm signing.
  - Each release's updater manifest records the newest DB migration number it contains (needed by P3.6).
  - Publish an Arch package (`.pkg.tar.zst`) for every version, so Arch rollback has something to install.
- **Don't:** add arm64 builds.
- **Files:** `.github/workflows/linux-build.yml`, `.github/workflows/macos-release-promote.yml`, `docs/release-process.md`.
- **Verify:** a dry-run tag produces deb, rpm and Arch packages with valid signatures; `latest.json` has both platforms with correct signatures.
- **Depends:** P0.3, P3.2.

### P3.4 AUR `mnema-bin` (S)
- **Goal:** Arch users install and update through their AUR helper.
- **Read:** §4.12 (AUR); §10 decision 1.
- **Steps:**
  - A PKGBUILD that repackages the release `.deb` and verifies its checksum.
  - Release CI bumps `pkgver` and checksums and pushes to the AUR.
  - The built-in updater is off for AUR installs. Detect a pacman-owned install, or use a build flag.
- **Don't:** build from source in the PKGBUILD; don't let the in-app updater try to replace pacman-owned files.
- **Files:** `packaging/aur/PKGBUILD` (new), the release workflow.
- **Verify:** `makepkg -si` on a clean Arch VM; the app opens; the updater UI shows "updated by your package manager".
- **Depends:** P3.3.

### P3.5 Docs (S)
- **Goal:** the docs describe Linux as shipped.
- **Steps:**
  - A Linux column in `SUPPORTS.md` (capability per desktop, from report §5 and §7).
  - `docs/release-process.md`: Linux steps, the AUR bump and the GNOME extension `shell-version` bump.
  - A Linux addition to `docs/agents/capture-pipeline.md` for its quirks (restore tokens, desktop Stop, PipeWire link rules).
  - Mark ADRs 0060–0064 as implemented once their slices land.
- **Don't:** copy the report into the docs; link to it.
- **Files:** `SUPPORTS.md`, `docs/release-process.md`, `docs/agents/capture-pipeline.md`, `docs/adr/0060`–`0064`.
- **Verify:** every Linux row in `SUPPORTS.md` matches the shipped behaviour.
- **Depends:** the slices it documents.

### P3.6 Uncovered-build banner and safe rollback (M, every platform)
- **Goal:** a build newer than the owner's update window shows a permanent banner, never a lock, and offers rollback only when it's safe.
- **Read:** ADR 0064; `docs/licensing/CONTEXT.md` "Documented edge case — fresh install after Update Window lapse"; §10 decision 9.
- **Steps:**
  - Split today's `AvailableOutOfWindow` into two states: "a newer update is past your window" and "this running build is past your window". Today `running_build_window_gate` reuses one state and shows only a status line.
  - Before showing the banner, run one Receipt Refresh, so a just-renewed owner never sees it.
  - The banner: "This version isn't covered by your licence. Your updates ended on ⟨date⟩." with **Renew** and, when safe, **Roll back to ⟨covered version⟩**. It can be collapsed for the current launch and comes back on every launch. Recording keeps working.
  - Rollback on macOS, deb and rpm: point the updater at the covered version's manifest and allow the downgrade with `tauri-plugin-updater`'s `version_comparator` (2.10.1, `updater.rs:184`).
  - Rollback on Arch: show the `sudo pacman -U <covered package URL>` command and how to stop the next system update from upgrading it again.
  - Safety check: compare the covered build's newest migration number (from its manifest, P3.3) with the newest migration applied to the local DB. If the DB is ahead, hide Roll back and say: "Rolling back isn't possible because this version updated your data. Renew to keep it, or keep using it as is."
- **Don't:** lock recording or any feature; don't add `ignore_missing` to the migrator or make migrations backward-compatible; don't grow `app_updates.rs` (already 1,641 lines): put the new logic in a new module.
- **Files:** `apps/desktop/src-tauri/src/app_updates.rs:405-471` (the gate and decision helpers), a new `apps/desktop/src-tauri/src/app_updates/uncovered_build.rs`, `apps/desktop/src/lib/update-window.ts:55-65`, `crates/app-infra/src/db.rs:20` (read the applied migration high-water mark), the licensing banner UI.
- **Verify:** unit tests for the state split and the migration comparison; a manual test with a build date after `update_through` (banner shows, recording continues, Roll back hidden when the DB is ahead and working when it isn't); `bun run check`.
- **Depends:** the banner and state split depend on nothing and can ship now on macOS. Rollback depends on P3.3 (migration number in the manifest) and, for Arch, P3.4.
