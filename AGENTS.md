# AGENTS

## Workspace
- Two roots of truth: Bun/Turbo workspace (`package.json`) and Rust Cargo workspace (`Cargo.toml`).
- `apps/desktop` is the only JS app. Shared native/backend code lives in `crates/*`.
- Platform support status lives in `SUPPORTS.md`; update it when adding or changing macOS, Windows, or Linux behavior.

## Commands
- Run all repo commands from the repo root.
- Frontend: `bun run dev`, `bun run check`, `bun run build`.
- Tauri CLI: `bun run tauri -- dev` or `bun run tauri -- build`.
- Desktop Rust check: `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` (needs `mnema-cli` sidecar — see Verification).
- Focused Rust: `cargo check -p <crate>` and `cargo test -p <crate>`.

## Boundaries
- **Svelte UI**: `apps/desktop/src/`. Shell: `+layout.svelte`, dashboard: `+page.svelte`, settings: `settings/+page.svelte`.
- **Tauri commands**: wired in `apps/desktop/src-tauri/src/lib.rs`; keep `#[tauri::command]` registration and frontend `invoke(...)` names in sync.
- **Status-bar/tray**: Rust-owned in `status_bar.rs`; keep it as a native `TrayIconBuilder`/`Menu`/`CheckMenuItem` surface with `tray_*` prefix IDs.
- **Recording Lifecycle**: `apps/desktop/src-tauri/src/native_capture/lifecycle.rs` is the owning seam; Tauri handlers and hooks are thin adapters over it.
- **`crates/app-infra`**: SQLite, embedded sqlx migrations, background jobs, frame batches, OCR/processing pipeline.
- **`crates/capture-types`**: serde types shared across frontend, Tauri, and native layers.
- **`crates/capture-screen/microphone/writers`**: capture primitives and media output.
- **`crates/ai-runtime`**: the Reasoning Engine — provider-agnostic `rig-core` wrapper (cloud Anthropic/OpenAI, local Ollama/Llamafile). Imported as `ai_engine` alias in `apps/desktop/src-tauri/Cargo.toml` to avoid name collision. See [ADR 0028](docs/adr/0028-reasoning-engine.md).
- **Keychain**: provider API keys stored ONLY in OS keychain via `crates/app-infra/src/ai_provider_key_store.rs` — never in config files.

## AI Features

- **Reasoning Engine** (`crates/ai-runtime`): `EngineConfig`, `extract::<T>()`, `run_agent_loop()`. Settings shape and provider-centric model selection: [ADR 0034](docs/adr/0034-ai-settings-are-provider-centric-with-one-global-default-model.md), amended by [ADR 0035](docs/adr/0035-provider-identity-is-a-per-instance-id-not-the-kind.md) (provider identity is a stable per-instance `id`, so same-kind providers can coexist; the keychain account, model `provider` tag, and pin all key off the instance id). Single resolver: `crate::ai_runtime::resolve_engine_config(settings, pin, feature_override_model)`.
- **Ask AI** (Quick Recall + Chat): in-process via `crates/ai-runtime`, implemented in `apps/desktop/src-tauri/src/ask_ai.rs`. Stateless-per-turn over the persistent conversation store. See [ADR 0033](docs/adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md).
- **Ask AI streaming = backend-owned, frontend renders** (invariant): `ask_ai.rs` holds the one render-ready `TurnView` per turn, parses the answer into typed `AnswerBlock`s (`mnema-bars`/`mnema-dossier`/`mnema-timeline`) in `ask_ai/answer_view.rs`, formats tool labels + icon paths server-side, and streams structured ops over the versioned `ask_ai_update` event with an `ask_ai_snapshot` reattach command. Both doors (`Chat.svelte`, `routes/quick-recall/+page.svelte`) only render — no fence parsing, tool-label formatting, or phase machine frontend-side (Markdown→HTML is the one render step that stays frontend, in `AnswerProse`). Wire shapes are hand-mirrored `capture-types/src/conversation.rs` ↔ `lib/insights/conversation.ts` (no codegen — keep the serde round-trip test + `bun run check` green). Parsed blocks persist (migration `0036`); cold reattach + legacy `blocks=NULL` parse-on-read live in the desktop `get_conversation` command. Deep context: [`docs/agents/ask-ai-streaming.md`](docs/agents/ask-ai-streaming.md).
- **User Context**: storage + policy in `crates/app-infra/src/user_context/`; LLM orchestration in `apps/desktop/src-tauri/src/user_context/`. Deep context: `docs/user-context/CONTEXT.md`. Retention never cascades to `user_context_*` tables; Delete Recent Capture does. See [ADR 0029](docs/adr/0029-user-context-outlives-raw-retention-privacy-delete-cascades.md).
- **AI Temporal Grounding**: Ask AI leads each turn with local time + UTC offset from the frontend (`askAiClock.ts`). User Context uses labeled UTC. Digest uses local offset recovered from `range_start_ms`.

## Capture Pipeline
Detailed quirks for capture/storage/OCR/privacy behavior: [`docs/agents/capture-pipeline.md`](docs/agents/capture-pipeline.md).

Key invariants:
- Capture Segment Duration is capped at 5 minutes.
- App infra schema changes belong in `crates/app-infra/migrations` (embedded via `sqlx::migrate!`).
- Deferred startup: window opens fast; maintenance + workers run in `mnema-deferred-startup` thread after the window opens. Do not move maintenance or worker-spawn back into the synchronous path.
- Display-unavailable (sleep/lock/disconnect) is a transient liveness condition, NOT a privacy failure — keep the session alive, skip finalize, wait for display return. See [ADR 0021](docs/adr/0021-recover-from-display-unavailable-as-transient-liveness.md).
- Delete Recent Capture ≠ Retention Cleanup: deletes whole overlapping segments; Retention never touches `user_context_*` tables.

## Verification
- UI-only: `bun run check`.
- One Rust crate: `cargo check -p <crate>` or `cargo test -p <crate>`.
- Tauri/cross-crate Rust: `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- Cross-stack: both of the above + `bun run check`.
- Tauri lib tests: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib <filter>`.
- Status-bar tests: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib status_bar`.
- AI runtime: `cargo check -p ai-runtime`. Keychain store: `cargo test -p app-infra ai_provider_key_store`.
- **`mnema-cli` sidecar required** for any `apps/desktop/src-tauri` cargo invocation: run `bash scripts/prepare-mnema-cli-sidecar.sh debug` first on a clean checkout.
- `local-whisper` feature requires `cmake` in `PATH`.
- `speakrs` is the sole on-device diarization provider (pure-Rust pyannote-community-1 + WeSpeaker on CoreML; see [`crates/speaker-analysis/docs/adr/0003`](crates/speaker-analysis/docs/adr/0003-remove-sherpa-make-speakrs-sole-diarization-provider.md)). Its `speaker-analysis-speakrs` feature (on by default in `apps/desktop/src-tauri`) links OpenBLAS via speakrs' `openblas-static` feature: `openblas-src` **builds OpenBLAS (incl. Fortran LAPACK) from source and links it statically**, so the shipped binary carries no `/opt/homebrew/.../libopenblas.dylib` runtime dependency (that mismatch was the v0.1.9 launch crash). This needs a build-time toolchain only: `brew install gcc` (provides `gfortran` + the static `libgfortran.a`/`libquadmath.a`/`libgcc.a`); no runtime OpenBLAS install. Two build-time requirements: (1) the dynamic gfortran/quadmath runtime that openblas-src still emits is force-statically-linked in `apps/desktop/src-tauri/build.rs` (discovers archive paths from the Fortran compiler at build time — never bake them into a tracked `.cargo/config.toml`); (2) OpenBLAS's from-source `make all` links its own test programs with `-lgfortran`, so the gcc lib dir must be on the linker search path or the from-source build fails at its test link. The dev/build scripts (`scripts/dev-app.sh`, `scripts/build-macos-local-sign.sh`) handle this by sourcing `scripts/openblas-build-env.sh`; for a direct `cargo`/`bun run tauri` invocation, source it first too (`. scripts/openblas-build-env.sh`) — it exports `LIBRARY_PATH="$(dirname "$(gfortran -print-file-name=libgfortran.dylib)")"` (typically `/opt/homebrew/opt/gcc/lib/gcc/current`). **For any build you distribute** (CI release or a local release you hand to others), also export `OPENBLAS_DYNAMIC_ARCH=1`: otherwise OpenBLAS is tuned to the build machine's exact Apple Silicon core and can illegal-instruction-crash on older generations (e.g. an M4-built release on an M1). CI sets it in `macos-release.yml`. Tradeoff: the first build after a `cargo clean` compiles OpenBLAS from source (slow; needs network to fetch the source — slower still with `DYNAMIC_ARCH` since it builds every arm64 kernel).

## Workflow
- Use `@tauri-apps/plugin-dialog` for confirmations, alerts, and file dialogs — not browser-native `window.confirm`/`alert`.
- When new repo-specific gotchas are discovered, ask the user whether to add them to `CLAUDE.md` (invariants) or `docs/agents/capture-pipeline.md` (deep quirks).
