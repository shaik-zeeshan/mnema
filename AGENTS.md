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
- `crates/app-infra` owns SQLite, embedded sqlx migrations, background jobs, frame batches, and the OCR/processing pipeline.
- `crates/capture-types` owns serde types shared across frontend, Tauri, and native layers.
- `crates/capture-screen`, `crates/capture-microphone`, and `crates/capture-writers` own capture primitives and media output.
- `crates/capture-runtime` holds generic runtime helpers.

## Quirks
- The desktop frontend is SPA-only: `apps/desktop/src/routes/+layout.ts` sets `ssr = false`, and `apps/desktop/svelte.config.js` uses `@sveltejs/adapter-static` with `fallback: "index.html"`.
- Tauri expects Vite on port `1420` with HMR on `1421`; `apps/desktop/vite.config.js` hard-pins those ports and ignores `src-tauri/**` in the watcher.
- `apps/desktop/src-tauri/tauri.conf.json` runs `beforeDevCommand: bun run dev` and `beforeBuildCommand: cargo clean --manifest-path src-tauri/Cargo.toml && bun run build`; `tauri build` always cleans the Rust crate first.
- Recording settings persist to `recording-settings.json` under Tauri `app_config_dir()` when available. App infra state lives under `<saveDirectory>/.z`, with SQLite at `<saveDirectory>/.z/db/app.sqlite3`; changing `saveDirectory` changes that DB location on the next app start.
- Native capture output is date-organized under `<saveDirectory>/.z/recordings/YYYY/MM/DD/`. Screen recordings are saved as `<session>-segment-####.mov`, while audio stays separate under `<saveDirectory>/.z/recordings/YYYY/MM/DD/audio/<session>/segment-####/`. Hidden per-segment workspace directories under the same date folder are used for temporary capture artifacts and frame exports.
- App infra schema changes belong in `crates/app-infra/migrations`; migrations are embedded via `sqlx::migrate!`.
- Native capture is macOS-oriented; many capture code paths and tests are behind `cfg(target_os = "macos")`.
- The dashboard inactivity debug surface has two different audio notions: `idleDebug.activitySources` carries threshold-qualified microphone/system-audio idle used for inactivity decisions, while `microphoneActivityLastUnixMs` / `systemAudioActivityLastUnixMs` are raw sample timestamps and should be labeled as samples rather than activity.

## Verification
- UI-only changes: `bun run check`.
- Rust changes in one crate: start with `cargo check -p <crate>` or `cargo test -p <crate>`.
- Tauri wiring or cross-crate Rust changes: run `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- Cross-stack or settings/storage changes: run both `bun run check` and `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- Focused tests in `apps/desktop/src-tauri/src/lib.rs` may need `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib <test-filter>`; without `--lib`, filtered Tauri tests may not run as expected.

## Workflow
- When new repo-specific behavior, commands, structure, or gotchas are discovered during a change, ask the user whether that context should also be added to `AGENTS.md`.
