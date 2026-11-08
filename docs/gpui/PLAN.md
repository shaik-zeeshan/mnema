# Mnema GPUI desktop app: PLAN

> Approved by the founder 2026-11-08. The rules come from `docs/gpui/BUILD-PROMPT.md` (§ numbers below point there). Facts were re-checked on 2026-11-08 by three research agents against `main` @ 42477c48 and the crate sources in `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.

## Context

Mnema's desktop app is Tauri + Svelte. We are building a native GPUI app next to it that looks, feels and behaves the same and reuses the Rust backend. The shared logic moves out of `apps/desktop/src-tauri` into a new Tauri-free crate, `crates/app-core`. Both apps run on it until the GPUI app reaches parity (S17). Then Tauri and Svelte are deleted. "The same" is measured with golden screenshots, side-by-side parity composites and timing tests (§5, §6).

## What research changed in the brief

Each item below changes a slice. None of them reopens a founder ruling.

1. **GPUI cannot hold up quit.** `on_app_quit` futures get 200 ms (`gpui-pre-0.3.11/src/app.rs:80`, `:1310`), and the macOS delegate has no `applicationShouldTerminate:`. So the graceful exit keeps today's `terminate:` swizzle (`windows.rs:1048-1092`), ported from objc 0.2 to objc2. S0 proves it works on GPUI's `NSApplication`.
2. **The fonts are woff2 variable fonts only** (`apps/desktop/static/fonts`). GPUI loads TTF/OTF. S0 converts them with fontTools and checks that weights 400/500/600 render differently. If they don't, it cuts static instances.
3. **The parity harness does not exist.** It's only a recipe in `DEVELOPER-PLAN.md:95-117`. New slice **P0** builds it.
4. **No test uses a Tauri mock app.** All 1312 tests pass `None` for `Option<&AppHandle>`, so the extraction rewrites no test harness. A fake `Host` also makes the untested `Some(handle)` branches testable.
5. **Some moves have to happen together.** `native_capture` → `app_infra` (heavy), `ask_ai` → `native_capture` start/stop, and `native_capture` → `ask_ai::mcp`. So S3 (capture) can't run in parallel with S2. It runs after S2's `app_infra` batch, and `ask_ai`/`conversation`/`debug_health` move with capture.
6. **Build-time plumbing has to follow the moved code.** `rustc-link-arg` doesn't propagate from a lib, so each final binary keeps the static gfortran logic: one shared `scripts/build/fortran_static_link.rs`, `include!`d by both binaries' `build.rs`. `option_env!` values (`MNEMA_BUILD_DATE_MS`, license/CRL keys) bake into the crate that holds the code, so app-core gets its own `build.rs`. `MNEMA_APP_IDENTIFIER` is read from `tauri.conf.json` by `crates/{app-infra,cli}/build.rs` and is always `day.mnema`. The GPUI dev app relies on `MNEMA_APP_CONFIG_DIR`/`MNEMA_SAVE_DIRECTORY` (already honoured by app-infra and the CLI). S17 moves the identifier source.
7. **Small API facts.**
   - `on_open_urls` gets no `&mut App`, so URLs go through a channel.
   - `reduce_motion` is read once at init. S5 adds an `NSWorkspace` accessibility-display observer.
   - `RenderImage` wants BGRA.
   - `VisualTestAppContext` needs `with_asset_source` for icons to render.
   - `gpui_tokio` is not published, so we vendor its 100 lines (needs `gpui-pre-util`).
   - gpui-kit 0.9.0 is a facade over `gpui-component`/`gpui-base` 0.9.0.
   - Tests use `gpui_kit::test::TestWindowExt` under `test-support`.
8. **Counts.** 36 managed states (31 in the builder at `lib.rs:537-572`, plus 5 inside `app_infra::initialize`, `app_infra.rs:1610-1646`). 248 commands. 32 event names across 64 sites. The panel reclass is at `windows.rs:677-746`, not `macos.rs`. NSDateFormatter "j" is at `clock_format.rs:35-53`.

## Orchestrator decisions (mine, small, reversible)

- **AppCore keeps a typed state map** (`core.state::<T>()` / `try_state::<T>()`), mirroring `app.state::<T>()`. The extraction then becomes a mechanical rename of ~185 call sites instead of a redesign. Trade-off: one runtime lookup per call, like today. Typed fields can come later, file by file.
- **One tokio runtime, owned by AppCore.** The Tauri host calls `tauri::async_runtime::set(core.runtime())`. The GPUI host calls the vendored `gpui_tokio::init_from_handle`. The 104 `tauri::async_runtime::*` sites become `core.spawn*`.
- **Same data on both sides by construction.** App-core gets a `fixtures` module (feature `fixtures`) that seeds a temp data dir: DB rows plus generated JPEG frames, a fixed "now", and one function per surface × state. A `dump-fixture` example calls the real AppCore commands on that seed and writes `command → JSON` maps. Those maps are what the Playwright stub returns to Svelte. The GPUI tests open the same seed. Mockups keep their own baked data, and the parity agent ignores content differences there.
- **Build slots.** Each agent target dir is ~20 GB and only 69 GB is free. At most **3 concurrent cargo-building agents**, each with `CARGO_TARGET_DIR=~/.cargo/worktree-targets/gpui-slot-{1,2,3}`. The brief assigns the slot, and no two live agents share one. So the six UI slices run in two waves of three.
- **Crate names.** `crates/app-core` (package `app-core`) and `apps/desktop-gpui` (package `mnema-gpui`), both workspace members.

## Order of work

```
T0 tooling ─┬─ S0 spike ── STOP: show founder ──┬─ S1 ─ S2 (B0→B6) ─ S3 ─ S4 ─┐
            └─ P0 parity harness (with S0/S1)   └─ S5 ─ S6 ──────────────────┼─ S7 ─ S16 ─ S17
                                                       wave A: S8 S11 S13    │
                                                       wave B: S12 S10* S15* ┘ (* need S7)
                                                       S9 after S8; S14 after S7
```

## Rules every slice follows (also in `.claude/skills/gpui-mnema/SKILL.md`)

- Load the `gpui-mnema` skill first. Read `AGENTS.md`. Its invariants all apply: logging levels, no content or secrets in logs, migrations never deleted, no legacy shims, new `MNEMA_*` added to `turbo.json passThroughEnv`, ≤800 lines per file.
- **Work in your own worktree.** Never edit, stash or `git checkout --` the shared worktree. Hand back a patch (`git diff > /scratch/<slice>.patch`), the test output, and the screenshot paths. The orchestrator re-applies the patch and re-runs the verify commands.
- **Cargo setup.** `export CARGO_TARGET_DIR=~/.cargo/worktree-targets/gpui-slot-<N>` (N is given in your brief). Run `. scripts/openblas-build-env.sh` before any build that links speakrs (app-core, desktop, gpui). Run `bash scripts/prepare-mnema-cli-sidecar.sh debug` before any `apps/desktop/src-tauri` cargo command. Run `rustfmt` on touched files only, never bare `cargo fmt`. Use background Bash for long builds; no sleep-polling.
- **Known failures.** The four `resume_screen_from_inactivity_*` failures and the known flakes (native_capture global state, ask_ai pidfile, the occasional chatgpt_auth hang) already exist. Don't chase them, and don't add new ones.
- **GPUI conventions.**
  - Pins: `gpui-kit = "=0.9.0"` (default features; `test-support` in dev). Every interactive element gets `.id()` + `.role()`.
  - Never block the main thread. Use `Tokio::spawn` or `background_spawn`, and store or `.detach()` every Task/Subscription.
  - Colours, sizes and motion come only from `theme.rs` tokens. Icons are Lucide only, never unicode glyphs.
- **Done means:**
  - the layers named in the slice pass;
  - the golden PNGs were opened with Read and looked at;
  - the parity gap list is attached for UI slices;
  - native-by-design gaps (§9.4: scroll physics, text selection, context menus, focus ring, IME, window drag/resize) are labelled as such.

---

## Status log

- **2026-11-08 T0 done** (2c315085). **P0 done** (0abcfa90).
- **S0 done:** all 11 items pass.
  - Cold start ~160 ms release (to first paint).
  - Idle CPU 0.0% with every window hidden.
  - The Tauri app still builds with the shared lock.
  - Fonts need static cuts.
  - Needs Rust ≥1.93: the founder chose to pin stable, and `rust-toolchain.toml` = 1.99.1.
  - Quit swizzle → AppKit's original `terminate:` still fires GPUI's `on_app_quit`.
  - Open for S7: a real tray click (founder), and an unexplained off-screen 500×500 window with sharingState 1.
- **S1 done:**
  - app-core tests 10/10; desktop lib 1313 passed / 0 failed on Rust 1.99.1 (the 4 `resume_*` failures no longer reproduce);
  - `otool -L` unchanged;
  - `CoreEvent` payloads are `serde_json::Value` until S2/S3 move their types (shortcut);
  - new 1.99 warnings: `fetch_update` deprecated (`licensing.rs:129`), `__eh_frame` too large linker message, `block 0.1.6` future-incompat.
  - Incident: the S1 smoke ran against the real `~/.mnema-dev`, and its 7-day retention deleted 37 old segments. Rule added: agents launch only against copies.
- **S2 B0 + B1 done** (d9820408, 630dab6f).
- **S5 done:**
  - 33 behaviour tests; 12 shell goldens (approved by the founder after viewing, 2026-11-08).
  - Shell data is empty in the real app until S2/S3 move the commands (`shell/data.rs` shortcuts).
  - **Founder rulings, 2026-11-08:**
    - no in-window blur: titlebar and popovers use **solid tinted surfaces**, logged as native-by-design;
    - the titlebar surface glyphs may stay the app's own drawings (an allowed exception to Lucide-only).
  - **Still open:** GPUI has no letter-spacing (kicker 0.12em).
  - The `terminate:` swizzle lives at git f45ddec5 `apps/desktop-gpui/src/spike/native.rs`, for S7.
- **S2 B2 + B5 merged:**
  - B5 left `broker_authorization_channel` for B6 (see the B6 seams).
  - `Host::open_url` now returns `Result` and opens absolute paths with `open_path`. This fixes a B2 regression where "open log" lost its error toast.
  - app-core now depends on `global-hotkey 0.7` (the Tauri plugin's version) for shortcut validation, while the GPUI app pins 0.9. Reconcile in S7: app-core validates strings, and each host converts.
- **S6 done:**
  - 42 tests; 20 gallery goldens approved by the founder after viewing (2026-11-08). To do: drop the "pressed primary" gallery cell (the app never uses a primary toggle) and regenerate `gallery-buttons-*` (pre-approved).
  - Gaps logged for polish:
    - hover/focus colour transitions are instant (spec 120–220 ms quart);
    - no popover scale-in;
    - the hover tooltip follows the pointer;
    - the date input lacks drag-scrub and range/year modes (S8/S12 add them where needed);
    - the light-theme modal scrim is faint without blur;
    - the pressed primary button is unreadable (a kit.css bug reproduced faithfully: founder call).
  - Facts:
    - offscreen goldens taller than ~1040 px get clamped, so keep viewports ≤1000 px;
    - GPUI shadows don't follow radius+spread (use `ui::ring_box` for focus rings).

### Progress report (founder rule, 2026-11-08)
After **every** merged slice/feature:
1. update `docs/gpui/report/data.json` (now / done / next / metrics / screenshots);
2. run `python3 docs/gpui/report/build.py`;
3. run `npx -y postplan@latest upload target/gpui-report/index.html --draft d9ymz3pr4vyt`.

The live URL is https://d9ymz3pr4vyt.postplan.dev. It's public by link, so never put secrets, keys, private paths or real user data in it.

### Next up (orchestrator queue; keep current)
1. **Running, two at a time:** S3b `segments.rs` (slot 3) and S13 Settings (slot 1). S3a done in 5467e174. S12 Insights waits for a free slot (its old agent never wrote anything, so relaunch it fresh). Old worktrees:
   - S3a capture leaves: `/Users/shaikzeeshan/Code/mnema/.claude/worktrees/agent-ac2748086a4bbf839`
   - S13 Settings: `/Users/shaikzeeshan/Code/mnema/.claude/worktrees/agent-aef107fcd394658f0`
   - S12 Insights: `/Users/shaikzeeshan/Code/mnema/.claude/worktrees/agent-acad0b6ce3e5374fa`
   - Done before the pause: S8 2a17488a, S11 bc38c3bb, B6b 26e994bb. **S2 is complete.**
   - Waiting for founder sign-off: Timeline (10 new) + Chat (10 new) + shell (12 changed) baselines.
   - Queued, no slot yet: the S8 perf pass (fix the frame-time metric to work-per-frame + dropped frames, and chase the scrub stutters up to 170 ms); drop the "pressed primary" gallery cell; S15 small windows; S10 Quick Recall (needs S7); S9 AudioDrawer.
   - The hourly auto-resume cron is off. Resume only when asked.
2. **Waiting for S3** (all in src-tauri): app_infra startup/retention/processing workers + `run_deferred_startup_blocking` (read NativeCaptureState), voice_enrollment, select_semantic_search_model, wipe_user_context, the privacy-exclusion apply step, the installed-app list.
3. **UI waves:** S13 Settings next, then S12 Insights, S10 Quick Recall and S15 small windows.
5. **Docs debt:** `AGENTS.md` and `docs/user-context/CONTEXT.md` still name src-tauri paths for ai_runtime and user_context. Update them in one pass after S3.
4. **After B6b:** S3 (capture + ask_ai), then S4, then S7.

## T0 · Tooling (orchestrator, before S0)

**Goal:** the repo has the plan, subagents, skill and hooks that every later slice relies on.

**Build:**
1. Copy this plan to `docs/gpui/PLAN.md`.
2. Write `.claude/agents/`, each short and specific:
   - `gpui-slice-builder`: one slice, in a worktree, on a slot; returns patch + test output + PNG paths.
   - `gpui-parity-checker`: read-only; runs P0 for one surface; returns a gap table with severity and the composite path.
   - `gpui-feel-checker`: runs the layer-2 timing tests + the layer-5a budget report; returns a pass/fail table.
   - `core-extractor`: moves one src-tauri module into app-core behind `Host`, keeping both hosts compiling.
3. Write `.claude/skills/gpui-mnema/SKILL.md`: the conventions above, plus the APIs as S0/S1/S5/S6 settle them (Host trait, CoreEvent, tokio bridge, image cache, theme token API, test helpers, fixture API). Each slice that changes a convention updates the skill.
4. Hooks in project `.claude/settings.json` (via the `update-config` skill):
   - PostToolUse on Edit/Write of `apps/desktop-gpui/**/*.rs` or `crates/app-core/**/*.rs`: `rustfmt --edition 2021 <file>`, then fail if `wc -l` > 800.
   - Stop hook: `cargo check -p app-core -p mnema-gpui`. It runs only when those crates exist and have changes in `git status`, so it doesn't add minutes to idle stops.
5. Commit: `chore(gpui): plan, agents, skill, hooks`.

**Verify:** `claude` lists the 4 agents. Editing a dummy 801-line `.rs` under `apps/desktop-gpui` trips the hook. Then remove the dummy.

---

## S0 · Spike: prove the stack (stop and show the founder before S1)

**Goal:** one throwaway binary proves every risky platform piece works with gpui-kit 0.9 inside our workspace.

**Read:** the `gpui-mnema` skill. Ropy `src/app.rs:191-301`, `src/gui/{tray,hotkey}.rs`, `src/utils.rs:34-60` (clone to scratch at `abf21ef`). Zed `crates/gpui_tokio/src/gpui_tokio.rs` @ `f816e1f`. `gpui-pre-0.3.11/src/app/visual_test_context.rs`. `windows/macos.rs:29-66` (sharingType). `windows.rs:1048-1092` (terminate swizzle).

**Build** (crate `apps/desktop-gpui`, package `mnema-gpui`, added to workspace `members`; spike code in `src/bin/spike.rs` + `src/spike/`; S5 deletes it):
1. **Fonts.**
   - `uvx --from fonttools fonttools ttLib.woff2 decompress` both woff2 files into `apps/desktop-gpui/assets/fonts/*.ttf`. Copy `OFL.txt`.
   - Register them with `cx.text_system().add_fonts`.
   - Render a line at 400/500/600 in Hanken Grotesk and Spline Sans Mono. If the variable weights don't resolve, cut static instances with `fonttools varLib.instancer` and record that.
2. **Window.** gpui-kit `Root` and a few §3.7 tokens (bg, fg, accent), dark + light. Main-style titlebar: `appears_transparent`, traffic lights at the 78 px inset, `app_owns_titlebar_drag` + `start_window_move`.
3. **Panel.** `WindowKind::PopUp` 800×60, 16 px corners, transparent background. Read its NSWindow via `HasWindowHandle` and check: non-activating, floating, all Spaces, can become key.
4. **sharingType.** Set `NSWindow.sharingType = none` via objc2 before the first show. Check with a CGWindowList query (`kCGWindowSharingState == 0`).
5. **Tray.** `tray-icon 0.26.1` (default features off) with a template icon and a 3-item muda menu. Wire it std thread → `async_channel` → `cx.spawn` → `cx.update` (the ropy pattern).
6. **Global hotkey.** `global-hotkey 0.9.0`, ⌘⌥Space toggles the panel, same channel pattern.
7. **Deep links.** `Application::on_open_urls` registered before `run`, forwarded through a channel. Wrap the binary in a scratch `.app` with `CFBundleURLTypes` (`mnema-gpui-dev`) and ad-hoc sign it. Use a scratch script, never `build-macos-local-sign.sh`. Then `open 'mnema-gpui-dev://open/x'` while it's running and while it isn't.
8. **Tokio bridge.** Vendor gpui_tokio at `src/bridge/tokio.rs`. Run one sqlx query on a temp SQLite DB through `app_infra`'s sqlx version, and show the result in the window.
9. **Graceful quit.** Port the `terminate:` swizzle to objc2. ⌘Q runs an async 2 s "shutdown" on tokio, then exits. Check whether GPUI's own quit path still fires `on_app_quit`.
10. **Golden test.** `tests/visual_spike.rs`: `VisualTestAppContext::with_asset_source(.., gpui_kit::assets::Assets)` → `capture_screenshot`, compared against `tests/visual/baselines/spike-window-dark.png`, tolerance 2/channel at a 0.99 match. Port zed's comparator into `tests/visual/compare.rs`, which later slices reuse.
11. **Measure:** cold start to first paint (log a timestamp in `main` and in the first `render`), idle CPU with every window hidden (`top -l 5 -pid`), and that the workspace `Cargo.lock` resolves with Tauri and GPUI together (no `links` conflicts).

**Don't:** touch src-tauri or any crate. Polish visuals. Add Windows/Linux `cfg`s.

**Files:** `Cargo.toml` (members), `Cargo.lock`, `apps/desktop-gpui/{Cargo.toml,build.rs?,src/bin/spike.rs,src/spike/*,src/bridge/tokio.rs,assets/fonts/*,tests/visual_spike.rs,tests/visual/compare.rs,tests/visual/baselines/spike-*.png}`, `scripts/gpui-spike-app.sh` (scratch `.app` wrapper).

**Verify:**
```
. scripts/openblas-build-env.sh
cargo build -p mnema-gpui --bin spike
cargo test -p mnema-gpui --test visual_spike
bash scripts/gpui-spike-app.sh && open target/gpui-spike/MnemaSpike.app && open 'mnema-gpui-dev://open/x'
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml   # Tauri still builds with the shared lock
```
Plus screenshots of the window (light/dark), the panel and the tray menu, opened and looked at.

**Report to founder:** a pass/fail table for the 11 items, the surprises, the numbers from step 11, and whether anything changes the plan. **Then stop.**

**Depends on:** T0.

---

## P0 · Parity harness (runs alongside S0/S1)

**Goal:** one command produces `GPUI | Tauri | mockup` composites for a surface × state × theme, and an agent can write a gap list from them.

**Read:** `docs/mockups/redesign/DEVELOPER-PLAN.md:95-117`, `docs/mockups/redesign/kit.js:1-20` (`?theme=`), the `?state=` handling in each mockup page (`access.html` uses `?view=`; `onboarding.html` adds `&step &perm &folder`), and memory `mnema-verify-ui-by-rendering`.

**Build:**
1. `apps/desktop/parity/`:
   - Add `@playwright/test` as an `apps/desktop` devDependency.
   - `tauri-stub.ts`: an `addInitScript` defining `__TAURI_INTERNALS__` (`invoke`, `transformCallback`, `convertFileSrc`, window label in `metadata`) and `__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener`.
   - Invoke reads a `command → JSON` map. Unmocked commands return the empty value of their real Rust return type (`null` for Option, `[]` for Vec incl. `drain_*`, `false`, `0`, `""`), with name rules (`[]` for `list_*`, `false` for `has_pending*`, `{}` otherwise) only as the fallback. *(Built 2026-11-08: a fake `{}` for an Option-returning command shows phantom overlays.)* `/debug` needs `developerOptionsEnabled: true`.
   - The page clock is fixed with `page.clock.setFixedTime` at the fixture's "now".
2. `apps/desktop/parity/shoot.ts <surface> <state> <theme>`:
   - Starts or reuses Vite on :1420.
   - Screenshots the route at a fixed viewport, and the mockup at `file://…/<page>.html?state=..&theme=..` (window element only).
   - Writes `target/parity/<surface>/<state>-<theme>/{tauri,mockup}.png`.
3. `scripts/parity.sh <surface> [state] [theme]`:
   - Runs `shoot.ts`, plus the GPUI side: `PARITY_DIR=<dir> PARITY_STATE=<state> PARITY_THEME=<theme> cargo test -p mnema-gpui --test visual -- <surface>` must write `$PARITY_DIR/gpui.png`.
   - Composes `composite.png` with ImageMagick `montage` (or a 30-line Rust bin if it's missing).
4. A surface → route → mockup page → states table in `apps/desktop/parity/surfaces.ts`, covering every surface in §3.6.
5. **Until S1 lands the fixture dumper:** hand-written JSON maps for the shell only. S1 replaces them with generated ones.

**Don't:** add Playwright to CI. Change Svelte code to make it screenshot-friendly. Hide or omit unmocked commands with `null`.

**Files:** `apps/desktop/package.json`, `apps/desktop/parity/*`, `scripts/parity.sh`, `.gitignore` (`target/parity`).

**Verify:** `bash scripts/parity.sh shell populated dark` produces a composite. The Tauri half shows the real shell with data, not a frozen blank. Open it and look.

**Depends on:** T0.

---

## S1 · app-core skeleton

**Goal:** `crates/app-core` exists with `Host`, `Paths`, `CoreEvent`, `AppCore` and the fixtures module. The Tauri app runs on it unchanged, with no behaviour moved yet.

**Read:** `lib.rs:535-1007` (startup, the `.manage()` list at 537-572), `app_infra.rs:1505,1610-1646` (dirs + states added there), the 32 events (table in this plan's appendix A), `native_capture_settings.rs:75-90` (save dir), `lib.rs:49-67` (log constants).

**Build:**
1. `crates/app-core/src/host.rs`:
   ```rust
   pub trait Host: Send + Sync + 'static {
       fn emit(&self, event: CoreEvent);
       fn message(&self, req: MessageDialog);                       // fire-and-forget
       fn confirm(&self, req: ConfirmDialog) -> oneshot::Receiver<bool>;
       fn open_url(&self, url: &str);
       fn refresh_tray(&self);
       fn open_window(&self, req: WindowRequest);                   // Main{tab}, Settings{tab}, CliAccessRequest{..}, Update, PermissionHelper{..}
       fn window_signal(&self, s: WindowSignal);                    // CaptureExclusionChanged, HideAllViews(bool), ...
       fn exit(&self, kind: ExitKind);
   }
   ```
2. `paths.rs`: `Paths { data, config, cache, log }`, built from a bundle id plus the existing `MNEMA_*` overrides. Unit-test both the Tauri dev and GPUI dev layouts.
3. `event.rs`: `enum CoreEvent`, one variant per event in appendix A. Window-targeted events (`open_settings_tab`, `quick_recall_*`) stay host-side and are not variants.
4. `core.rs`:
   - `AppCore { host: Arc<dyn Host>, paths, runtime: tokio::runtime::Runtime, states: StateMap }`, with `state::<T>()` / `try_state::<T>()` / `manage(T)` / `spawn` / `spawn_blocking` / `block_on`.
   - `AppCore::new(host, paths)` registers the 31 builder `Default` states (the types stay in src-tauri for now; S2 moves them, so S1 registers only what already lives in crates).
   - `log = "0.4"` dependency.
5. `build.rs`: stamps `MNEMA_BUILD_DATE_MS` and the `rerun-if-env` lines for the license/CRL `option_env!`s (copied from src-tauri `build.rs`).
6. `scripts/build/fortran_static_link.rs`: extracted from src-tauri `build.rs`, which now `include!`s it.
7. src-tauri:
   - `tauri_host.rs` implements `Host`: `emit` maps each `CoreEvent` to its old name and payload, `confirm` uses the dialog plugin, `refresh_tray` → `status_bar::refresh`, `open_window` → `windows::*`, `exit` → the existing exit coordinator.
   - `lib.rs` builds `Arc<AppCore>`, `.manage()`s it, and calls `tauri::async_runtime::set(core.runtime_handle())`.
8. `fixtures.rs` (feature `fixtures`):
   - `seed(dir, FixtureId)` writes DB rows through app-infra plus generated JPEGs, at a fixed now of `2026-11-04T14:32:00+05:30`.
   - Fixture ids: `empty`, `populated`, `processing`, `error`.
   - `examples/dump_fixture.rs <fixture> <out.json>` calls the read commands (whatever is in app-core so far, and grows with S2/S3) and writes the `command → JSON` map P0 consumes.

**Don't:** move any module's behaviour, change event names or payloads, or touch Svelte.

**Files:** `Cargo.toml`, `crates/app-core/**`, `apps/desktop/src-tauri/{Cargo.toml,build.rs,src/lib.rs,src/tauri_host.rs}`, `scripts/build/fortran_static_link.rs`.

**Verify:**
```
. scripts/openblas-build-env.sh && bash scripts/prepare-mnema-cli-sidecar.sh debug
cargo test -p app-core
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib    # same pass/fail set as before (4 known failures)
# dev-sandbox smoke on a COPY of ~/.mnema-dev (never the real dir: startup retention deletes data)
```
Also add a test that every `CoreEvent` variant maps to exactly its old event name.

**Depends on:** S0 sign-off.

---

## S2 · Extract the low-coupling modules (Workflow W1, in batches)

**Goal:** every src-tauri module except capture/ask_ai/windows/tray/updater lives in app-core with its tests, and its Tauri commands are one-liners.

**Read:** the S1 code, the `core-extractor` agent definition, and appendix B (batches and dependency edges).

**Build:** one W1 run per batch, in this order. Each batch: one `core-extractor` per module in parallel worktrees (≤3 at once because of the build-slot rule) → the orchestrator applies the patches in order → runs verify once.
- **B0 foundation:**
  - `native_capture_debug_log.rs` → `app_core::log`, keeping the caller-module target and not adding a wrapper;
  - `native_capture_settings.rs` (RecordingSettingsState, read/write, paths);
  - the model-download-lock helpers (`app_infra.rs:2031-2082`);
  - the `AppInfraState` alias.
- **B1 pure leaves:** clock_format, third_party_notices, ocr_budget, speaker_analysis_runtime, licensing/adapter, conversation.rs types, native_capture_output, native_capture_inactivity (+system_idle).
- **B2:** chatgpt_auth, one_time_prompts, general_app_log, managed_storage_layout (+disk_space), usage_charts, debug_pipeline, debug_status, transcription_deepgram.
- **B3 models + semantic:** ocr_models, speaker_analysis_models, audio_transcription_models, semantic_search_models + semantic_search_worker (a cycle, so they move together), semantic_search_query, voice_enrollment.
- **B4 AI:** ai_runtime + user_context (a cycle, together), privacy_redaction_sources, sensitive_capture_recommendations.
- **B5:**
  - licensing: its `app_updates::spawn_update_check` and `status_bar::refresh` calls become `CoreEvent::LicenseStatus` (the Tauri host reacts by checking for updates and refreshing the tray);
  - crl_refresh;
  - broker_authorization_channel (`windows` → `Host::open_window`);
  - the keyboard_bindings model (parse, validate, persist; registration stays host-side).
- **B6:** app_infra (+frame_preview, quick_recall) and cli_access, plus `broker_authorization_channel` (deferred from B5) with these seams (orchestrator ruling, 2026-11-08):
  - `Host::open_window` returns `Result<(), String>`, so a failed open cancels the request as `closed`, as today;
  - `WindowSignal::CloseCliAccessRequest` → the Tauri host `destroy()`s the window (not `close()`, which would fire the CloseRequested cancel);
  - `OnboardingStateStore` (+ `current_onboarding_state_for_app`) moves out of `windows.rs` into app-core state instead of becoming a host query. `windows::is_graceful_exit_in_progress` and `shutdown_background_workers_for_app_exit` go behind AppCore's exit coordinator.

Each module's moves:
- `&AppHandle` → `&AppCore`;
- `app.state::<T>()` → `core.state::<T>()`;
- `path().app_*_dir()` → `core.paths()`;
- `emit` → `core.host().emit(CoreEvent::…)`;
- `tauri_plugin_log::log` → `log`;
- `tauri::async_runtime` → `core.spawn*`.

Its tests move with it. Each Tauri command becomes `fn x(core: State<Arc<AppCore>>, …) -> … { app_core::x::x(&core, …) }`. New `Some(core)` tests run on a `FakeHost` that records emits.

**Don't:**
- Redesign a module, rename a command, or change a payload.
- Leave a forwarding shim in src-tauri: delete the old file.
- Move `windows`, `status_bar`, `app_updates` I/O, `webview_cache` or the `lib.rs` builder.
- Delete migrations.

**Files:** `crates/app-core/src/<module>/**`, `apps/desktop/src-tauri/src/{lib.rs,<module> commands}`.

**Verify (per batch):**
```
cargo test -p app-core
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
bun run check
git diff --stat main -- 'crates/app-infra/migrations'   # must be empty
```
After B6, also run a dev-sandbox smoke: the timeline loads, Ask AI answers one question, and Settings saves.

**Depends on:** S1.

---

## S3 · Extract capture + ask_ai

**Goal:** the recording lifecycle and Ask AI live in app-core. Tauri handlers are thin.

**Read:**
- `native_capture/lifecycle.rs` (12 AppHandle methods, all pass-through: 161, 203, 226, 251, 310, 329, 352, 566, 646, 699, 760, 1014);
- the 36 functions in `segments.rs` (appendix C);
- `native_capture.rs:817-849, 1498-1538` (emits, the display-recovery static);
- `not_recording_reminder.rs`;
- `docs/agents/capture-pipeline.md`, `docs/agents/ask-ai-streaming.md`.

**Build** (W1 again, strictly in order; each step compiles both hosts):
1. Already free: runtime, disk_space, output, browser_url_ax.
2. metadata, activity, capture_health, system_audio, microphone_away.
3. microphone, privacy.
4. `segments.rs`:
   - `&AppHandle`/`Option<&AppHandle>` → `&AppCore`/`Option<&AppCore>`;
   - the license-block dialog (1538-1549) → `Host::message`;
   - `status_bar::refresh` → `Host::refresh_tray`;
   - `windows::*` calls → `Host::window_signal` / `open_window`;
   - split `segments.rs` (5948 lines) by responsibility into ≤800-line files.
5. `lifecycle.rs`, then the `native_capture.rs` root (3683 lines, split too). The static `display_reconfiguration_recovery_app_handle` becomes a `Weak<AppCore>`.
6. `not_recording_reminder.rs` → app-core. `macos_notification.rs` stays host-side behind `WindowSignal::NotRecordingReminder{..}`, so the GPUI host can reuse the same file or GPUI's `show_system_notification`.
7. conversation + ask_ai (+mcp/*), debug_health.

**Don't:** change capture behaviour, timings or the liveness rules. Touch ADR-0021/0052 logic beyond the mechanical moves. Make any non-mechanical edit to tests.

**Files:** `crates/app-core/src/{native_capture,ask_ai,conversation,debug_health}/**`, src-tauri command files, `lib.rs`.

**Verify:**
```
cargo test -p app-core            # capture tests (~16k lines) green except the 4 known
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```
Then a real recording smoke in the Tauri dev sandbox: start, pause, resume, stop, all three sources; display sleep/wake (screen suspends, system audio continues); a 10-minute run with two segments committed; Ask AI streams an answer. The founder runs any TCC check in a signed build.

**Depends on:** S2 (B6).

---

## S4 · Startup and graceful exit in app-core

**Goal:** `AppCore::start(host)` and `AppCore::shutdown(kind)` own §3.2's order, and both hosts call them.

**Read:** `lib.rs:535-1007`, `lib.rs:437-459` (pure exit-request action), `lib.rs:981-1004`, `app_infra::initialize`, the deferred-startup spawn, and memory `mnema-desktop-lib-suite-4-preexisting-failures`.

**Build:**
1. `start`, in this order:
   - recording settings;
   - panic hook;
   - `app_infra::initialize` (directory lock → `StartError::AlreadyRunning`, so the host shows a dialog and exits);
   - broker socket;
   - model-missing warnings;
   - notifier registration hooks (host-provided);
   - return.
2. The host opens its window, then calls `core.spawn_deferred_startup()` (thread name `mnema-deferred-startup`). It is never on the window path.
3. `shutdown`: `begin_exit` → stop capture → shut workers (timeout) → requeue orphaned jobs → unload Whisper → `Host::exit`.
4. The Tauri `lib.rs` shrinks to plugins + window + tray wiring.

**Don't:** reorder steps or move maintenance or workers onto the sync path.

**Verify:** the cargo commands above. Plus: start the dev sandbox before and after, and diff the Info log lines from start to "deferred startup done". They must be the same apart from timestamps.

**Depends on:** S2, S3.

---

## S5 · GPUI app shell

**Goal:** the real `mnema-gpui` binary opens the main window on a real AppCore. It has the titlebar, status bar, surface switch (placeholder surfaces), themes, fonts, keymap + help sheet, and the notifications popover.

**Read:**
- `docs/mockups/redesign/{SHELL.md,kit.css,tokens.css,index.html}`;
- `apps/desktop/src/routes/+layout.svelte` (tokens at :431 dark, :626 light, type scale :532-537, charts :594/:761);
- `lib/shell/{Titlebar,StatusBar,NotificationsPop,ShortcutsPop,StatusPop}.svelte` + `status-bar.ts`, `status-pop.ts`;
- `lib/theme.svelte.ts`, `lib/keyboard.ts`, `lib/keyboard-bindings.svelte.ts:14-130`;
- `apps/desktop/CONTEXT.md`.

**Build:**
1. `main.rs`:
   - `app_core::speaker_analysis_runtime::maybe_run_subprocess_helper_and_exit()` first;
   - logger (`~/Library/Logs/<id>/rust.log`, 5 MB × 5, Info globally, Debug for `APP_LOG_TARGET_PREFIXES`; reuse the constant from app-core);
   - `gpui_kit::application().with_assets(..)`, `on_open_urls` → channel;
   - `run`: fonts, theme, `AppCore::start(GpuiHost)`, main window, `spawn_deferred_startup`.
   - Dev identity `day.mnema.gpui-dev`, `MNEMA_SAVE_DIRECTORY=~/.mnema-gpui-dev` + `MNEMA_APP_CONFIG_DIR` set by `scripts/dev-gpui.sh`, which works on a **copy** of `~/.mnema-dev`.
2. `bridge/events.rs`: `GpuiHost::emit` → `async_channel` → one `cx.spawn` loop → per-event `Entity` updates.
3. `theme.rs`: every §3.7 token for dark and light, derived fills (hairline, wash, wash-strong), type roles, spacing, radii, control heights, easing and durations. Mapped onto gpui-kit `ThemeColor` (second `Theme::update` after `Theme::change`). Modes system/light/dark follow `observe_window_appearance`, with a 180 ms cross-fade. Reduce-motion observer.
4. `shell/titlebar.rs`, `shell/status_bar.rs`, `shell/surface_pill.rs`, `shell/command_field.rs`, `shell/notifications_pop.rs`, `shell/shortcuts_pop.rs`, `shell/status_pop.rs`. Sizes per §3.6.
5. `keymap.rs`:
   - actions and bindings from `keyboard-bindings.json`;
   - Alt chords match the physical key;
   - app shortcuts suppressed in inputs and buttons;
   - reserved Esc/Tab/⇧Tab/frame arrows;
   - the help sheet built live per surface.
6. `ui/clock.rs`: one `now(cx)` global, frozen in tests.
7. Feel instrumentation (feature `feel-metrics`): frame time, keystroke→paint, summon latency and startup milestones, logged to Debug target `mnema_gpui::feel`. `scripts/gpui-feel-report.sh` turns the log into the §5 budget table.
8. `tests/visual.rs` harness: `VisualTestAppContext::with_asset_source`, fixture seed via `app_core::fixtures`, `UPDATE_BASELINE=1`, the S0 comparator, PNG output for P0.
9. Delete the S0 spike code.

**Don't:** build surface contents or shared controls beyond what the shell needs (S6 does those).

**Verify:**
```
cargo test -p mnema-gpui                       # layer 2: ⌘K/⌘J/⌘1/⌘2, Tab flip, gear toggle, theme switch, tooltip 350 ms
cargo test -p mnema-gpui --test visual -- shell   # goldens: shell × {empty,populated,recording,narrow} × {light,dark}
bash scripts/parity.sh shell                    # composites + gap list (gpui-parity-checker)
bash scripts/dev-gpui.sh                        # runs on the dev-data copy; startup time vs Tauri logged
```

**Depends on:** S0, S1 (S4 for real startup; until then it calls the pieces directly).

---

## S6 · Shared controls

**Goal:** every shared control exists on gpui-kit, themed to §3.7, shown in a gallery window with goldens.

**Read:** `apps/desktop/src/lib/styles/kit.css` (`.mx-btn .mx-notice .mx-banner .mx-empty .mx-skel* .mx-toast(s) .mx-pop .mx-menu .mx-tip .mx-chip .mx-badge .mx-seg .mx-switch .mx-input .mx-progress .mx-spin .mx-keys`), `lib/components/{Segmented,Switch,Select,Combobox,Stepper,Slider,Input,DateInput,Choice,ActionSelect,FieldWarning,WindowStrip}.svelte`, `tooltip.ts`, `docs/mockups/redesign/index.html`.

**Build:** `ui/` with one file per control:
- button (all variants);
- segmented (420 ms expo thumb), switch, select, combobox, choice, stepper, slider, input, date input;
- popover (blur 22, scale .98 / −4 px, shadow), menu (gpui-kit `native_menu` for context menus);
- tooltip (350 ms dwell, instant on focus, hides on scroll);
- notice/banner, empty, static skeleton, toast (rise 8 px + drain bar), modal/dialog;
- chip, badge, progress, spinner, kbd keys;
- `WindowStrip` (38 px, grey dots → red/amber/green on hover, drag region).

Focus ring is 3 px accent-glow; disabled 0.4; busy 0.6. Plus a `gallery` bin.

**Don't:** fork gpui-kit components. Wrap and theme them.

**Verify:** `cargo test -p mnema-gpui ui::` (behaviour + timing: tooltip 349/350 ms, toast drain). `cargo test -p mnema-gpui --test visual -- gallery` (every control × state × theme). The gallery parity against `index.html`.

**Depends on:** S5.

---

## S7 · Native layer

**Goal:** everything in §3.5 that GPUI doesn't give us works in a bundled dev `.app`.

**Read:**
- `status_bar.rs` (ids :11-26, pure `build_menu_model` :160, `apply_read_only_status` :262, dialogs);
- `keyboard_bindings.rs:1015-1080`;
- `lib.rs:257-378, 573-575, 893-921`;
- `windows.rs:677-746, 1048-1092`;
- `windows/macos.rs`, `clock_format.rs:35-53`;
- `not_recording_reminder/macos_notification.rs`;
- `app_updates.rs:584-596` (parentless rfd);
- ropy tray/hotkey.

**Build:** `native/` modules:
- `tray.rs`: full menu from `build_menu_model` (moved to app-core in S2/S3), same `tray_*` ids, confirm/partial-failure dialogs, update item. tray-icon 0.26.1, created on the main thread inside `run`.
- `hotkeys.rs`: global-hotkey 0.9.0, defaults ⌘⌥R/P/M/Space, pressed only, gated on onboarding complete + enabled, unregister-all/re-register, failure → in-app warning `global-shortcuts-registration-failed`.
- `deep_links.rs`: the pure parsers from `lib.rs:270-339` (moved), `mnema-gpui-dev` scheme.
- Single instance: the `.app-infra.lock` + LaunchServices, verified.
- `dock.rs`: `set_activation_policy`; Regular while any non-Quick-Recall window is visible; `on_reopen` with nothing visible → main.
- `quit.rs`: the objc2 terminate swizzle from S0 → `AppCore::shutdown`. Covers ⌘Q, Dock quit, tray quit and logout.
- `notification.rs`: the not-recording reminder. Reuse `macos_notification.rs` (category `day.mnema.not-recording`, Start / Snooze for today). Don't also register GPUI's handler.
- `window_chrome.rs`: sharingType none / read-only (developer options) before first show, continuous corners, panel details.
- `dialogs.rs`: GPUI `prompt`/`prompt_for_paths` with a parent window, rfd with a parent where they don't fit; opener; clipboard text + image; save to Downloads.
- `sys.rs`: `on_system_sleep/wake`, `observe_displays`, lock state, `NSDateFormatter "j"` 24 h.

**Don't:** use muda for the app menu (GPUI owns `set_menus`). Show a parentless rfd dialog.

**Verify:**
- `cargo test -p mnema-gpui native::` (menu model, deep-link parsing, hotkey re-register logic).
- `scripts/bundle-gpui-app.sh --dev` (an early cut of S16: unsigned/ad-hoc dev bundle, `day.mnema.gpui-dev`).
- Then a checklist, driven by the orchestrator with `screencapture -l` + AX where possible: tray before and after onboarding, each hotkey, a URL while running and while not, a Finder double-launch, the Dock icon rules, ⌘Q waits for capture stop, the reminder with both actions, sleep/wake.
- TCC items go on the founder's signed-build list.

**Depends on:** S4, S5.

---

## S8 · Timeline

**Goal:** the Timeline surface with the stage, overlays, rail, jumper, scrub engine and frame actions, at frame-time budget.

**Read:**
- `routes/+page.svelte`: 7938 lines. Constants at `:166-184` (paging), `:195-211` (scrub), stale CSS at `:7016-7018`.
- `lib/timeline/*`: TimelineJumper, JumperCalendar, JumperTimeList, RecallMatchBoxes, RecallLandingChip, ActionSelect, `jumper-cache-core.ts`.
- `lib/frame-preview.ts:29-49`.
- `docs/mockups/redesign/timeline.html`.

**Build:** `timeline/`, ≤800 lines per file, about 10 files:
- `view.rs`
- `top_bar.rs`
- `stage.rs`: radius 12, contain, stale 0.55 opacity / 0.6 saturation, dev overlay.
- `ocr_overlay.rs`, `match_overlay.rs`: solid = keyword, dashed = semantic.
- `rail.rs`: 36 px, masked edges, 8 px slots, fixed caret, 1×8/16 ticks, readout 9 px/190 ms.
- `bands.rs`: 18 px icons, match mark, audio lane 11 px, hover tooltip.
- `jumper.rs`
- `scrub_engine.rs`:
  - throttle 160/240 ms; display ≤80 ms; settle 180 ms; warm settle 1200 ms; exact frame 500 ms;
  - prefetch radius 4→3→2→1 at 2/4/8 px/ms; max uncached 9/7/5/3.
- `paging.rs`: 200 / prefetch 120 / cap 5000 / head poll 1500 ms / poll page 50 / budget 20 / viewport buffer 80.
- `image_cache.rs`: the `ImageCache` impl, 192 entries, failures cached 5 s, `drop_image` on evict.
- `frame_actions.rs`: Copy image C, Download D, Copy text, Open host.

Input: wheel anywhere scrubs; ←/→ ±1, ⇧ ±10.

**Don't:** use `asset://`, blob or webview tricks. Decode images on the main thread.

**Verify:**
- Timing tests at the boundaries of each constant (e.g. no display at 79 ms, display at 80 ms).
- Goldens timeline × {empty, populated, stale, matches, narrow} × theme.
- `parity.sh timeline`.
- `feel-metrics` fast-scrub run: p95 frame time ≤8 ms ProMotion / ≤16 ms otherwise (`gpui-feel-checker`).

**Depends on:** S6. **Slot:** wave A.

---

## S9 · AudioDrawer

**Goal:** the audio drawer with transcript reader (karaoke + follow), speaker-coloured waveform, speaker repair panel and native playback.

**Read:** `lib/timeline/audio/{AudioDrawer,DrawerHeader,DrawerStatePanels,DrawerTransport,WaveformScrubber,SpeakerRepairPanel,TranscriptReader,SpeakerMark,DrawerNotice}.svelte`, `audio-drawer-view.ts`, `audio-transport.svelte.ts`, `waveform-peaks.svelte.ts`. Commands: `get_audio_segment_media`, `get_audio_segment_waveform_peaks`, `list_audio_segments`, `list_speaker_turns`.

**Build:**
- `timeline/audio/*`: peek 50 vh / expanded; Space play; ←/→ ±5 s; ⇧ ±30 s.
- Native playback in `native/audio_player.rs`: AVAudioPlayer via objc2 on the segment's file path (no base64). Position is polled at display rate only while playing.

**Verify:** key + timing tests; goldens drawer × {peek, expanded, repair, no-transcript} × theme; parity; playback by hand in `dev-gpui.sh`.

**Depends on:** S8.

---

## S10 · Quick Recall panel

**Goal:** the summon-anywhere panel in ≤50 ms warm, with every QR constant.

**Read:**
- `routes/quick-recall/*`: BarShell, Field, FilterPicker, ResultsList, RecentRows, EmptyStates, AskPane, QuickLook(+Side), BestMoment, SearchResultCard, AnswerProse;
- stores: `searchStore.svelte.ts:47-49`, detailStore, filterSurfaces, states;
- `AskPane.svelte:83`;
- `windows.rs:43-66`, plus the quick-recall window code in `windows.rs`;
- `lib/markdown.ts:257-262`;
- `docs/mockups/redesign/recall.html`.

**Build:** `quick_recall/`:
- `WindowKind::PopUp` panel, reused and never destroyed. 800×60 idle → ≤640 body; top edge 15% down the work area, centred, grows down; 16 px corners.
- Dismiss on blur, except within 300 ms of summon or after a one-shot suppress. State resets 5 s after hide.
- Field: mode chip, filter pills, phrase tinting, ghost completion (→).
- Results + BestMoment, with focus kept in the field.
- QuickLook: 476 px, 7 frames 15 s apart.
- AskPane: streaming via `CoreEvent::AskAiUpdate`, numbered citations, "Continue in Chat ↵".
- RecentRows on ↓ (max 8); FilterPicker.
- Debounces: search 250 ms, commit 700 ms, auto-ask 750 ms.

**Verify:**
- `advance_clock` tests at 249/250, 699/700, 749/750, the 300 ms blur grace and the 5 s reset.
- Goldens × {idle, results, quicklook, ask-streaming, ask-done, recent, filter} × theme.
- Parity.
- Summon latency from `feel-metrics` in the dev `.app`.

**Depends on:** S6, S7. **Slot:** wave B.

---

## S11 · Chat

**Goal:** the Chat surface with streaming answers at frame budget.

**Read:** `routes/chat/+page.svelte`, `lib/chat/{ChatList,Chat,ChatTurn,ChatComposer,ChatEmpty,ChatSources,ChatErrorTurn,ModelPicker,ModelPickerMenu,ScopePanel}.svelte`, `FrameDetailModal.svelte`, `AnswerProse.svelte` (caret :389-395, Copied :73-78), `lib/markdown.ts:257-262`, `docs/agents/ask-ai-streaming.md`, `docs/mockups/redesign/{chat,model-picker}.html`.

**Build:** `chat/`:
- Layout: list 276 px (240 below 1180 px); thread max 760.
- Question bubble radii lg lg xs lg.
- Steps disclosure with live dot; rows rise in. The thought box keeps its tail in view.
- Answer blocks prose (`TextView::markdown`; dangling `**` closed while streaming; highlight only after finish; static 7×15 caret), bars, dossier, timeline.
- Source chips → FrameDetailModal. Copy ("Copied" 1.2 s), regenerate, model, context bar.
- Composer: Enter / ⇧Enter; Send↔Stop. ModelPicker is shared with Settings. ScopePanel. Greeting chips never auto-send.
- Toasts: deleted 4 s; pin failure + Retry 5 s.

**Verify:**
- Tests: the streaming version is monotonic, Copied reverts at 1200 ms, toast timings.
- Goldens × {empty, list, mid-stream, done, error} × theme.
- Parity.
- Streaming frame time p95.

**Depends on:** S6. **Slot:** wave A.

---

## S12 · Insights

**Goal:** the Insights surface: Overview, Journal (+ receipt viewer), Subjects (+ drawer, Undo), Context and charts.

**Read:** `routes/insights/+page.svelte`. `lib/insights/`:
- Overview: Overview, OverviewLede, OverviewStates, OverviewExhibits, OverviewFeed, Category/App/FocusDetailModal.
- Journal: DayTimeline, DaySpine, JournalRiver, ReceiptViewer.
- Subjects: Subjects, SubjectsLede, SubjectsList, ConvictionMap, SubjectDetail, SubjectHeader, SubjectChart, ConclusionTiers, EvidenceDrawer, Pager.
- Context: Context, ContextSide.
- `charts/*`.
- `dismissUndo.svelte.ts:5`.

Also `docs/mockups/redesign/insights.html` and memory `mnema-insights-empty-states-direction-a`.

**Build:**
- `insights/{overview,journal,subjects,context,charts}/`.
- Sticky blurred 56 px header with tabs + counts + 2 px ink underline. Engine notices.
- Journal: `[`/`]` step days. The receipt timelapse at 8×/16× runs on GPUI animation frames.
- Subjects: Pin/Dismiss with a 5000 ms delayed-commit Undo ("Dismissed · Undo" + toast drain; navigating away commits immediately). ⌘J Ask.
- Charts: MiniBars, StackedBar, Timeline, Sparkline, Heatmap, ConfidenceBar, as GPUI paths or quads (gpui-kit charts only where they match).

**Verify:**
- Undo tests at 4999/5000 ms, plus navigate-away.
- Goldens for every tab × {empty, populated} × theme.
- Parity.

**Depends on:** S6. **Slot:** wave B.

---

## S13 · Settings

**Goal:** the full Settings surface with all 20 sections and the autosave engine.

**Read:**
- `routes/settings/+page.svelte`, `lib/settings/{SettingsRail,SettingsBar,groups.ts}`, `lib/settings/panels/*` (section list in appendix D);
- `lib/settings/ui/*`;
- `settings/state/autosave-core.ts:78-85`;
- `docs/mockups/redesign/settings.html`;
- memories `mnema-settings-control-convention`, `mnema-settings-init-effect-untrack`.

**Build:**
- `settings/`: full window with no titlebar or status bar.
- Rail 252 px: 52 px drag strip, "← Back to app" (Esc), `/` search over keywords, 5 groups with tinted tiles, footer with REC + version.
- 52 px bar: breadcrumb + save status (saved / saving / resolve issues / couldn't save + Retry).
- One scroll column with scroll-spy.
- `settings/autosave.rs`: recording 450 ms, source switches 0 ms, mic 250 ms; blocked while a privacy command runs.
- One file per section (MCP split across several). ReloadButton spins for at least 500 ms.

**Verify:**
- Autosave tests at 449/450, 0 and 249/250, plus the privacy block.
- Goldens for every section × theme.
- Parity.

**Depends on:** S6. **Slot:** wave A.

---

## S14 · Onboarding

**Goal:** the 8-step onboarding window with the two hard gates, downloads, voice and finale.

**Read:**
- `routes/onboarding/*`, `onboarding-flow.svelte.ts:88`;
- `screens/{Welcome,Permissions,CaptureStorage,YourSettings,ChangeSettings,Setup,Voice,Finale}.svelte`;
- `lib/onboarding/*`;
- `docs/mockups/redesign/onboarding.html`;
- **all memories `mnema-onboarding-*` first.**

**Build:**
- `onboarding/`: undecorated, transparent window, 1120×800 / 920×620, 12 px continuous corners. Closing before done = exit.
- 52 px strip: drawn traffic lights, step name, tick ruler, "N / 8".
- Steps per §3.6. Permissions and Capture & Storage are the only hard gates. Setup never blocks. Voice is 15 s. Finale starts capture and shows the first real frame + OCR.

**Verify:** step-flow tests, goldens per step × theme, parity (with `?step=`). **Founder: signed-build TCC pass.**

**Depends on:** S6, S7.

---

## S15 · Small windows

**Goal:** the access request, update, permission helper and debug (+ mini) windows.

**Read:** `routes/{access/request,update,permission-helper,debug}/*`, `lib/components/WindowStrip.svelte`, `lib/debug/*`, `windows/permission_helper.rs`, `docs/mockups/redesign/{access,update,developer,status}.html`, memory `mnema-debug-redesign-plan`.

**Build:** `small/`:
- **Access request** (520×600 / 520×200, fits content): focus starts on Deny, Enter unbound, Esc and close both deny ("closed"), then a receipt view.
- **Update** (480×520 / 420×240): identity band, notes band, controls.
- **Permission helper:** non-activating panel 460×62, 8 px corners, bottom-centre 28 pt above the work area, `orderFrontRegardless`, polls the grant every 1 s, closes itself.
- **Debug** (520×640 / 440×480, gated on developer options): verdict + 5 tabs. Mini mode 300×120, always on top.

**Verify:** keyboard tests (Deny focus, Esc), goldens per window × state × theme, parity.

**Depends on:** S6, S7. **Slot:** wave B.

---

## S16 · Packaging + updater

**Goal:** a founder-signed GPUI `Mnema.app` that installs, updates from a test feed, refuses a tampered or wrong-team update, and has a working CLI.

**Read:**
- `scripts/entitle-macos-app.sh`, `scripts/prepare-mnema-cli-sidecar.sh`, `scripts/build-macos-local-sign.sh` (read only; never run it);
- `tauri.conf.json` (Info.plist strings, URL schemes, entitlements);
- `apps/desktop/src-tauri/{Entitlements*.plist,Info.mnema-cli.plist}`;
- `app_updates.rs` (feed :20-21, `AppUpdateState` :47, timer :945-949);
- `.github/workflows/{macos-release,macos-release-promote}.yml`;
- §9.2.

**Build:**
1. `scripts/bundle-gpui-app.sh [--dev|--release]`:
   - `cargo build --release -p mnema-gpui` with `OPENBLAS_DYNAMIC_ARCH=1` for release;
   - assemble `Mnema.app`: Info.plist (usage strings, `CFBundleURLTypes`, LSMinimumSystemVersion 15, arm64), icons, `THIRD_PARTY_LICENSES.md`, `mnema-cli` at `Contents/MacOS/mnema-cli`;
   - then `entitle-macos-app.sh`, which makes `Helpers/mnema-cli.app` + the symlink, embeds the profile and signs.
2. Notarize + staple in the same script (`--notarize`).
3. `crates/app-core/src/updates/`:
   - move the pure parts of `app_updates.rs`: settings, gate, state machine, feed parse, 2 h timer;
   - minisign verify (pubkey `F6BBB53B2FAA615D`);
   - download → extract to temp;
   - **install gate** via `security-framework`/objc2: `SecStaticCodeCheckValidity` (all-arch | nested | strict); team == `SecCodeCopySelf` team; bundle id == own; `Helpers/mnema-cli.app` same team; no own team → install disabled + Warn;
   - replace the bundle atomically, relaunch;
   - a refusal logs Error with a reason code only, keeps the old app, and shows `Failed` "This update isn't signed by Mnema" + Retry.
4. Gate tests: a valid fixture passes; one byte changed fails; an ad-hoc re-sign fails on team; a different bundle id fails. Fixtures are built at test time with `codesign`; identity-needing tests are `#[ignore]` and documented.
5. A release workflow job for the GPUI artifact. `macos-release-promote.yml` is changed only if the artifact name must change.

**Don't:** run `build-macos-local-sign.sh`. Hard-code the team id. Shell out to `codesign` in production code.

**Verify:** `cargo test -p app-core updates::` (+ the `--ignored` gate tests the founder runs with their identity), `bash scripts/bundle-gpui-app.sh --dev` builds, then the founder signs, installs, updates from a test feed and checks both refusals and `mnema-cli status`.

**Depends on:** S7.

---

## S17 · Cut-over (founder go only)

**Goal:** GPUI is the only desktop app.

**Build:**
- Identity → `day.mnema`, keeping the exact config/log/data layout.
- `MNEMA_APP_IDENTIFIER` source → a constant in a tiny shared `build.rs` include (no more `tauri.conf.json`).
- Delete `apps/desktop` (Tauri + Svelte + TS mirrors) and the `TauriHost`.
- Update `AGENTS.md`, `SUPPORTS.md` ("GPUI app: macOS only"), `CONTEXT-MAP.md`, `turbo.json`, CI.
- Delete the stale `apps/desktop/src-tauri/Cargo.lock`.

**Verify:** CI green. The W3 parity sweep has no open high-severity gaps. The founder's signed build passes the §3.5 checklist.

**Depends on:** everything + founder go.

---

## Workflows (load `workflow-authoring` before each; ≤10 agents)

- **W1 extract-core:** per batch, parallel `core-extractor`s (≤3 build slots) → patches → orchestrator applies + verifies.
- **W2 build-surface:** builder → parity-checker → feel-checker → fix loop (max 3 rounds) → hand back.
- **W3 parity-sweep:** a parity-checker per surface → one merged gap table.
- **W4 review:** `reviewer-{logic,concurrency,data-integrity,behavioral,rules,performance}` over a slice diff + an adversarial verifier per finding.

A push notification goes out when a workflow finishes while you're away, or when I'm blocked on a decision.

## Verification of the whole effort (§10)

- Every surface and window has goldens in both themes.
- No open high-severity parity gaps.
- Every §5 constant is asserted in a test.
- Every §5 budget is measured by `gpui-feel-report.sh` and met or waived.
- Every §3.5 behaviour is checked by the founder in a signed build.
- No file over 800 lines (hook + `find … | xargs wc -l` check).

---

## Appendix A · CoreEvent variants (32 event names, today's emit sites)

- **Streams:**
  - `ask_ai_update` (`ask_ai.rs:1600`, 1917, 2005, 2040, 2615);
  - the four model download progress events (`*_models.rs`, semantic also `semantic_search_worker.rs:1356`);
  - `native_capture_session_changed` (`native_capture.rs:842`);
  - `app_update_status_changed` (`app_updates.rs:330`).
- **Invalidations:**
  - `conversation_changed`, `user_context_changed`, `audio_segments_changed`;
  - `timeline_data_changed` (`app_infra.rs:2886, 3555, 4153`);
  - `scrub_preview_cache_changed` (`frame_preview.rs:3075`);
  - `recording_settings_changed`, `recording_settings_domain_changed`;
  - `app_notifications_changed`, `microphone_controller_changed`, `microphone_auto_disconnect_transition_failed`, `open_app_notifications`;
  - `license_status`, `license_deep_link(_done)`;
  - `keyboard_bindings_settings_changed`, `mcp_authorization_changed`, `chatgpt_login_update`;
  - `system_did_wake`, `screens_did_sleep/wake`;
  - `broker_open_capture_result`, `insights_open_conversation`.
- **Host-only (window-targeted):** `open_settings_tab`, `quick_recall_dismissed`, `quick_recall_query`.
- **Drain queues deleted in GPUI** (the payload is passed when the view opens): broker_open_capture_result, insights_open_conversation, license_deep_link, open_settings_tab, quick_recall_query.

## Appendix B · Extraction dependency edges that set the batch order

- **Cycles that move together:** semantic_search_models ↔ worker, ai_runtime ↔ user_context, conversation ↔ ask_ai.
- **Couplings that become Host calls or CoreEvents:**
  - licensing → app_updates/status_bar;
  - native_capture → status_bar ×8, windows, licensing;
  - broker_authorization_channel → windows;
  - keyboard_bindings → windows + capture start/stop (the handler stays host-side).
- **Capture blockers:** app_infra (B6) must be in app-core before capture moves. ask_ai moves with capture because of start/stop/mcp.

## Appendix C · `segments.rs` AppHandle functions (36: 12 `&`, 22 `Option<&>`, 2 owned)

- **State:** :96, :2825, :3542, :4385, :4520.
- **State + paths:** :2444, :2527, :2619.
- **State + emit:** :2687.
- **Spawn:** :2235.
- **State + emit + tray:** `spawn_segment_loop` :4705.
- **Dialog:** `graceful_stop_for_license_block` :1517.
- **In-app notifications:** :1201, :1482, :1855, :2006.
- **The other 20 only forward the handle.**

## Appendix D · Settings sections

- **General:** Appearance, Startup, Shortcuts.
- **Capture:** Sources, Video, Microphone, Privacy.
- **Intelligence:** Providers, Ask AI, User Context, OCR, Transcription, Speakers (+voice enrollment), Semantic Search, MCP (8 files).
- **Data:** Storage, CLI Access, Delete Recent Capture.
- **About:** License & Trial, Updates, Developer.
