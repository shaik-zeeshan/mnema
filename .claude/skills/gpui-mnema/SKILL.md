---
name: gpui-mnema
description: Conventions for building the Mnema GPUI desktop app (apps/desktop-gpui, package mnema-gpui) and the shared crates/app-core - crate pins, theme tokens, Host trait, tokio bridge, image cache, accessibility id+role rule, test helpers, build slots, the 800-line cap. Load before any work on a docs/gpui/PLAN.md slice.
---

# Mnema GPUI conventions

The plan is `docs/gpui/PLAN.md`; the brief and founder rulings are `docs/gpui/BUILD-PROMPT.md`. `AGENTS.md` invariants all apply. A section marked *(set by Sx)* is filled in when that slice lands. Until then, follow the plan text.

## Stack (exact pins, upgraded on purpose only)
- `gpui-kit = "=0.9.0"` (a facade over `gpui-component`/`gpui-base` 0.9.0; re-exports `gpui::*` from `gpui-pre =0.3.11`, Zed @ f816e1f). It already enables `gpui-pre-platform`'s `font-kit` (no text without it). Dev: `features = ["test-support"]`.
- Entry: `gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| { gpui_kit::init(cx); … })`. Every window's root view goes inside `gpui_kit::Root` (`gpui_kit::open_window` does it).
- Sources to read: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/{gpui-pre-0.3.11,gpui-pre-macos-0.3.11,gpui-component-0.9.0,gpui-base-0.9.0}`.
- Tray: `tray-icon 0.26.1` (default features off; brings `muda 0.21.2`). Hotkeys: `global-hotkey 0.9.0`. Never muda for the app menu: GPUI owns `set_menus`.
- macOS only. No `cfg` branches or abstractions for Windows/Linux.

## Facts that bite
- **Quit can't be blocked by GPUI.** `on_app_quit` gets 200 ms. The graceful exit uses our objc2 `terminate:` swizzle → `AppCore::shutdown`.
- **`on_open_urls` has no `&mut App`.** Send the URLs into an `async_channel` and handle them in a `cx.spawn` loop.
- **`reduce_motion()` is read once at init.** Our observer updates it.
- **`RenderImage` is BGRA.** Swap channels when decoding RGBA. Free GPU memory with `drop_image` when evicting.
- **Fonts:** GPUI loads one face per file and ignores a variable font's weight axis. So we embed **static** 400/500/600 TTFs (`apps/desktop-gpui/assets/fonts/*-{400,500,600}.ttf`, cut with `varLib.instancer`; decompress woff2 with `uvx --from fonttools --with brotli`) via `cx.text_system().add_fonts`. UI font is Hanken Grotesk; data/mono is Spline Sans Mono.
- **Toolchain:** pinned by `rust-toolchain.toml` (1.99.1; `gpui-pre-util` needs ≥1.93). Plain `cargo` picks it up, **but run `unset RUSTUP_TOOLCHAIN` first**: this machine's sessions inherit `RUSTUP_TOOLCHAIN=1.92.0`, which overrides the file.
- **Transparent windows:** the gpui-component root paints the theme background, so use `Root::new(..).bg(transparent_black())`.
- **Never run a nested modal loop** (NSMenu popUp, `runModal`) inside `cx.update`: GPUI re-borrows app state and panics with "RefCell already borrowed". Defer it outside the update.
- **NSWindow access:** call `HasWindowHandle::window_handle(window)` explicitly. `window.window_handle()` is GPUI's own method.
- **Golden test binaries** use `harness = false` with their own `main`, because AppKit needs the main thread.
- **Tray icon checks:** on macOS 26 the tray icon lives in Control Centre's window list. Menu-bar managers (Ice) may hide it.
- **OS event threads** (tray, hotkey, notifications): std thread on the receiver → `async_channel::send_blocking` → `cx.spawn(async move |cx| while let Ok(e) = rx.recv().await { cx.update(..) })`.

## Architecture rules
- **One `AppCore`, two hosts.** Behaviour lives in `crates/app-core`. The GPUI side holds only view state in `Entity<T>`, subscribes to `CoreEvent`s, and calls `AppCore` methods directly. There is no IPC and there are no mirrored DTOs.
- **Never block the GPUI main thread** on sqlx, IO or model work. Use `Tokio::spawn(cx, fut)` (vendored `bridge/tokio.rs`, the runtime is AppCore's) or `cx.background_spawn`.
- **Tasks and Subscriptions cancel on drop.** Store them in the entity or call `.detach()`.
- **Accessibility:** every interactive element gets `.id(..)` and `.role(gpui::Role::…)`, plus `.aria_label(..)` when it has no text. Tests and AX driving find elements by these.
- **Visuals:** colours, type, spacing, radii, control heights, easing and durations come only from `theme.rs` tokens. No literals in views. Icons are Lucide (`gpui_kit::assets`) only, never unicode glyphs; titlebar icon stroke is 1.75.
- **Motion:** ease-out only (expo `.16,1,.3,1`, quart `.25,1,.5,1`), 120/220/420 ms. When `reduce_motion()` is on, skip to the end state.
- **Time:** read through `ui::clock::now(cx)` so tests and goldens are frozen.
- **Files ≤800 lines.** Split by responsibility. A hook enforces it.
- **Logging:** use the `log` crate. Never log content (titles, URLs, OCR, transcripts, prompts, answers, queries, keys). Pick levels per `AGENTS.md`.

## app-core API *(set by S1; update when it changes)*
- `Host` trait (`emit`, `message`, `confirm` → `tokio::sync::oneshot::Receiver<bool>`, `open_url`, `refresh_tray`, `open_window(WindowRequest)`, `window_signal(WindowSignal)`, `exit(ExitKind)`), `Paths { data, config, cache, log }` (`Paths::from_env(bundle_id)`), `CoreEvent`.
- `AppCore::new(host, paths)` owns the one tokio runtime; `AppCore::{state, try_state, manage, spawn, spawn_blocking, block_on, runtime_handle, host, paths}`. `state::<T>()` returns `Arc<T>`.
- `CoreEvent` payloads are `serde_json::Value` (the old Tauri payload) until S2/S3 move each payload type into a crate; unit events carry none. The Tauri wire names live in `src-tauri/src/tauri_host.rs` (`tauri_event`).
- `app_core::fixtures::{seed(dir, FixtureId) -> Paths, seed_paths, FIXTURE_NOW}` for tests and goldens (feature `fixtures`, default-on; src-tauri uses `default-features = false`). Seeding sets `MNEMA_CAPTURE_INDEX_KEY_DIR` for the process. `cargo run -p app-core --example dump_fixture -- <fixture> <out.json>` writes the parity `command → JSON` map.

## Theme API *(set by S5)*

## Shared controls *(set by S6)*

## Tests
- **Behaviour:**
  - `#[gpui::test]` with `TestAppContext` / `VisualTestContext`;
  - `cx.simulate_keystrokes("cmd-k")`, `cx.executor().advance_clock(Duration::from_millis(449))` then `run_until_parked()`;
  - `gpui_kit::test::TestWindowExt` (`find`, `click`, `hover`, `press`, `input`, `scroll`, `drag`).
  - Assert every timing constant on both sides of its boundary.
- **Goldens:**
  - `VisualTestAppContext::with_asset_source(platform, Arc::new(gpui_kit::assets::Assets))` → `capture_screenshot(window)`;
  - baselines at `apps/desktop-gpui/tests/visual/baselines/<surface>-<state>-<theme>.png`, compared at tolerance 2/channel with a 0.99 match; on failure a `*_diff.png`;
  - `UPDATE_BASELINE=1` regenerates them only with founder sign-off.
  - Open the PNGs with Read and look at them.
- **Parity:** `bash scripts/parity.sh <surface> [state] [theme]` → `target/parity/…/composite.png` (GPUI | Tauri | mockup). The GPUI visual test must write `$PARITY_DIR/gpui.png` for `$PARITY_STATE`/`$PARITY_THEME` when those are set. Port :1420 must be free or serve this worktree.

## Building
- Agent worktrees are created from `main`: first `git merge --ff-only new-gpui-desktop-app` (or the base commit in your brief).
- Your brief gives you a build slot N (1-3). Run `export CARGO_TARGET_DIR=$HOME/.cargo/worktree-targets/gpui-slot-$N`. Never share a slot with a live agent: a shared target dir gives phantom errors, and each one costs ~20 GB.
- `export CARGO_INCREMENTAL=0`: incremental caches doubled slot sizes and filled the disk twice on 2026-11-08 (~25 GB free with 3 slots).
- `. scripts/openblas-build-env.sh` before any build linking speakrs. `bash scripts/prepare-mnema-cli-sidecar.sh debug` before any `apps/desktop/src-tauri` cargo command.
- `rustfmt` on touched files only. Never a bare `cargo fmt`, which churns ~17 unrelated files.
- The four `resume_screen_from_inactivity_*` failures and the known flakes already exist. Don't chase them, and don't add to them.
- **Never launch an app against the founder's real data** (`~/.mnema-dev`, `~/.mnema`, `~/Library/Application Support/day.mnema*`). Startup runs retention and repair passes, which delete data. Smoke tests use a copy: `cp -R ~/.mnema-dev <scratch>/mnema-dev-copy` and point `MNEMA_SAVE_DIRECTORY`/`MNEMA_APP_CONFIG_DIR` at it, or use an `app_core::fixtures` seed.
- Never run `scripts/build-macos-local-sign.sh`: the founder runs signed builds. TCC flows (screen, mic, system audio, accessibility) only work in a signed build launched with `open`.
