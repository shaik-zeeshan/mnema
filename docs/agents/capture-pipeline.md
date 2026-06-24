# Capture Pipeline — Agent Reference

Deep-dive quirks for the capture/storage/OCR/privacy system. Referenced from `CLAUDE.md`.

---

## Frontend / App Layout

- SPA-only: `apps/desktop/src/routes/+layout.ts` sets `ssr = false`; `svelte.config.js` uses `@sveltejs/adapter-static` with `fallback: "index.html"`.
- In production packaging the SPA entry can arrive as `"/index.html"` — normalize static-entry paths in the shell before route-gating.
- Tauri expects Vite on port `1420` with HMR on `1421`; `vite.config.js` hard-pins these and ignores `src-tauri/**`.
- Bundle identifier is `com.shaikzeeshan.mnema`; changing it changes the app identity for Tauri/macOS config and cache dirs.

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
- Audio: `<saveDirectory>/recordings/YYYY/MM/DD/audio/<session>/segment-####/`
- Hidden per-segment workspace dirs under the same date folder hold temporary screen artifacts and exported JPEG frames.

---

## Hidden Segment Workspace Cleanup

- Cleanup safety depends on the sibling `<session>-segment-####.mov`. If missing **or unopenable** (e.g. missing `moov`), preserve non-empty hidden workspaces. Empty/no-reference hidden workspaces are safe to remove.
- Repair headlessly: `cargo run -p app-infra --bin repair_hidden_segment_workspaces -- <saveDirectory>`
- Tauri app runs repair at startup + every 5 minutes in the background.

---

## Frame Preview

- Preferred source: per-segment binary sidecar `<session>-segment-####.frame-index.bin` written by `crates/capture-screen` at finalization. Falls back to legacy JSON sidecar, then first-frame timestamp estimate.
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
- ScreenCaptureKit invalidation has two paths: `NSWorkspaceWillSleepNotification` (proactive) and `SCStreamDelegate` (`stream:didStopWithError:`/`streamDidBecomeInactive:`). The Recording Lifecycle reconciles either by dropping only live screen/system-audio state while preserving microphone.
- On sleep: preserve stale screen/system-audio paths in `current_segment_output_files` even after clearing live handles — wake recovery reads them to derive interrupted segment outputs.
- When the segment loop ends a session internally (`mark_runtime_session_failed`), it must broadcast `emit_native_capture_session_changed` + `status_bar::refresh` after exiting — user/command stops own this broadcast, but an internal failure has no other announcer.

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
- `PrivacyFilter` suspension is capped (`MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS`) then escalates to a manual-restart notification. `DisplayUnavailable` never escalates.
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

---

## Deletion Semantics

- **Delete Recent Capture** ≠ Retention Cleanup. It is a confirmed recovery action that deletes whole overlapping Capture Segments/Audio Segments and derived app data. No secure erase promise.
- **User Capture Pause** ≠ inactivity pause: keeps the Capture Session alive, finalizes the active segment, records nothing while paused, resumes only from explicit user action.
- **Retention Policy does NOT cascade to `user_context_*` tables** — enforced by a structural test in `capture_retention.rs`.
- **Delete Recent Capture DOES cascade** to User Context via `UserContextStore::delete_derived_for_capture_subjects`.

---

## Speaker Analysis Models

On-device diarization runs through the sole **speakrs** provider (pure-Rust pyannote-community-1 + WeSpeaker on CoreML, model id `pyannote-community-1-wespeaker`). Its preset ships **raw model files** (no `.tar.bz2`): each descriptor `relative_path` lands under the model store, `sherpa_params` is absent, and the helper reads them via `from_dir` / `SPEAKRS_MODELS_DIR`. Set the descriptor's `required_files` to the destination layout so install verification matches.
