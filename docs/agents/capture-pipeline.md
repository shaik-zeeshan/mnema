# Capture Pipeline — Agent Reference

Deep-dive quirks for the capture/storage/OCR/privacy system. Referenced from `CLAUDE.md`.

---

## Frontend / App Layout

- SPA-only: `apps/desktop/src/routes/+layout.ts` sets `ssr = false`; `svelte.config.js` uses `@sveltejs/adapter-static` with `fallback: "index.html"`.
- In production packaging the SPA entry can arrive as `"/index.html"` — normalize static-entry paths in the shell before route-gating.
- Tauri expects Vite on port `1420` with HMR on `1421`; `vite.config.js` hard-pins these and ignores `src-tauri/**`.
- Bundle identifier is `day.mnema` (pre-2026-07 builds were `com.shaikzeeshan.mnema`; a one-time config-dir rename in `lib.rs` migrates old installs). Changing it changes the app identity for Tauri/macOS config and cache dirs, and resets TCC permissions. Keychain service names deliberately keep the old `com.shaikzeeshan.mnema.*` strings — they are storage keys, not bundle-id-derived.

---

## Settings Persistence

- Recording settings → `recording-settings.json` under `app_config_dir()`.
- Keyboard shortcuts → `keyboard-bindings.json` under `app_config_dir()`; registration is Rust-owned in `keyboard_bindings.rs`, not Svelte.
- App infra state (SQLite) → `<saveDirectory>/db/app.sqlite3`; changing `saveDirectory` changes the DB location on next start.
- One-time dismissible prompts → shared One-Time Prompt State under `app_config_dir()`, NOT `recording-settings.json` or localStorage. Prompt IDs must be stable/versioned and track shown/dismissed/completed timestamps.

---

## Native Capture Output Structure

- Date-organized: `<saveDirectory>/recordings/YYYY/MM/DD/`
- Screen segments: `<session>-segment-####.mov`
- Audio: `<saveDirectory>/recordings/YYYY/MM/DD/audio/<source-session>-segment-####.m4a`. Microphone and system audio each get their own source-session id (`mic_…`, `sysaudio_…`), which is what keeps them from colliding in the one flat `audio/` directory. A mid-segment restart (mic reconnect, tap rebuild, inactivity resume) appends `-<unix-ms>` to the same segment index rather than opening a new one.
- Hidden per-segment workspace dirs under the same date folder hold temporary screen artifacts and exported JPEG frames.

---

## Hidden Segment Workspace Cleanup

- Cleanup safety depends on the sibling `<session>-segment-####.mov`. If missing **or unopenable** (e.g. missing `moov`), preserve non-empty hidden workspaces. Empty/no-reference hidden workspaces are safe to remove.
- Repair headlessly: `cargo run -p app-infra --bin repair_hidden_segment_workspaces -- <saveDirectory>`
- Tauri app runs repair at startup + every 5 minutes in the background.

---

## Frame Preview

- Preferred source: per-segment binary sidecar `<session>-segment-####.frame-index.bin` written by `crates/capture-screen` at finalization. Falls back to legacy JSON sidecar, then first-frame timestamp estimate.
- Sidecar `video_offset_ms` is recorded **live at export time** as `sample_pts − first_appended_sample_pts`. Never derive it by pairing index entries with video samples positionally: the `.mov` receives every appended frame while JPEG exports are throttled, so entry *k* is generally NOT video sample *k* (the old positional pairing drifted up to ~23s per 60s segment). Sidecars written before 2026-07-03 carry those wrong offsets and are deliberately not repaired.
- `get_frame_preview` returns **asset-backed file paths**, not base64. Stable frames via Tauri asset scope; fallback/video previews materialized under `app_cache_dir()/frame-previews/`.
- Screen finalization rejects existing-but-unopenable `.mov` files — invalid segments must fail finalization, not be committed.
- Convert legacy JSON sidecars: `cargo run -p app-infra --bin convert_frame_index_sidecars -- <saveDirectory>/recordings`

---

## Scrub Preview

- A separate low-cost navigation tier — NOT a source of Captured Frame truth.
- OCR/copy/download/parked inspection must go through `get_frame_preview`.
- Artifacts live under `app_cache_dir()`, keyed by segment/video-offset/rendition/source freshness — not under `<saveDirectory>/recordings`.
- Cache files are disposable: cleaned opportunistically by age + count on startup and on-demand writes.

---

## Capture Segments and Retention

- **Segment Duration cap: 5 minutes.** This caps each rotated Capture Segment, not total session length.
- Schema changes → `crates/app-infra/migrations`; embedded via `sqlx::migrate!`.
- Retention models in `capture_sessions` + `capture_segments`; uses local-calendar cutoffs, preserves `person_profiles`, prunes via `timeline_data_changed`.

---

## OCR / Processing Pipeline

- OCR dedupe is two-stage: `crates/capture-screen` computes frame equivalence data; `crates/app-infra` skips the job when an earlier equivalent frame is already eligible as the OCR fallback.
- `crates/app-infra/src/captured_frame_equivalence.rs` is the seam for Captured Frame Equivalence policy — always use `CapturedFrameEquivalenceResolver`, never rebuild quarantine/version/proof-match logic inline.
  - OCR/job fallback → nearest earlier equivalent frame.
  - Dashboard `duplicateOf` UI → earliest frame in the chain (so adjacent duplicates point back to the origin).
- Equivalence scope: session-wide by default; narrows to the same hidden segment workspace when the candidate path originates from one.
- `FrameBatchStore`: keep finalize-job scheduling inside the same transaction as frame persistence, OCR job enqueue, and batch attachment.

---

## Processing Job Retry / Reclamation

- Failed jobs retry with wall-clock backoff via `processing_jobs.next_attempt_at`, bounded by `failure_count` (genuine failures only).
  - OCR: `OCR_RETRY_BACKOFF_SECONDS` [30–120s], capped at `OCR_FAILED_JOB_MAX_ATTEMPTS`.
  - Audio: `AUDIO_RETRY_BACKOFF_SECONDS` [60–300s], capped at `AUDIO_FAILED_JOB_MAX_ATTEMPTS`.
- Auto-claim (`claim_next_queued_job_*`, ordered `id ASC`) skips jobs with a future `next_attempt_at`. Explicit `claim_queued_job(id)` (reprocess) bypasses backoff.
- `requeue_processing_job_in_transaction` resets `next_attempt_at` to NULL (immediately eligible); backoff is failure-retry-only.
- **Orphaned job reclamation** (`reconcile_orphaned_running_jobs`): requeues `running` rows left from app quit/crash back to `queued`. Abandonment does NOT spend a failure attempt. Only runs when nothing is executing: at startup (inside deferred startup, before workers spawn) and at graceful shutdown (after workers are aborted). Crash-loop backstop: `RECLAIM_ATTEMPT_CEILING` (10). See [ADR 0020](../adr/0020-reclaim-orphaned-processing-jobs-by-requeue.md).

---

## Deferred Startup

App startup is split so the window opens fast:

- **Fast path** (`AppInfra::initialize_fast_with_processing_registry`): opens the encrypted SQLite pool + builds stores. Tauri commands can serve queries immediately.
- **Deferred path** (`run_deferred_startup_blocking`, in `mnema-deferred-startup` background thread after `open_startup_window`): runs maintenance, hidden-segment repair, sidecar conversion, OCR-disabled reconciliation, audio/speaker backfill, then spawns workers.
- **Order is load-bearing**: maintenance + repair must complete before any worker or capture auto-start. Do not move these back into the synchronous path.
- Use `AppInfra::initialize` (fast + maintenance combined) for tests and the CLI sidecar only.

---

## Recording Lifecycle

- **Single owning seam**: `apps/desktop/src-tauri/src/native_capture/lifecycle.rs`. Tauri handlers are thin adapters.
- macOS-oriented; many capture paths and tests are behind `cfg(target_os = "macos")`.
- ScreenCaptureKit invalidation has two paths: `NSWorkspaceWillSleepNotification` (proactive) and `SCStreamDelegate` (`stream:didStopWithError:`/`streamDidBecomeInactive:`). Will-sleep stops the writer cleanly and drops **only live screen state** — microphone and system audio keep recording (`clear_screen_state_for_sleep_or_stop`). A delegate-reported stop (`capture_stream_system_stopped`, e.g. `-3815` on display sleep) is terminal (`stream_terminated`): the stop path skips the doomed second `stop_stream` but still finalizes the writers, and the reconcile enters the `DisplayUnavailable` suspension — never a bare state clear, which used to let the next segment rotation fail the whole session. `tick_rotation` has a suspend-instead-of-fail backstop for a missing screen session. See ADR 0021's amendments.
- On sleep: preserve stale screen paths in `current_segment_output_files` even after clearing live handles — wake recovery reads them to derive interrupted segment outputs. Do not clear the system-audio path there: the tap is still writing that file, and clearing it orphans the live segment.
- When the segment loop ends a session internally (`mark_runtime_session_failed`), it must broadcast `emit_native_capture_session_changed` + `status_bar::refresh` after exiting — user/command stops own this broadcast, but an internal failure has no other announcer.

---

## System Audio (Core Audio process tap)

Design authority: [ADR 0052](../adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md). Crate: `crates/capture-system-audio`. Desktop adapter: `apps/desktop/src-tauri/src/native_capture/system_audio.rs` (the sibling of `microphone.rs` — nothing in it knows about the screen, and that is the point).

- **`capture-screen` captures no audio.** `set_captures_audio(false)`; the SCK audio output, its callback arm, and the system-audio activity atomics are gone (the atomics live in `capture_system_audio::activity` now). Do not reintroduce an SCK audio path or a fallback — the ADR rejected a dual-path mode outright.
- **A tap generation is disposable.** `SystemAudioTapSession::start` builds tap + aggregate + IOProc; dropping it tears the generation down. Teardown order is Stop → DestroyIOProc → DestroyAggregate → DestroyTap. Aggregates a crashed run left behind would collide with the UIDs a new tap mints and fail its start outright, so they are reaped by UID prefix (`mnema-system-audio-<pid>-<instance>`) in **deferred startup** — off the first-paint path, since a stale aggregate can only matter once a recording starts.
- **Rebuild is the only recovery** (`rebuild.rs`), never a patch of a live tap: `RebuildReason` is `default_output_device_changed`, `device_died`, `exclude_list_moved`, or `zero_watchdog`. All four only *request* a rebuild; it happens on the caller's tick in `SystemAudioCaptureSession::poll`, because tearing a tap down blocks on the in-flight IOProc and doing it from a listener block would deadlock.
- **The zero-watchdog assumes guilt after silence** (`watchdog.rs`): 30 s without a non-zero sample trips a rebuild, doubling to a 600 s cap, reset by any sound. One rule ("how long since the last sound?") covers both zeros-arriving and deliveries-stopping. It **must run while system audio is paused for inactivity** — resume means "sound detected", so a wedged tap can never wake itself. `tick_system_audio` is therefore unconditional on pause state.
- **The tap follows the device's format**, it does not pin one: the ASBD is device-dependent (44.1 ↔ 48 kHz) and changes across rebuilds. The writer is created lazily once the observed format is stable (mic's pattern). No resampler — transcription/VAD/diarization decode and resample themselves.
- **Every rebuild is a segment boundary**, not a splice: the in-flight segment finalizes and a fresh file opens via `system_audio_resume_file` (the same collision-safe naming an inactivity resume uses). Never continue a writer across generations — it cannot survive a format change.
- **Permission is inferred, never known.** Process taps have no authorization API, so `permission.rs` derives the tri-state from one persisted `SystemAudioEvidence` (`none` / `silent_session` / `sound_heard`, an `app_settings` row — the string values are stable, renaming one resets every install's evidence). A delivered sound proves a grant; silence proves nothing, so silence only counts after `SILENT_SESSION_AFTER_MS` (60 s) of tap and only ever raises a *dismissible* "may be blocked" hint. Unsupported OS always wins over evidence.
- **Every tap line is prefixed `system-audio-tap:`** (`capture_system_audio::LOG_PREFIX`) — grep that and nothing else. Nearly all of them (including every rebuild) are **Debug-level**, so they reach `rust.log` only in a debug build or with developer-options debug logging on; the stale-aggregate reap at startup is the exception, logged at Info. Manual drills: [`system-audio-drills.md`](system-audio-drills.md).

---

## Inactivity / Audio

- Inactivity decisions run on the segment loop's 1s poll interval; microphone/system-audio producers must preserve a **peak-since-last-poll** signal — a single latest raw sample can miss brief bursts.
- Per-family inactivity: treat unrequested sources as ineligible. Do not let microphone/system-audio policies fall back to `internal_fallback` for unrequested sources.
- Inactivity activity mode is fixed to `system_input_or_screen_or_audio`. Normalize legacy `activityMode`/`inactivityActivityMode` values on save.
- Audio speech detection: shared under `audioSpeechDetection.detector` (`silero`, `webrtc`, `off`). Selection is exact — do not silently fall back from Silero to WebRTC.
- Dashboard inactivity debug: `idleDebug.activitySources` = threshold-qualified idle (for decisions); `microphoneActivityLastUnixMs`/`systemAudioActivityLastUnixMs` = raw sample timestamps (label as samples, not activity).
- System-audio transcription is gated by `system_audio_speech_activity` job. Segment commit enqueues that job — not direct transcription.

---

## Privacy / Display Unavailable

- Live privacy refresh coalesces through `apps/desktop/src-tauri/src/native_capture/privacy.rs`: settings/workspace events request a generation, wake `SegmentLoopControl`, and the segment loop applies completed filters.
- **Display-unavailable is a transient liveness condition, NOT a privacy failure.** Code: `privacy_filter_display_unavailable`. Suspend with `CaptureSuspensionKind::DisplayUnavailable` (not `PrivacyFilter`). Never escalate; keep the session alive; gate restart on `screen_display_available()` + `DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL`. See [ADR 0021](../adr/0021-recover-from-display-unavailable-as-transient-liveness.md).
- **Suspension scope is screen-only, except `LowDisk`.** `suspend_screen_capture` keeps its name but touches the screen alone: microphone and system audio record through `PrivacyFilter` and `DisplayUnavailable`. `LowDisk` uniquely stops all three (shared volume, [ADR 0040](../adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md)) and is the only path that commits the mic's finalized tail (`commit_suspended_microphone_outputs`) — the other kinds never stop the mic, so they have nothing to commit.
- `PrivacyFilter` suspension is capped (`MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS`) then escalates to a manual-restart notification. `DisplayUnavailable` never escalates.
- **Privacy exclusion is two mechanisms, one list.** Windows are excluded by the ScreenCaptureKit filter; **audio** is excluded by the process tap's exclude list (own process + privacy-listed apps), which the segment loop forwards from the same collected privacy update and which rebuilds the tap only when the resolved process-object list actually moves. That audio exclusion is **parity** with the deleted SCK content filter, not a feature — losing it silently records privacy-excluded apps ([ADR 0052](../adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md)).
- Sensitive Capture Protection V1: app-only privacy model (ADR 0006, ADR 0008). Do not add URL/title/private-window/password-field filtering to live privacy.
- ADR 0013: browser-integrated credential-entry suspension and add-on metadata are explicitly rejected. Browser Metadata Collection is native URL probing only.
- Recommended App Exclusions: Rust-owned exact bundle-id catalog, finite curated categories. Keep Known Browser App values separate. Do not default-recommend System Settings, Terminal, messaging, or email.
- Status-bar source toggles update persisted `RecordingSettings` only while stopped. Tray + frontend sync via `native_capture_session_changed` (start/stop) and `recording_settings_changed` (settings).

---

## Browser URL Metadata

Native URL probing only (ADR 0013 — no extensions/add-ons). Strategy lives in `crates/capture-metadata/src/lib.rs` (`KNOWN_BROWSER_APPS`, `BrowserUrlDialect`, `browser_url_applescript`, `BrowserUrlProbeCache`); execution in `apps/desktop/src-tauri/src/native_capture_metadata.rs` (`active_browser_url`, `browser_url_probe_for_active_bundle`).

- **Dialect matters — Safari is NOT `front document`.** Chromium browsers read `URL of active tab of front window`; WebKit (Safari/Orion) reads `URL of current tab of front window`. `front document` is ordered by *focus recency*, not window z-order, so with multiple Safari windows it can return a background window's URL instead of the visually-frontmost tab.
- **The URL cache is title-gated, not time-gated.** The front-window title is captured fresh every tick; `BrowserUrlProbeCache::cached_url_for` forces a re-probe whenever the title changes for the same browser. This is what prevents the desync where a previous tab's URL is served under a new page's title (e.g. an old GitHub URL stamped on a frame whose title already reads "…Start Page"). `BROWSER_URL_PROBE_BACKSTOP_INTERVAL` (5s) is only a backstop for navigations that change the URL without changing the title (some single-page apps).
- The probe runs **only when the frontmost app is a known browser bundle** (`is_known_browser_bundle`); the cache exists to throttle the per-probe `osascript` subprocess, not to poll.
- **Gecko browsers read via the Accessibility API, not AppleScript.** Firefox (`org.mozilla.firefox`) and Zen (`app.zen-browser.zen`) expose no scriptable URL surface (`url_script_app_name` is `None`), so their descriptor sets `url_strategy: Some(BrowserUrlStrategy::Accessibility)`. The read lives in `apps/desktop/src-tauri/src/native_capture_browser_url_ax.rs`: it climbs the focused element's parent chain and returns the `AXURL` of the outermost `AXWebArea` (the active tab). It is opt-in and **permission-gated** — it yields no URL until the macOS Accessibility permission is granted (the reader gates on holding that permission). AppleScript dialects (`BrowserUrlStrategy::AppleScript(BrowserUrlDialect::…)`) still cover Chromium and WebKit.
- **App identity must not wait on the URL read (focus-change pre-pass).** A metadata refresh publishes the *whole* snapshot — app name **and** URL — in one write *after* the live URL read returns. Because the Gecko AX read can cost ~1.4s, a naïve single-pass publish leaves frames exported right after switching **into** a browser stamped with the **previous** app's name (the symptom: a Zen frame labelled "Hitch"). Fix in `native_capture/privacy.rs`: on `PrivacyRefreshReason::WorkspaceFocusChanged`, the collection thread first runs `collect_privacy_filter_update(.., BrowserUrlReadMode::Cached)` to publish the fresh frontmost app identity (with the cached URL) within ~ms, then the `Live` pass upgrades only the URL for the same app. The privacy *decision* is bundle-only (URL-independent), so the pre-pass changes no filtering; it is gated to focus changes so the 1 Hz fallback poll doesn't re-enumerate windows.
- **Frames are stamped by capture time, not JPEG-write-completion time.** `on_frame_exported` (`native_capture/segments.rs`) fires only *after* the JPEG finishes writing (~100–300ms after capture, more under load), so stamping it with the *current* `latest_snapshot` mislabels the last frame captured just before an app switch with the app switched **into** (pixels show app A, chip says B — the opposite direction from the pre-pass bug above). Fix in `native_capture_metadata.rs`: `CaptureMetadataRuntime` keeps a small ring buffer of `(published_at_unix_ms, snapshot)`; the provider selects `snapshot_in_effect_at(artifact.captured_at_unix_ms)` — the most recent snapshot published at or before the frame's capture instant. Frames captured before the first snapshot publish (session start) still stamp `None`. History clears with `reset_recording_session_privacy_state`. This fixes both mislabel directions.

---

## Deletion Semantics

- **Delete Recent Capture** ≠ Retention Cleanup. It is a confirmed recovery action that deletes whole overlapping Capture Segments/Audio Segments and derived app data. No secure erase promise.
- **User Capture Pause** ≠ inactivity pause: keeps the Capture Session alive, finalizes the active segment, records nothing while paused, resumes only from explicit user action.
- **Retention Policy does NOT cascade to `user_context_*` tables** — enforced by a structural test in `capture_retention.rs`.
- **Delete Recent Capture DOES cascade** to User Context via `UserContextStore::delete_derived_for_capture_subjects`.

---

## Speaker Analysis Models

On-device diarization runs through the sole **speakrs** provider (pure-Rust pyannote-community-1 + WeSpeaker on CoreML, model id `pyannote-community-1-wespeaker`). Its preset ships **raw model files** (no `.tar.bz2`): each descriptor `relative_path` lands under the model store, `sherpa_params` is absent, and the helper reads them via `from_dir` / `SPEAKRS_MODELS_DIR`. Set the descriptor's `required_files` to the destination layout so install verification matches.
