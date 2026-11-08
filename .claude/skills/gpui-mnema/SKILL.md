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
- **No blur (founder ruling 2026-11-08):** GPUI can't blur inside a window. The titlebar, popovers and any "glass" surface use a solid tinted surface token, and parity logs it as native-by-design. The titlebar surface-pill glyphs are the app's own drawn SVGs, the one allowed exception to Lucide-only.
- **Motion:** ease-out only (expo `.16,1,.3,1`, quart `.25,1,.5,1`), 120/220/420 ms. When `reduce_motion()` is on, skip to the end state.
- **Time:** read through `ui::clock::now(cx)` so tests and goldens are frozen.
- **Files ≤800 lines.** Split by responsibility. A hook enforces it.
- **Logging:** use the `log` crate. Never log content (titles, URLs, OCR, transcripts, prompts, answers, queries, keys). Pick levels per `AGENTS.md`.

## app-core API *(set by S1; update when it changes)*
- `Host` trait (`emit`, `message`, `confirm` → `tokio::sync::oneshot::Receiver<bool>`, `open_url` → `Result<(), String>`, `refresh_tray`, `open_window(WindowRequest)` → `Result<(), String>` (an `Err` means the window never appeared), `window_signal(WindowSignal)`, `exit(ExitKind)`), `Paths { data, config, cache, log }` (`Paths::from_env(bundle_id)`), `CoreEvent`.
- `AppCore::new(host, paths)` owns the one tokio runtime; `AppCore::{state, try_state, manage, spawn, spawn_blocking, block_on, runtime_handle, host, paths}`. `state::<T>()` returns `Arc<T>`.
- `CoreEvent` payloads are `serde_json::Value` (the old Tauri payload) until S2/S3 move each payload type into a crate; unit events carry none. The Tauri wire names live in `src-tauri/src/tauri_host.rs` (`tauri_event`).
- `app_core::fixtures::{seed(dir, FixtureId) -> Paths, seed_paths, FIXTURE_NOW}` for tests and goldens (feature `fixtures`, default-on; src-tauri uses `default-features = false`). Seeding sets `MNEMA_CAPTURE_INDEX_KEY_DIR` for the process. `cargo run -p app-core --example dump_fixture -- <fixture> <out.json>` writes the parity `command → JSON` map.

## Theme API *(set by S5)*
- **Colours:** `let p = theme::tokens(cx);` gives a `Palette` (Copy). Fields are the CSS names minus `--app-`, snake case: `p.bg`, `p.text_muted`, `p.accent_glow`, `p.source_mic`, `p.status_running_dot`, `p.chart_grey_3`, `p.cat_research`, … (every `+layout.svelte` token, both modes). kit.css derived fills too: `p.hairline` (border 80%), `p.wash` / `p.wash_strong` (fg 4/8%, light 5/9%; wash_strong = selected row), `p.titlebar_glass` (82%), `p.pop_bg` (raised 96%), `p.shadow_popover`.
- **Scales** (`theme::*`): `FONT_UI` (Hanken Grotesk), `FONT_MONO` (Spline Sans Mono); `text::{XS,SM,BASE,MD,LG,XL,PROSE,H1,DISPLAY}` = 10/11/12/13/16/20/14/28/40; `W_REGULAR/W_MEDIUM/W_SEMIBOLD`; `leading::{BODY,SNUG,TIGHT,ONE}`; `space::S1..S8` = 4…64; `radius::{XS,SM,MD,LG,XL,PILL}`; `size::{H_SM,H_MD,H_MINI,TITLEBAR,STATUS_BAR,TRAFFIC_LIGHTS,…}`; `breakpoint::{CMD_HINTS,STATUS_WIDE}`; `DISABLED_OPACITY`, `BUSY_OPACITY`.
- **Motion:** `motion::{FAST,MED,SLOW}` = 120/220/420 ms, `THEME_FADE` 180, `TOOLTIP_DWELL` 350; easings `motion::expo`, `motion::quart` (real cubic-beziers) for `Animation::with_easing`. GPUI animations already honour `cx.reduce_motion()`; `theme::reduce_motion::observe(cx)` keeps that flag live.
- **Modes:** `theme::init(setting, cx.window_appearance(), cx)` once (also registers the static fonts and maps everything onto gpui-kit's `ThemeColor` + fonts/radii, so kit components match); `theme::set_setting(AppearanceSetting, cx)` for Settings › Appearance; `theme::set_system(appearance, cx)` from `observe_window_appearance` (the shell wires it). A mode change cross-fades 180 ms: root views call `theme::request_fade_frames(window, cx)` in `render`; `theme::fading(cx)` for tests. `theme::mode(cx)` → `Mode::{Dark,Light}`.
- **Shell helpers** (`ui::`): `with_tip(el, text)` (350 ms dwell), `icon(path, size)` (inherits text colour; GPUI's `svg` doesn't), `icon_button`, `kbd`, `kicker`, `sep`, `popover` + `pop_in`. Icons come from `assets::MnemaAssets` (kit bundle + extra Lucide + the two titlebar glyphs, all re-stroked to 1.75); add a Lucide icon to `icon_assets!` in `assets.rs` before using it.
- **Gaps GPUI can't draw:** no letter-spacing (kicker tracking), no in-window backdrop blur, no box `scale()`. `.test_support()` changes an element's type under `test-support`, so helpers return `impl ui::Control`.

## Shared controls *(set by S6)*
- **Shape:** `ui::<control>` = one file per kit.css control. Builders are `RenderOnce` (`#[derive(IntoElement)]`): the caller owns the value and passes `on_change` / `on_click`; open / highlight / focus / transition state lives in keyed element state, so every control needs a stable `id`. Every interactive part has `.id()` + `.role()` (+ `aria_label`). `Force::{Hover,Focus,Press}` (`.force(..)`) pins a state for goldens; never move a real mouse in a golden.
- **Controls:** `Button::new(id, label)` / `Button::icon_only(id, icon_path, aria)` · `.primary() .ghost() .danger() .sm() .h(px) .icon(p) .icon_after() .accent_icon() .kbd("⌘E") .pressed() .disabled() .soft_disabled()` (aria-disabled: tip still shows) `.busy() .spinner() .tip(text) .on_click(..)`. `segmented::Segmented::new(id, aria, options, selected).md().mono().on_change(|ix,w,cx|)` (`opt_icon(label, icon)`). `switch::Switch::new(id, aria, on).on_toggle(..)`. `input::TextInput::new(id, aria, &Entity<InputState>).num().invalid().w(..)` + `input::field_warning(msg, p)`. `stepper::Stepper::new(id, aria, &InputState).range(min,max).step(n).unit("days")` (owner calls `stepper::clamp_raw` on blur/Enter). `slider::Slider::new(&Entity<SliderState>, aria).label(l, value_text)` (gpui-base headless slider + our ←/→/Home/End). `select::Select::new(id, aria, options, selected).placeholder().warn().busy().compact()`; `combobox::Combobox::new(..)` same + search. `choice::Choice::new(id, aria, Vec<ChoiceOption>, selected).list()` — tiles need a definite-width parent. `date_input::DateInput::new(id, value, today).range(min,max).period(Unit::Week).density(f).on_change(|start,end,unit,w,cx|)`; date math in `calendar.rs`.
- **Overlays:** `popover::surface(p)` / `popover(id,label,p)` + `pop_in`; `menu::{menu, menu_item, option_row}`; right-click = gpui-kit `menu::NativeMenu`. `with_tip(el, text)` (350 ms, hidden by a scroll over the owner); `Button::tip` also shows it instantly on keyboard focus (`tooltip::focus_tip`). `toast::Toasts` (one entity per window: `push(Toast::ok(..).action("Undo", true, f))`, TTL `theme::TOAST_TTL`, rise 8 px / 220 ms, drain bar). `dialog::Modal::new(id, title).lead(el).child(el).on_close(f)` (scrim 62%, Esc/scrim/× close).
- **Static pieces:** `notice::{notice, banner, inline, strong}` and `empty::empty(EmptyLayout, Tone, ..)` share `ui::Tone` (`colors(p)` = tone/bg/bd). `skeleton::{skel,line,title,thumb,circle,lines,row,chart}`. `progress::{spinner(size::SPIN*), progress(Some(v)|None), progress_meta}`. `chip::{chip(label, ChipKind), badge(tone)}`. `kbd`, `keys(&["⌘","K"])`. `window_strip::WindowStrip::new(title).action(el).on_close(f)`.
- **Helpers:** `ring(color)` shadow for opaque boxes, `ring_box(color, w, r)` for clear ones (GPUI paints a shadow under the whole box, and a shadow's radius doesn't grow with its spread, so circles need `ring_box`); `outline(color, r)` = CSS 2px/2px-offset outline; `mix(a, b, wa)` = premultiplied `color-mix`; `alpha(c, w)`; `last_change(id, value)` keys a transition on change; `focus_handle(id)` (Tab stop) + `focus_visible(&h, force, window)`; `text_width(..)` for self-sizing controls.
- **Gallery:** `cargo run -p mnema-gpui --bin gallery [group]` (⌘T flips theme). Goldens `gallery-<group>-<theme>.png` for `gallery::Group::ALL`; keep a group's viewport ≤1000 px tall or the offscreen window gets clamped to the display. Parity: `bash scripts/parity.sh gallery components|feedback` (no Tauri side).
- **Known gaps:** no in-window blur (`theme::POP_SURFACE_ALPHA`, one line), no CSS colour transitions on hover/focus (instant), no `scale()` press on tiles, hover tooltips sit at the pointer, DateInput has no drag-scrub, no range mode/presets, no week-row/month-grid calendars (S8/S12 own them).

## Timeline *(set by S8)*
- **Data:** surfaces read through a `Source`-style trait (`timeline::data::Source`), sync, called on the background executor. `CoreSource` answers empty until a command moves into app-core; `timeline::fixture::FixtureSource::load(<state>)` serves `apps/desktop/parity/fixtures/<surface>/<state>.json`, records calls (`calls()`) and can be told to fail, for timing tests. View models live in `model.rs` until the DTOs move.
- **State machines:** pure, over explicit `Instant`s from `cx.background_executor().now()` (the test clock), returning effects the view runs; the view arms one timer task for `next_deadline()`. That's how `advance_clock` reaches them.
- **Images:** `timeline::image_cache::FrameImages` is the `ImageCache` (LRU 192, failures 5 s, `drop_image` on evict). Load in `render` (it needs the draw phase), keep the last decoded image painted until the next one is ready.
- **Goldens with async work:** `VisualTestAppContext` runs a *simulated* clock: pump with `advance_clock` + `run_until_parked` until the view settles (`tests/visual/timeline.rs::settle`). Shell goldens now show the Timeline over the parity fixture.
- **GPUI:** the scroll sign is the content's motion (a browser's `deltaY` is the viewport's: negate it). Key actions for a surface register on the shell root (the focus path), not the surface div. `.test_support()` changes the type: call it last.

## Settings *(set by S13)*
- **Shape:** `settings::Settings` holds the drafts (`draft` vs a private `saved` `RecordingSettings`, plus keyboard JSON and the mic state); sections in `settings/sections/*` read `s.d()` and write through `s.edit(|d| …, cx)` / `edit_keyboard` / `edit_mic`. Build rows with `settings::row::{section, row(..).desc().control().below().full().build(p)}`; wire controls with `settings::on(cx, |s, v, cx| …)` / `settings::click(cx, …)`.
- **Autosave:** `settings/autosave.rs` is pure over `Instant`s (450 ms recording, 0 ms sources, 250 ms mic; nothing saves while `privacy_in_flight`). Saves go through `data::Backend` on the background executor; `fixture::FixtureBackend` records them for tests. Recording-domain, mic and privacy saves fail honestly until S3 moves `native_capture_settings`; keyboard saves are live.
- **Confirms** go through `settings::confirm` → `Host::confirm`; the GPUI host drops confirms (reads as Cancel) until S7.
- **Goldens:** `settings-<state>-<theme>.png` for `settings::fixture::STATES` (group tops + deeper sections reached by `open_tab`), all from `parity/fixtures/settings/default.json`, shell not recording.

## Insights *(set by S12)*
- **Data:** `insights::data::Backend` (sync, run on the background executor; every method has a "nothing / no app core" default, so `NoBackend` is empty). `CoreBackend` calls app-core's `user_context`, `usage_charts`, `app_infra` and `ai_runtime` directly; `insights::fixture::FixtureBackend::load(<set>)` serves `parity/fixtures/insights/{populated,empty}.json` and records writes (`calls()`). Loads use per-slot generation tokens, so a stale answer never lands.
- **Charts** live in `insights/charts/` (stacked bar, heatmap, focus split, sparkline + line chart on canvas paths, mini bars, confidence bar, spec rows): reuse them, don't redraw.
- **Delayed-commit Dismiss:** `insights::undo::DelayedDismiss` (5000 ms). Any navigation away (tab, back, another subject, the shell leaving Insights) calls `Insights::leave`, which commits now.
- **Shell:** Insights emits `InsightsEvent::{Ask, OpenTimeline}`; the shell routes ⌘J and Esc to it first, and bare keys (`[` `]` ↑ ↓) through `insights::register_keys` on its root.

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
- **Never touch the real keychain.** Any process that may reach app-core secrets (provider keys, the vault, the DB key) must run with `MNEMA_TRIPWIRE_NO_KEYCHAIN=1` and `MNEMA_DEV_MASTER_KEY_FILE=<scratch file>` (+ `MNEMA_CAPTURE_INDEX_KEY_DIR=<scratch dir>`). That includes throwaway fixture dumpers and examples, not just tests. On 2026-11-08 a fixture dumper hung on a real keychain read.
- **`rustfmt` on a `lib.rs`/`mod.rs` follows the whole module tree.** Format only leaf files you touched, or revert the churn.
- **Never launch an app against the founder's real data** (`~/.mnema-dev`, `~/.mnema`, `~/Library/Application Support/day.mnema*`). Startup runs retention and repair passes, which delete data. Smoke tests use a copy: `cp -R ~/.mnema-dev <scratch>/mnema-dev-copy` and point `MNEMA_SAVE_DIRECTORY`/`MNEMA_APP_CONFIG_DIR` at it, or use an `app_core::fixtures` seed.
- Never run `scripts/build-macos-local-sign.sh`: the founder runs signed builds. TCC flows (screen, mic, system audio, accessibility) only work in a signed build launched with `open`.
