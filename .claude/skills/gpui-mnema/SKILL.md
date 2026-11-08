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
- **Fonts:** embedded TTF from `apps/desktop-gpui/assets/fonts` via `cx.text_system().add_fonts`. UI font is Hanken Grotesk; data/mono is Spline Sans Mono.
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
- `Host` trait (`emit`, `message`, `confirm`, `open_url`, `refresh_tray`, `open_window`, `window_signal`, `exit`), `Paths`, `CoreEvent`, `AppCore::{state, try_state, manage, spawn, spawn_blocking, block_on, host, paths}`.
- `app_core::fixtures::seed(dir, FixtureId)` for tests and goldens (feature `fixtures`).

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
- **Parity:** `bash scripts/parity.sh <surface> [state] [theme]` → `target/parity/…/composite.png` (GPUI | Tauri | mockup).

## Building
- Your brief gives you a build slot N (1-3). Run `export CARGO_TARGET_DIR=$HOME/.cargo/worktree-targets/gpui-slot-$N`. Never share a slot with a live agent: a shared target dir gives phantom errors, and each one costs ~20 GB.
- `. scripts/openblas-build-env.sh` before any build linking speakrs. `bash scripts/prepare-mnema-cli-sidecar.sh debug` before any `apps/desktop/src-tauri` cargo command.
- `rustfmt` on touched files only. Never a bare `cargo fmt`, which churns ~17 unrelated files.
- The four `resume_screen_from_inactivity_*` failures and the known flakes already exist. Don't chase them, and don't add to them.
- Never run `scripts/build-macos-local-sign.sh`: the founder runs signed builds. TCC flows (screen, mic, system audio, accessibility) only work in a signed build launched with `open`.
