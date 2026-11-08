# Prompt: build the Mnema desktop app in GPUI

> Paste everything below the line into a fresh Claude Code session started in the
> `new-gpui-desktop-app` worktree. You (the session) are the **orchestrator**: you plan,
> fan work out to subagents and workflows, review, and talk to the founder. You do not
> write the whole app yourself.
>
> Facts below were gathered on 2026-11-08 from `main` @ 42477c48. Treat file:line
> references as pointers; re-check them before relying on them.

---

## 0. Mission

Build a native **GPUI** desktop app for Mnema that matches the current Tauri + Svelte app
in **look, feel and behaviour**. It must reuse the existing Rust backend rather than
rewriting it. The Tauri app stays shipping until the GPUI app reaches parity; then Tauri
is deleted.

"Matches" is measured, not eyeballed. Every surface is screenshot-compared against both
the live Tauri app and the redesign mockups, in light and dark. The "feel" constants in
§5 are asserted by tests.

I opt into multi-agent orchestration for this project. Use the **Workflow** tool,
**subagents** (Agent tool), **worktree isolation**, **skills**, **hooks**, **Monitor**,
**plan mode** and **push notifications** as described in §7. Keep each workflow under ~10
agents unless I raise it.

## 1. Decisions already made (do not re-litigate)

| # | Decision | Why |
|---|---|---|
| D1 | **Side by side, not a rewrite-in-place.** New crate `apps/desktop-gpui`. Shared logic moves from `apps/desktop/src-tauri/src` into a new Tauri-free crate `crates/app-core`. Both apps compile against it until cut-over. | Lets us run Tauri and GPUI on the same data and diff them, which is the only honest "feel" test. *(Founder confirmed 2026-11-08.)* |
| D2 | **Stack:** `gpui-kit = "=0.9.0"`, which pins `gpui-pre = "=0.3.11"` (weekly snapshot of Zed @ `f816e1f`). Add `gpui_platform = { package = "gpui-pre-platform", version = "=0.3.11", features = ["font-kit"] }` if you touch GPUI directly. Exact `=` pins only, upgraded on purpose. | crates.io `gpui` is stuck at 0.2.2 (no a11y, no activation policy, no `gpui_platform`). gpui-kit gives Input, Select, VirtualList, Table, Markdown TextView, Popover, Dialog, Sheet, Tooltip, Toast, Charts, Calendar, Slider, Sidebar, Lucide icons. **Without `font-kit` no text renders.** |
| D3 | **Visual source of truth:** `docs/mockups/redesign/*.html` + `kit.css` + `tokens.css` + `SHELL.md` + `IMPLEMENTATION.md`, cross-checked against the running Tauri app. Where the two disagree, the running app wins and the gap is listed for the founder. | The current app was built from those mockups, and the mockups contradict their own prose in places. Fact-check the drawings. |
| D4 | **No webview.** Markdown renders natively (gpui-kit `TextView::markdown`). There is no `asset://`, `convertFileSrc`, blob-URL trick, or WKWebView cache purge. Images load from file paths with our own LRU. | The point of GPUI. |
| D5 | **Dev identity** `day.mnema.gpui-dev`, data dir `~/.mnema-gpui-dev`. At cut-over the GPUI app takes `day.mnema` and must keep the exact config/log/data dir layout, `Contents/MacOS/mnema-cli` symlink and `Helpers/mnema-cli.app`. The CLI, broker socket and updater depend on those paths. | `.app-infra.lock` is single-writer, so both apps can never share one live data dir. Compare against **copies** of the same dev data. |

## 2. Read first (orchestrator reads these, briefs agents to read their part)

- `AGENTS.md` (= `CLAUDE.md`): **all invariants still apply**, including logging levels,
  no secrets/content in logs, migrations never deleted, no legacy shims, `MNEMA_*` must be
  added to `turbo.json passThroughEnv`, the 800-line file cap, and the `mnema-cli` sidecar
  being needed for any desktop cargo build.
- `CONTEXT-MAP.md` → `apps/desktop/CONTEXT.md` (UI vocabulary), `apps/desktop/src-tauri/CONTEXT.md`.
- `docs/agents/capture-pipeline.md`, `docs/agents/ask-ai-streaming.md`.
- `docs/mockups/redesign/{SHELL.md,IMPLEMENTATION.md,kit.css,tokens.css}` and the page per surface.
- `SUPPORTS.md` (update it when platform behaviour changes).
- GPUI references, cloned on demand into a scratch dir, not the repo:
  - `longbridge/gpui-kit`: `website/docs/*.md`, `crates/component/src`, `docs/ACCESSIBILITY-UI-TESTING.md`.
  - `zed-industries/zed`: `crates/gpui/examples`, `crates/gpui_tokio` (~100 lines, copy it), `crates/zed/src/visual_test_runner.rs`.
  - `StudentWeis/ropy`: `src/gui/tray.rs`, `src/gui/hotkey.rs`. A real gpui-kit 0.9 app with tray + global hotkey; the closest pattern to ours.

## 3. What exists today (the thing you are reproducing)

### 3.1 Code shape
- **Shared crates already free of Tauri:** ~135k lines in `crates/*`, of which `app-infra` is 84.5k (SQLite/sqlx, migrations, jobs, OCR pipeline, search, user context, vault, broker). Do not touch their design.
- **`apps/desktop/src-tauri/src`:** ~104k lines, of which ~44k are tests. Of the ~59k production lines, ~85–90% uses Tauri only as:
  1. a typed state lookup (`app.state::<T>()`, ~185 calls; 190 `tauri::State` params);
  2. dirs (`path().app_data_dir()/app_config_dir()/AppCache`);
  3. an event bus (~65 `emit` sites);
  4. a few dialog/opener calls;
  5. `tauri::async_runtime`, which is plain tokio.
- **Tauri-only by nature (~6–7k lines):** `lib.rs`, `windows*`, tray rendering in `status_bar.rs`, `app_updates.rs` (tauri updater), `webview_cache.rs`, `windows/parked_view.rs`, global-shortcut registration, asset-scope grants.
- **Hardest extraction:** `native_capture/lifecycle.rs` ("the owning seam") still takes `AppHandle` and threads it into ~36 functions in `native_capture/segments.rs`. These are used for state lookup, disk-space paths, a license-block dialog and emits, and `spawn_segment_loop(app_handle)`. Some `Option<&AppHandle>` / `*_in_dir` variants already exist, so follow that split.
- **Wire types:** `crates/capture-types` (session, recording settings, conversation/`TurnUpdate`/`AnswerBlock`, user_context, licensing, microphone, usage_charts, logs). Many DTOs live in src-tauri modules instead: `FramePreviewDto`, `ProcessingJobDto`, `AudioSegmentMediaDto`, the model-download DTOs, `AppUpdateStatus`, `KeyboardBindingsSettings`, `BrokerOpenCaptureResultPayload`. GPUI uses them as Rust structs directly, so the hand-mirrored TS types disappear.
- **Commands:** 248 registered (`lib.rs:641-890`). By domain: capture 43, timeline/frames/search/jobs/speakers 45 (`app_infra.rs`), frame previews 6, Quick Recall 5, Ask AI + MCP 12, conversations 8, user context 21, AI providers/ChatGPT 13, Deepgram 5, model downloads 20, licensing 6, CLI access/broker 10, privacy 5, debug/logs 10, updates 5, keyboard 2, windows/onboarding 15, handoff queues 6, voice 2, usage charts 3, storage/misc 9. **In GPUI these are direct method calls on `app-core`, not IPC.**

### 3.2 Startup and shutdown (keep the order)
`lib.rs:535-1007`:
1. Register ~35 `Default` states.
2. Plugins: single-instance → open main; log; deep-link; updater; global-shortcut.
3. Setup:
   - deep links;
   - macOS `terminate:` swizzle → graceful exit;
   - recording settings from disk;
   - updates/prompts/tray/keyboard init;
   - panic hook;
   - `app_infra::initialize` (directory lock; "already running" dialog + exit);
   - broker Unix socket;
   - model-missing warnings;
   - native notifiers (mic device, wake, display reconfig, metadata);
   - open startup window;
   - spawn **`mnema-deferred-startup`** thread;
   - tray refresh.
4. The deferred thread (vault unlock, maintenance, repair passes, workers, CRL/receipt timers, not-recording reminder, auto-start capture, update timer) **never moves onto the window-open path.**
5. Graceful exit: `begin_exit` → stop capture → shut workers (timeout) → requeue orphaned jobs → unload Whisper → `_exit(0)` / restart.

### 3.3 Events → become `app-core` events
Streams:
- `ask_ai_update` (versioned `{conversationId, version, turnIndex, update: TurnUpdate}`, monotonic per live turn);
- the 4 `*_model_download_progress` events;
- `native_capture_session_changed`;
- `app_update_status_changed`.

Invalidations:
- `conversation_changed`, `user_context_changed`, `audio_segments_changed`;
- `timeline_data_changed{reason, deletedBefore, startedAt, endedAt, deletedFrameIds, deletedAudioSegmentIds}`;
- `scrub_preview_cache_changed{rendition,start,end}`;
- `recording_settings_changed`, `recording_settings_domain_changed`;
- `app_notifications_changed`, `microphone_controller_changed`;
- `license_status`, `license_deep_link(_done)`;
- `keyboard_bindings_settings_changed`, `mcp_authorization_changed`, `chatgpt_login_update`;
- `system_did_wake`, `screens_did_sleep/wake`;
- `broker_open_capture_result`, `insights_open_conversation`;
- `open_settings_tab`, `quick_recall_dismissed/query`.

The "event + drain-on-mount queue" pairs only exist because a new webview might not be listening yet. In GPUI, pass the payload when opening the view and delete the queue.

### 3.4 Windows

| Window | Size / min | Chrome & behaviour |
|---|---|---|
| main | 800×600 / 640×480 | Native titlebar overlay, hidden title, 78px traffic-light inset. Close = hide; destroy = exit. Hosts Timeline / Insights / Chat / Settings. |
| onboarding | 1120×800 / 920×620 | Undecorated, transparent, shadow, 12px continuous corners. Closing before done = exit. |
| cli-access-request | 520×600 / 520×200 | Same chrome; height fits content. Closing = deny ("closed"). |
| debug | 520×640 / 440×480 | Same chrome; developer-options gated; mini mode 300×120 always-on-top. |
| update | 480×520 / 420×240 | Same chrome; fits content. |
| quick-recall | 800×60 idle → ≤640 | **NSPanel**: non-activating, floating level, all Spaces, transient, full-screen auxiliary, not hide-on-deactivate, can become key, `cancelOperation:` no-op, accepts first mouse. 16px corners. Top edge 15% down the work area, centred, grows downward. Dismiss on blur except within 300ms of summon or after a one-shot suppress. Reused, not destroyed. State resets 5s after hide. |
| permission-helper | 460×62 | Non-activating panel, 8px corners, bottom-centre 28pt above work-area edge, `orderFrontRegardless` (never steals focus from System Settings), polls the grant every 1s, closes itself. |

Every window:
- **`NSWindow.sharingType = none`** (`read-only` when developer options are on) **before the first show**, so Mnema never records itself.
- Dock icon shown while any non-Quick-Recall window is visible.
- `Reopen` with nothing visible → open main.

GPUI mapping:
- `WindowKind::PopUp` is already a non-activating `NSPanel` at popup level on all Spaces. Use it for Quick Recall and the helper, then adjust level/collection behaviour via `HasWindowHandle` + objc2.
- `TitlebarOptions{appears_transparent, traffic_light_position}` + `app_owns_titlebar_drag` + `start_window_move` for main.
- `WindowBackgroundAppearance::Transparent`/`Blurred` for panels.
- `set_activation_policy` for the Dock.

### 3.5 OS integration GPUI does not give you (each is its own slice)
- **Tray** (`status_bar.rs`). Template icon only; the tooltip changes with state. **Before onboarding:** Complete Setup / Settings / Quit. **After:**
  - disabled status header (low disk / read-only);
  - Start/Stop Recording (disabled + "Starting…/Stopping…" while busy);
  - Pause/Resume;
  - Sources ▸ Screen / Microphone / System Audio checkboxes (disabled while recording; last source can't be unchecked);
  - Exclude Current App From Now On… (confirm; refuses `day.mnema`);
  - Delete Recent Capture ▸ 1 / 5 / 15 min (warning confirm + partial-failure dialogs);
  - optional update item (Install / Retry / Updating… / Restart to Finish);
  - Open Mnema, Settings, Quit Mnema.

  Keep the `tray_*` ids. The pure `build_menu_model` is reusable. Use `tray-icon 0.26.1` + `muda 0.21.2` (same objc2 0.6 as gpui-pre-macos). Create it on the main thread inside `run`. Pattern: std thread on `MenuEvent::receiver()` → `async_channel` → `cx.spawn` → `cx.update`. Never use muda for the app menu (GPUI owns `set_menus`).
- **Global shortcuts.** Defaults: ⌘⌥R record, ⌘⌥P pause, ⌘⌥M show/hide main, ⌘⌥Space Quick Recall. Pressed-only; gated on onboarding complete + enabled; unregister-all/re-register; a failure raises the in-app warning `global-shortcuts-registration-failed`; persisted in `keyboard-bindings.json`. Use `global-hotkey 0.9.0` with the same channel pattern.
- **Deep links** `mnema://` (`mnema-dev` in dev):
  - `license/activate?key=`, `license/claim?checkout_id=`, `license/renewed`;
  - `oauth/callback` (MCP);
  - `open/<id>` / `broker/open/<id>` (signed opaque id).

  Use `Application::on_open_urls` registered before `run`, plus `CFBundleURLTypes` in Info.plist.
- **Single instance:** the existing `.app-infra.lock` + LaunchServices (a bundled app isn't relaunched; URLs route to the running one). Verify a Finder double-launch and a URL open while running.
- **Quit:** route ⌘Q / Dock quit / tray quit / `terminate:` into the graceful exit. Check GPUI's quit hooks first; port the swizzle only if they can't block quit.
- **OS notification:** not-recording reminder, raw `UNUserNotificationCenter` (category `day.mnema.not-recording`, actions Start / Snooze for today). Reusable as-is. GPUI also has `on_system_notification_response`; pick one.
- **Dialogs** (Rust message/confirm + folder picker), **opener** (`x-apple.systempreferences:` panes, URLs, log file), **clipboard** (text + image), **save to Downloads**. Use GPUI `prompt` / `prompt_for_paths` / clipboard where they fit; otherwise `rfd` **with a parent window**. A parentless rfd dialog becomes `CFUserNotificationDisplayAlert` (see `app_updates.rs:586-596`).
- **Logging:** `tauri_plugin_log` → plain `log` + a file logger. Same file `~/Library/Logs/<id>/rust.log`, 5MB × 5, Info globally, Debug for `APP_LOG_TARGET_PREFIXES`. `debug_status::tail_app_log` must keep working.
- **Updater:** today it's minisign over `release.mnema.day/{stable,preview}/latest.json` with pubkey id `F6BBB53B2FAA615D`, a license update-window gate, checks every 2h, and a state machine Idle → … → RestartRequired. GPUI needs its own: same feed and format, signature verify, replace the bundle, relaunch (ruled in §9.2).
- **Native bits to port 1:1** (from `windows/macos.rs`, `windows.rs`): sharingType, continuous-corner content mask, panel reclass details, `NSDateFormatter "j"` for 24h, `NSWorkspace` icons + wake/sleep observers, `CGDisplayRegisterReconfigurationCallback`, lock state. GPUI has `on_system_sleep/wake` and `observe_displays`; prefer those where equivalent.
- **Packaging:** Info.plist usage strings (`NSMicrophoneUsageDescription`, `NSAudioCaptureUsageDescription`, `NSSpeechRecognitionUsageDescription`, `NSLocalNetworkUsageDescription`), `Entitlements.plist` (audio-input + app group `RJYMY4RR97.day.mnema.capture-index`), provisioning profile, `scripts/entitle-macos-app.sh` chain, notarize + staple, macOS 15+, arm64. `build.rs` force-static gfortran/quadmath is required. Speaker-analysis helper re-exec in `main.rs` must survive. Tauri bundling was done by `tauri build` + tauri-action; GPUI uses a repo bundle script (ruled in §9.3).

### 3.6 Surfaces

Sizes are non-test Svelte/TS lines; they tell you how to slice.

- **Shell (~5k).**
  - **Titlebar:** 38px; `titlebar-bg` 82% + blur 20 + saturate 1.4.
    - Left: surface pill (Timeline / Insights, 32×26 tabs, ink thumb 420ms expo, 10×1.5 green tick).
    - Centre: command field `clamp(260,32vw,440)×28` with a recall/ask chip; Tab flips it; ↵ recall → Quick Recall; ↵ ask / ⌘↵ → `/chat?q=`; hints hidden below 1000px.
    - Right: Chats, bell (badge = worst severity), gear (toggles Settings).
    - Keys: ⌘K recall, ⌘J ask, ⌘1 / ⌘2.
  - **Status bar:** 30px, mono 10px. REC pill + clock, pause/stop, source lanes (toggles when stopped, open `StatusPop` when recording, polled 2s, paused while idle), one flag, engine pill only when unhealthy, today's time, storage, "local only", `?` → `ShortcutsPop` (560px, 2 columns).
- **Timeline `/` (~14–18k, one 7.9k-line page; split it).**
  - **Top bar:** TimelineJumper (calendar + hour list popover), recall chip, OCR provider chip, OCR toggle with badge, frame actions, refresh.
  - **Stage:** radius 12, `contain` image, OCR boxes with hover text chip, match boxes (solid = keyword, dashed = semantic), jump-to-audio, ⋯ menu (Copy image C, Download D, Copy text, Open host), dev metadata overlay.
  - **Rail:** 36px with masked edges, 8px slots against a fixed centre caret, ticks 1×8 / 16 major. App-episode bands carry 18px icons; there's a match mark, an audio lane (mic / sys, 11px bars) and a hover tooltip. The readout flies 9px over 190ms in the scrub direction.
  - **Input:** wheel anywhere scrubs; ←/→ ±1, ⇧ ±10.
  - **Paging:** 200 per page, prefetch 120, cap 5000; head poll 1500ms.
  - **AudioDrawer:** peek 50vh / expanded. TranscriptReader with karaoke and follow mode; WaveformScrubber coloured by speaker; SpeakerRepairPanel slide-over. Space play, ←/→ ±5s, ⇧ ±30s.
- **Insights (~14k).** Sticky blurred 56px header with tabs Overview / Journal / Subjects / Context + counts and a 2px ink underline. Engine notices: pitch / off / unreachable / fix / derivation-off.
  - **Overview:** period capsule; lede with 4 stats; "Where the time went" (stacked columns, focus heat, app time); "What changed"; "Needs attention".
  - **Journal:** `[` / `]` step days, digest lede, spine, receipt viewer (rAF timelapse 8×/16× + cited audio).
  - **Subjects:** Conviction/Movement segmented, conviction map with 14-day tails, tiers, `SubjectDetail` + push-drawer `EvidenceDrawer`, Pin/Dismiss, ⌘J Ask.
  - **Context:** CRUD of authored statements.
  - **Charts:** MiniBars, StackedBar, Timeline, Sparkline, Heatmap, ConfidenceBar.
- **Chat (~4.3k).**
  - Layout: 276px list (240 below 1180px) + thread max-width 760.
  - Question bubble right-aligned, radii `lg lg xs lg`.
  - Steps disclosure with live dot; rows rise in.
  - "Thought process" box keeps its tail in view.
  - Answer blocks prose | bars | dossier | timeline; source chips → FrameDetailModal; actions (copy, regenerate, model, context bar).
  - Composer: Enter send, ⇧Enter newline, Send↔Stop. ModelPicker shared with Settings; ScopePanel; greeting empty state (chips never auto-send).
  - Toasts: deleted (4s); pin fail with Retry (5s).
- **Settings (~19k).**
  - Full window, no titlebar or status bar.
  - Rail 252px: 52px drag strip, "← Back to app" (Esc), `/` search over keywords, 5 groups with tinted icon tiles, footer with REC + version.
  - 52px bar: breadcrumb + save status (saved / saving / resolve issues / couldn't save + Retry).
  - One scroll column with scroll-spy.
  - Groups: General (Appearance, Startup, Shortcuts) · Capture (Sources, Video, Microphone, Privacy) · Intelligence (Providers, Ask AI, User Context, OCR, Transcription, Speakers, Semantic Search, MCP) · Data (Storage, CLI Access) · About (License & Trial, Updates, Developer).
- **Quick Recall (~7.2k).**
  - BarShell: 58px field, body ≤640, 32px hint footer; height reported to the window.
  - Field: mode chip search/ask/auto, filter pills, in-place phrase tinting, ghost completion (→).
  - Results: BestMoment + result cards, with focus staying in the field.
  - QuickLook: 476px, 7-frame strip 15s apart.
  - AskPane: ↵ gate / notice / streaming, numbered citations, "Continue in Chat ↵".
  - RecentRows on ↓ (max 8); FilterPicker.
- **Onboarding (~11.7k).**
  - 52px strip: drawn traffic lights, step name, tick ruler, "N / 8".
  - 8 steps: Welcome (one-shot filmstrip rewind) · Permissions · Capture & Storage (the only hard gates) · Your settings · Change settings · Setup (downloads, never blocks) · Voice (15s) · Finale (starts capture, shows the first real frame + OCR).
  - Read memory `mnema-onboarding-*` notes before touching it.
- **Small windows:**
  - Access request (focus starts on Deny, Enter unbound, Esc denies, then a receipt view).
  - Update (identity band, notes band, controls).
  - Permission helper.
  - Debug (verdict + 5 tabs + mini).
  - `WindowStrip`: 38px, three 12px grey dots that turn red / amber / green on hover; whole strip is a drag region.

### 3.7 Design tokens (port `tokens.css` + `kit.css` to one `theme.rs`; values in `apps/desktop/src/routes/+layout.svelte:431` dark, `:626` light)

| token | dark | light |
|---|---|---|
| bg / fg | #0c0c0e / #e2e2e8 | #f6f6f4 / #14141a |
| surface / subtle / raised / hover | #0e0e16 #101018 #13131a #1a1a2a | #fff #f6f6f4 #fbfbfa #eeeeec |
| border / strong / hover | #1e1e2e #2a2a3a #3a3a5a | #d8d8d4 #c4c4c0 #a4a4a0 |
| text strong/text/muted/subtle/faint | #e2e2e8 #c0c0d0 #9696ae #7e7e98 #33334a | #14141a #2a2a32 #5a5a6a #5e5e6a #9a9aa4 |
| accent / strong / bg / border / glow | #3dffa0 #2a8a60 #0d1f15 #1a4a30 rgba(61,255,160,.18) | #1f7a4a #155a36 #e6f4ec #9bd3b4 rgba(31,122,74,.16) |
| warn / danger / info | #d6a14a / #ff6b7a / #60b0ff | #9a5a12 / #c43a48 / #2b78c5 |
| source screen / mic / sysaudio | #c0b0ff / #80d0a8 / #b0c080 | #6f5ed1 / #2f8e59 / #8b7a2c |
| recording / paused dot | #ff3148 / #d6a14a | #d62236 / #c08018 |

Also from `kit.css`:
- **Derived fills:** hairline = border@80%; wash = fg 4% (5% light); wash-strong = fg 8% (9% light) = selected row. **No left accent bars.**
- **Fonts:** Hanken Grotesk (UI) + Spline Sans Mono (data, timestamps, kickers, kbd). Both are self-hosted woff2 in `docs/mockups/redesign/fonts`; embed them.
- **Type scale:** 10/11/12/13/16/20; body 13/1.6; roles display 40/600, h1 28, h2 20, h3 16, prose 14, label 12/500.
- **Kicker:** mono 10 uppercase, 0.12em tracking, green `//` prefix. Numbers are tabular.
- **Spacing:** 4/8/12/16/24/32/48/64. **Radius:** 4/6/8/12/16/pill. **Control heights:** 26/32.
- **Easing:** expo `cubic-bezier(.16,1,.3,1)`, quart `(.25,1,.5,1)`. Ease-out only, never bounce. **Durations:** 120/220/420ms.
- **Theme change:** cross-fade 180ms.
- **Focus ring:** 3px accent-glow. Disabled 0.4, busy 0.6.
- **Popover:** blur 22, from scale .98 / −4px, shadow `0 8 24 rgba(0,0,0,.22)`. Tooltip dwell 350ms (instant on focus), hides on scroll.
- **Skeletons are static** (no shimmer). Toast rises 8px with a drain bar. Staggered reveal is 50ms per item.
- **Reduced motion:** GPUI `App::reduce_motion()`.
- **Icons:** Lucide only, never unicode glyphs (gpui-kit-assets ships Lucide). Titlebar icons use stroke 1.75.
- **Theme modes:** system / light / dark; follow the OS via `observe_window_appearance`.
- **Time format:** system / h12 / h24; "system" reads the Mac's 24h toggle natively.

## 4. Target architecture

```
crates/app-core/          # NEW: Tauri-free logic moved out of src-tauri, + its tests
  host.rs                 # trait Host: emit(Event), dialog(..), open_url(..), refresh_tray(),
                          #   open_window(WindowRequest), exit(ExitKind)
  paths.rs                # Paths{data, config, cache, log} derived from bundle id + MNEMA_* overrides
  event.rs                # enum CoreEvent { AskAiUpdate{..}, SessionChanged(..), ... } (~30)
  core.rs                 # AppCore: owns the ~35 states (today's .manage() list), Arc-shared
apps/desktop/src-tauri/   # thin: TauriHost impl Host, commands = one-line calls into AppCore
apps/desktop-gpui/        # NEW
  main.rs                 # gpui_platform::application().run(..); helper re-exec stays first
  bridge/tokio.rs         # copy of zed gpui_tokio: runtime as a Global, Tokio::spawn(cx, fut)
  bridge/events.rs        # CoreEvent -> async_channel -> cx.spawn -> Entity updates
  native/                 # objc2 shims: sharing_type, corners, panel, tray, hotkeys, dock
  theme.rs, ui/           # tokens + shared controls (wrap gpui-kit, themed to §3.7)
  shell/, timeline/, insights/, chat/, settings/, quick_recall/, onboarding/, small/
  tests/visual/           # baselines/<surface>-<state>-<theme>.png
```

Rules:
- **One `AppCore`, two hosts.** Every behaviour change lands in `app-core` once. The GPUI side holds view state in `Entity<T>`, subscribes to `CoreEvent`, and calls `AppCore` methods via `Tokio::spawn`/`background_spawn`.
- **Never block the GPUI main thread** on sqlx, IO or model work.
- **Tasks and Subscriptions cancel when dropped:** store them or `.detach()`.
- **Images:** preview pipeline already writes JPEG files. `img(ImageSource::Resource(Path))` + our LRU (192 entries, failures cached 5s), implemented via the `ImageCache` trait. There's no HEIC, and `surface()` only takes NV12 `CVPixelBuffer`, so use RGBA `RenderImage` for decoded video frames. Audio playback needs a native player; today it's base64 into `<audio>`.
- **Accessibility:** every interactive element gets `.id()` + `.role()` (AccessKit only exposes those). This is also what makes UI driving and accessibility-tree tests possible.
- **Files ≤800 lines.** The 7.9k-line Timeline page becomes ~10 single-purpose files.

## 5. Feel contract (assert these in tests; they are the product's feel)

| Behaviour | Constant | Source |
|---|---|---|
| Settings autosave | recording 450ms, source switches 0ms, mic 250ms; blocked while a privacy command runs | `settings/state/autosave-core.ts:78-85` |
| Delayed-commit Undo | 5000ms; collapses to "Dismissed · Undo" + toast drain bar; navigating away commits now | `insights/dismissUndo.svelte.ts` |
| Scrub preview fetch | throttle 160ms (240 fast); display ≤ every 80ms, settle 180ms, exact frame 500ms; prefetch radius 4→1 past 2/4/8 px/ms; stale = 0.55 opacity / 0.6 saturation | `routes/+page.svelte:195-214` |
| Timeline paging | 200 / prefetch 120 / cap 5000 / head poll 1500ms | `+page.svelte:156-248` |
| Quick Recall | search debounce 250ms, commit 700ms, auto-ask 750ms after typing pauses, blur grace 300ms, reset 5s after hide, body ≤640 | `searchStore.svelte.ts:47-49`, `AskPane.svelte:83`, `windows.rs:43-66` |
| Tooltip | 350ms dwell, instant on keyboard focus | `components/tooltip.ts:16` |
| Motion | 120 / 220 / 420ms, expo / quart, ease-out only; segmented + nav thumb 420ms expo; readout 9px / 190ms | `kit.css:29-49` |
| Streaming answer | static 7×15 green caret (no blink); dangling `**` closed while streaming; highlight only after finish; "Copied" 1.2s | `AnswerProse.svelte` |
| Status pop | polls 2s while open, paused while render-idle | `StatusPop` |
| Reload button | spins ≥500ms | `ReloadButton` |
| Keyboard | Alt chords match physical key on macOS; app shortcuts suppressed in inputs/buttons; reserved Esc / Tab / ⇧Tab / frame arrows; help sheet built live per surface | `lib/keyboard*.ts` |

Plus budgets that are new and must be measured:
- **Cold start to first painted main window:** ≤ Tauri's time, measured on the same machine.
- **Frame time during fast scrub and during streaming:** p95 ≤ 8ms on ProMotion (≤16ms otherwise). No dropped frames on popover open.
- **Keystroke → paint** in the command field and Quick Recall: ≤ 1 frame.
- **Idle power** with every window hidden: no render loop. Compare to Tauri with back-to-back runs, per memory `mnema-recording-battery-cost`.
- **Quick Recall summon** (hotkey → first painted key panel): ≤ 50ms warm.

## 6. Testing view and feel

Five layers. A slice is not done until its layers pass and the screenshots have actually been opened and looked at (the Read tool shows PNGs). Grepping for element names is not verification.

1. **Logic (`cargo test -p app-core`).** All existing src-tauri tests move with the code and stay green. The keychain tripwire `MNEMA_TRIPWIRE_NO_KEYCHAIN=1` stays on. The `resume_screen_from_inactivity_*` failures (4) and the known flakes are pre-existing; don't chase them, but don't add to them.
2. **Behaviour (`#[gpui::test]`, `TestAppContext`/`VisualTestContext`, `gpui_kit::test::TestWindowExt`).** Per surface:
   - `simulate_keystrokes` for every binding in §5;
   - `dispatch_action`;
   - `simulate_click`/`simulate_mouse_move`/`simulate_resize`;
   - `advance_clock` to assert **every timing constant in §5** (e.g. autosave does not fire at 449ms and does at 450ms; Undo commits at 5000ms, not before);
   - `run_until_parked`.
   - Use a fake `AppCore` (or a real one on a temp dir) seeded with fixture data.
3. **View: golden screenshots.**
   - `VisualTestAppContext` + `capture_screenshot(window)` renders offscreen with real Metal. It needs no Screen Recording permission and is not affected by `sharingType = none`.
   - For each surface × key state (empty / loading / populated / error / streaming / narrow) × light/dark, save `tests/visual/baselines/<surface>-<state>-<theme>.png`.
   - Compare with a per-pixel tolerance. `UPDATE_BASELINE=1` regenerates, but only with founder sign-off on the diff.
   - Model it on Zed's `visual_test_runner.rs`.
4. **Parity vs the current app.** A `parity` script produces side-by-side composites (GPUI | Tauri | mockup) per state:
   - **Tauri side:** the documented Playwright harness: Vite on :1420, stubbed `__TAURI_INTERNALS__`, never return `null` for unmocked commands (`[]` for `list_*`, `false` for `has_pending*`, `{}` otherwise), fixed viewport. See `docs/mockups/redesign/DEVELOPER-PLAN.md:95-117`.
   - **Mockup side:** `file://…/<page>.html?state=..&theme=..`, screenshot the window element.
   - Same fixture data on all three sides.
   - A **parity agent** writes a gap list per surface (spacing, type, colour, radius, motion, missing states) with a severity, and the composite PNG path.
5. **Feel in the real app.**
   - (a) **Instrumentation build flag:** logs frame time, keystroke→paint, summon latency and startup milestones to a Debug log target (never content). A script turns the log into the §5 budget table.
   - (b) **Real-window drive:** with developer options on (sharingType becomes read-only), `screencapture -x -o -l <windowid>` (window id via a small CGWindowList Swift script) plus accessibility-tree driving (only id+role elements show up; needs a bundled `.app`). The `computer-use` skill is fine for exploratory passes.
   - (c) **Founder feel pass:** you produce a checklist page per surface (anatomy-theme HTML, light+dark, per `~/.claude/CLAUDE.md`) of what to try and what "right" feels like: scrub, summon, type, stream, resize, theme flip, sleep/wake, lock. **The founder runs signed builds themselves; never run `build-macos-local-sign.sh`.** TCC flows (screen, mic, system audio, accessibility) are only testable in a signed build via `open`, never `cargo run` / `tauri dev`.

## 7. How to use Claude Code for this

- **Plan mode first.** Read §2, then write `docs/gpui/PLAN.md` as slices in the format below. Each slice is a standalone section a fresh agent can run alone: goal, what to read, build steps, don'ts, files, verify commands, dependencies. Get my approval before building.
- **Custom subagents** (create in `.claude/agents/`, short and specific):
  - `gpui-slice-builder`: builds one slice in a worktree, runs its verify commands, returns a patch + test output + screenshot paths.
  - `gpui-parity-checker`: read-only; runs layer 4 for a surface and returns the gap list.
  - `gpui-feel-checker`: runs layer 2 timing tests + layer 5a budgets and reports a pass/fail table.
  - `core-extractor`: moves one src-tauri module into `app-core` behind `Host`, keeping both hosts compiling.
  - Reuse the built-ins: `Explore` for lookups, `Plan` for slice design, the `reviewer-*` agents + `deep-review` / `code-review` / `ponytail-review` skills for review.
- **Worktree isolation is mandatory** for every building or reviewing agent (`isolation: "worktree"`). Agents never edit, stash or `git checkout --` the shared worktree; they hand back a patch. You re-apply it and re-run the verify commands yourself. Watch for the shared `CARGO_TARGET_DIR` phantom-error trap (memory `mnema-shared-cargo-target-dir-phantom-errors`) and never run a bare `cargo fmt` across the workspace; scope it to touched files.
- **Workflows** (Workflow tool; load the `workflow-authoring` skill first):
  - **W1 extract-core:** pipeline over src-tauri modules in dependency order. Each module gets an extractor, then a verifier (`cargo test -p app-core` + `cargo check` desktop + both hosts compile). Batches of ≤8 modules.
  - **W2 build-surface:** for one surface, builder → parity-checker → feel-checker → fix loop (max 3 rounds), then hand back.
  - **W3 parity-sweep:** parallel parity-checker per surface; returns one merged gap table.
  - **W4 review:** the reviewer agents over a slice diff, plus an adversarial verifier per finding.
  - Keep each under ~10 agents; give every agent a narrow brief with file paths and the exact verify commands.
- **Skill:** write `.claude/skills/gpui-mnema/SKILL.md` holding the conventions (crate pins, theme tokens API, Host trait, tokio bridge, image cache, the a11y id+role rule, test helpers, the 800-line cap). Every slice agent loads it, so briefs stay short.
- **Hooks** (`update-config` skill), in project `.claude/settings.json`:
  - PostToolUse on Edit/Write of `*.rs` under `apps/desktop-gpui` / `crates/app-core`: `rustfmt` on that file only, plus a line-count check that fails above 800.
  - Stop hook: `cargo check -p app-core -p mnema-gpui`.
- **Monitor / background Bash** for long cargo builds (the first build compiles OpenBLAS from source; source `scripts/openblas-build-env.sh`). Don't sleep-poll.
- **Push notification** to my phone when blocked on a decision or when a workflow finishes while I'm away.
- **Talking to me:** plain language, the answer first, a recommendation with the trade-off. Ask design questions one at a time in prose (not AskUserQuestion cards). Ask before any non-obvious decision. Never put the session URL in commits/PRs (keep `Co-Authored-By`).

## 8. Slice outline (expand each into a full standalone section in PLAN.md)

| # | Slice | Depends on | Done when |
|---|---|---|---|
| S0 | **Spike:** a gpui-kit 0.9 window with our fonts + tokens, a PopUp panel, a tray, a global hotkey, `on_open_urls`, sharingType via objc2, the tokio bridge calling one sqlx query on a temp DB, one `capture_screenshot` golden test | — | All seven work in one throwaway binary. Report surprises. **Stop and show me before S1.** |
| S1 | `crates/app-core` skeleton: `Host`, `Paths`, `CoreEvent`, `AppCore` with the state list; TauriHost implementing it; no behaviour moved | S0 | Tauri app runs unchanged; `cargo test` desktop green |
| S2 | Extract via W1 (low coupling first): conversations, user_context, licensing adapter, chatgpt_auth, models, semantic worker, privacy, debug, usage charts, keyboard model, storage | S1 | Each module's tests run under `-p app-core`; Tauri commands are one-liners |
| S3 | Extract capture: `lifecycle.rs` + the ~36 `segments.rs` AppHandle functions, the notifiers, the not-recording reminder | S1 | Capture tests green in app-core; real recording smoke-tested in the Tauri dev sandbox |
| S4 | Extract startup / deferred startup / graceful exit into `AppCore::start(host)` / `shutdown()` | S2, S3 | Startup order identical (diff the Info log lines of both) |
| S5 | GPUI app shell: main window, titlebar, status bar, surface switch, theme (system/light/dark, cross-fade), fonts, keymap + help sheet, notifications popover | S1, S0 | Layers 2–4 for the shell |
| S6 | Shared controls on gpui-kit, themed to §3.7: button variants, segmented, switch, select/combobox, choice, stepper, slider, input, date input, popover, menu, tooltip, notice, empty, skeleton, toast, modal, WindowStrip | S5 | Gallery window renders every control × state × theme; goldens |
| S7 | Native layer: tray (full menu model), global hotkeys, deep links, single instance, Dock policy, quit → graceful exit, OS notification, dialogs/opener/clipboard, logger | S4, S5 | Each behaviour in §3.5 checked in a bundled dev `.app` |
| S8 | Timeline: stage, OCR + match overlays, rail + bands + audio lane, jumper, scrub engine (§5 constants), image LRU, frame actions | S6 | Feel tests for scrub + goldens + frame-time p95 |
| S9 | AudioDrawer: transcript reader, waveform, speaker repair, native audio playback | S8 | |
| S10 | Quick Recall panel: field, pills, ghost, results, QuickLook, AskPane streaming, recent, height-follow, blur rules | S6, S7 | Summon ≤50ms; all §5 QR constants |
| S11 | Chat: list, thread, turn (steps, thought box, typed answer blocks, sources → FrameDetailModal), composer, model picker, scope | S6 | Streaming frame time; goldens incl. a mid-stream state |
| S12 | Insights: Overview, Journal (+ receipt viewer), Subjects (+ drawer, Undo), Context, charts | S6 | Undo 5s test; goldens per tab |
| S13 | Settings: rail, search, scroll-spy, save status, all 20 sections, autosave engine | S6 | Autosave timing tests; every section golden |
| S14 | Onboarding (8 steps, hard gates, downloads, voice, finale) | S6, S7 | Signed-build TCC pass by the founder |
| S15 | Small windows: access request, update, permission helper, debug (+ mini) | S6, S7 | |
| S16 | Packaging: bundle, Info.plist, entitlements, CLI helper layout, `build.rs` static gfortran, sign/notarize script, release workflow, updater | S7 | Founder-signed build installs, updates from a test feed, refuses a tampered and a wrong-team update (§9.2 install gate), CLI works |
| S17 | Cut-over: identity → `day.mnema`, delete Tauri + Svelte + TS mirrors, update `AGENTS.md` / `SUPPORTS.md` / `turbo.json` / CI | all, founder go | CI green; parity sweep has no open high-severity gaps |

Parallelism: once S5 and S6 land, run S8, S10, S11, S12, S13 and S15 in parallel worktrees (W2 each). S2 and S3 can run in parallel.

## 9. Founder rulings (2026-11-08): locked, do not re-ask

1. **Side by side** (D1). Tauri keeps shipping until S17.
2. **Updater: our own**, not Sparkle. Keep the existing release pipeline exactly:
   - the `release.mnema.day/{stable,preview}/latest.json` feed and format;
   - the minisign pubkey `F6BBB53B2FAA615D`;
   - the license update-window gate, the 2h check timer and the `AppUpdateStatus` state machine.

   Only the download → verify → replace bundle → relaunch step is new code. `macos-release-promote.yml` stays untouched until S16 needs a new artifact name.

   **Install gate.** Before touching the installed app, the downloaded bundle must pass **both** checks; failing either refuses the install:
   - the minisign signature on the archive, as today;
   - an **Apple code-signature check on the extracted `.app`**.

   The code-signature check (Security framework via objc2 / `security-framework`, not by shelling out to `codesign`):
   1. `SecStaticCodeCreateWithPath` + `SecStaticCodeCheckValidity` with `kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSStrictValidate`.
   2. The new bundle's team identifier (`kSecCodeInfoTeamIdentifier`) must equal the **running app's own** team (read via `SecCodeCopySelf`), not a hard-coded string, so Apple Development builds can update each other in testing.
   3. Its bundle identifier must equal the running app's.
   4. The nested `Helpers/mnema-cli.app` must carry the same team.
   5. If the running app has no team (an unsigned or ad-hoc dev build), installing is disabled with a Warn log, never a silent pass.

   A refusal logs at Error with the reason code only, keeps the old app untouched, and shows `Failed` with "This update isn't signed by Mnema" + Retry.

   Tests:
   - a validly signed fixture passes;
   - the same fixture with one byte changed inside the bundle fails;
   - an ad-hoc re-signed copy fails on team mismatch;
   - a bundle with a different bundle id fails.

   Build the fixtures at test time in a temp dir with `codesign`; mark tests that need a real identity `#[ignore]` and document how to run them.
3. **Bundler: a repo script** (`scripts/bundle-gpui-app.sh`). It drives `cargo build --release`, assembles `Mnema.app` (Info.plist, icons, `THIRD_PARTY_LICENSES.md`, `Helpers/mnema-cli.app` + symlink), then reuses `entitle-macos-app.sh` for profile, entitlements and codesign. No `cargo-bundle` / `cargo-packager`.
4. **Native behaviour, same visuals.** Pixels, spacing, type, colour and motion match the current app. Behaviour that macOS does differently from a webview follows macOS:
   - scroll physics and rubber-banding;
   - text selection;
   - context menus (gpui-kit `native_menu`);
   - focus-ring drawing, IME and dictation;
   - window drag and resize.

   Parity gaps of this kind are logged as "native by design", not bugs.
5. **gpui-kit pinned at `=0.9.0` until parity (S17).** After that, upgrade monthly in a dedicated slice that re-runs every golden and feel test. A bug fix that needs a newer snapshot before then is a founder call.
6. **macOS only.** Windows/Linux are out of scope. Don't add `cfg` branches or abstractions for them. `SUPPORTS.md` gets a "GPUI app: macOS only" note.

## 10. Definition of done
- Every surface and window in §3.4/§3.6 exists in GPUI, with goldens in both themes.
- No open high-severity parity gaps.
- All §5 constants asserted by tests; all §5 budgets measured and met (or explicitly waived by the founder).
- Every behaviour in §3.5 checked in a signed build by the founder.
- `app-core` holds the logic; the Tauri host was deleted at S17.
- No file over 800 lines. `AGENTS.md`, `SUPPORTS.md` and the context docs updated.
