# AGENTS

## Workspace
- This repo has two roots of truth: the Bun/Turbo workspace in `package.json` and the Rust Cargo workspace in `Cargo.toml`.
- `apps/desktop` is the only JS app. Shared native/backend code lives in `crates/*`, not `packages/*`.

## Commands
- Run repo commands from the repo root.
- Frontend flow: `bun run dev`, `bun run check`, `bun run build`.
- Tauri CLI from the root: `bun run tauri -- dev` or `bun run tauri -- build`.
- Rust-only verification for the desktop app: `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- Focused Rust work: `cargo check -p <crate>` and `cargo test -p <crate>`.

## Boundaries
- Svelte UI lives in `apps/desktop/src`. The shell is `src/routes/+layout.svelte`, the dashboard is `src/routes/+page.svelte`, and settings live in `src/routes/settings/+page.svelte`.
- Tauri startup and command registration live in `apps/desktop/src-tauri/src/lib.rs`; keep new `#[tauri::command]` functions wired there and keep frontend `invoke(...)` names in sync.
- The owning seam for native capture is `apps/desktop/src-tauri/src/native_capture/lifecycle.rs` (**Recording Lifecycle**); Tauri command handlers and background hooks should stay thin adapters over that module rather than orchestrating `NativeCaptureRuntime` directly.
- `crates/app-infra` owns SQLite, embedded sqlx migrations, background jobs, frame batches, and the OCR/processing pipeline.
- `crates/capture-types` owns serde types shared across frontend, Tauri, and native layers.
- `crates/capture-screen`, `crates/capture-microphone`, and `crates/capture-writers` own capture primitives and media output.
- `crates/capture-runtime` holds generic runtime helpers.

## Quirks
- The desktop frontend is SPA-only: `apps/desktop/src/routes/+layout.ts` sets `ssr = false`, and `apps/desktop/svelte.config.js` uses `@sveltejs/adapter-static` with `fallback: "index.html"`.
- Tauri expects Vite on port `1420` with HMR on `1421`; `apps/desktop/vite.config.js` hard-pins those ports and ignores `src-tauri/**` in the watcher.
- `apps/desktop/src-tauri/tauri.conf.json` runs `beforeDevCommand: bun run dev` and `beforeBuildCommand: cargo clean --manifest-path src-tauri/Cargo.toml && bun run build`; `tauri build` always cleans the Rust crate first.
- Recording settings persist to `recording-settings.json` under Tauri `app_config_dir()` when available. App infra state lives under `<saveDirectory>/.z`, with SQLite at `<saveDirectory>/.z/db/app.sqlite3`; changing `saveDirectory` changes that DB location on the next app start.
- Native capture output is date-organized under `<saveDirectory>/.z/recordings/YYYY/MM/DD/`. Screen recordings are saved as `<session>-segment-####.mov`, while audio stays separate under `<saveDirectory>/.z/recordings/YYYY/MM/DD/audio/<session>/segment-####/`. Hidden per-segment workspace directories under the same date folder are used for temporary capture artifacts and JPEG frame exports.
- Hidden segment workspace cleanup safety depends on the sibling visible segment `<session>-segment-####.mov`: if that file is missing, app-infra preserves non-empty hidden workspaces for fallback/debugging, but empty/no-reference hidden workspaces are safe to remove. Existing stale safe workspaces can be repaired headlessly with `cargo run -p app-infra --bin repair_hidden_segment_workspaces -- <saveDirectory>/.z`, and the desktop Tauri app also runs this repair once at startup plus every 5 minutes in the background.
- Hidden segment workspace repair now treats an existing-but-unopenable sibling segment `.mov` (for example, a file missing `moov`) the same as a missing visible segment for cleanup safety, so broken visible segments preserve hidden frame fallbacks instead of deleting the only preview source.
- Screen frame preview lookup now prefers a sibling per-segment binary sidecar `<session>-segment-####.frame-index.bin` written by `crates/capture-screen` during screen output finalization. The sidecar stores only captured-at/frame-index identity plus finalized-video-relative offsets derived from the finished asset's sample timing, and `apps/desktop/src-tauri/src/app_infra.rs` falls back to the older first-frame timestamp estimate only when neither the binary sidecar nor a legacy JSON sidecar is present.
- Existing legacy JSON sidecars can be converted headlessly with `cargo run -p app-infra --bin convert_frame_index_sidecars -- <saveDirectory>/.z/recordings`.
- App infra schema changes belong in `crates/app-infra/migrations`; migrations are embedded via `sqlx::migrate!`.
- `FrameBatchStore` has transaction-scoped finalize helpers for batched frame insertion; keep finalize-job scheduling inside the same transaction as frame persistence, OCR job planning/enqueue, and batch attachment when changing `insert_frame_into_batch_and_maybe_enqueue_ocr_job` or related paths.
- OCR dedupe is two-stage: `crates/capture-screen` produces a lightweight `content_fingerprint` as a prefilter, and `crates/app-infra` skips a new OCR job when an earlier frame in the same session already has the same fingerprint.
- `crates/app-infra/src/captured_frame_equivalence.rs` is the explicit seam for **Captured Frame Equivalence** policy: callers resolve the nearest earlier equivalent **Captured Frame** through `CapturedFrameEquivalenceResolver` instead of rebuilding quarantine/version/proof-match logic.
- The equivalence seam has two caller semantics: OCR/job fallback uses the nearest earlier equivalent **Captured Frame**, while the dashboard `duplicateOf` UI uses the earliest earlier equivalent frame in the duplicate chain so adjacent duplicates still point back to the first origin frame.
- **Captured Frame Equivalence Scope** is explicit at that seam: lookup is session-wide by default, but narrows to the same hidden segment workspace when the candidate frame path originates from a hidden segment workspace.
- Native capture is macOS-oriented; many capture code paths and tests are behind `cfg(target_os = "macos")`.
- ScreenCaptureKit screen/system-audio liveness has two invalidation paths: AppKit `NSWorkspaceWillSleepNotification` clears the live screen side proactively, and the `crates/capture-screen` `SCStreamDelegate` also marks the stream dead on `stream:didStopWithError:` / `streamDidBecomeInactive:`; the `Recording Lifecycle` reconciles either signal by dropping only live screen/system-audio state while preserving microphone continuation.
- On system sleep, the `Recording Lifecycle` must preserve stale screen/system-audio paths inside `current_segment_output_files` even after clearing live `recording_file` / `system_audio_recording_file`, because wake recovery derives the interrupted segment outputs from that bookkeeping before starting the next segment.
- `apps/desktop/src-tauri/src/native_capture_output.rs` screen finalization now rejects existing-but-unopenable screen `.mov` files instead of treating mere file existence as success; invalid rotated segments should fail finalization rather than being committed and later breaking frame preview extraction.
- The dashboard inactivity debug surface has two different audio notions: `idleDebug.activitySources` carries threshold-qualified microphone/system-audio idle used for inactivity decisions, while `microphoneActivityLastUnixMs` / `systemAudioActivityLastUnixMs` are raw sample timestamps and should be labeled as samples rather than activity.
- Audio inactivity decisions run on the segment loop's coarse poll interval (currently 1s), so microphone/system-audio producers must preserve a peak-since-last-poll signal for inactivity evaluation; a single latest raw sample can miss brief real audio bursts.

## Verification
- UI-only changes: `bun run check`.
- Rust changes in one crate: start with `cargo check -p <crate>` or `cargo test -p <crate>`.
- Tauri wiring or cross-crate Rust changes: run `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- Cross-stack or settings/storage changes: run both `bun run check` and `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- Focused tests in `apps/desktop/src-tauri/src/lib.rs` may need `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib <test-filter>`; without `--lib`, filtered Tauri tests may not run as expected.

## Workflow
- When new repo-specific behavior, commands, structure, or gotchas are discovered during a change, ask the user whether that context should also be added to `AGENTS.md`.
