# Status popover: implementation plan

Target: `docs/mockups/redesign/status.html` › Part 2 "What Mnema is doing" (approved). States: `ok`, `catching-up`, `display-asleep`, `sysaudio-rebuild`, `low-disk-soon`, `engine-offline`, `not-recording`, `dev-on`. Add `&theme=light` for the light pass.
Branch: `t3/developer-window-and-logging`. Baseline: **`17634c2b`**.
Out of scope: the Developer window (finished, `DEVELOPER-PLAN.md`), the bell, any Rust.

Each slice is a complete brief for one agent. An agent reads **Standing rules + Contracts + its own slice**, nothing else from this plan.

## What the popover is (one paragraph every agent needs)
Everyone gets it, no setting needed. It opens **upward** from the status bar's left cluster when you click the REC pill, the lane group (only while starting/recording; while stopped the lanes stay source toggles), or the new "disk almost full" flag. It shows **now**: Capture rows, a Catching up group, Disk, an AI group **only while the engine is unhealthy** (same rule as the `mx-engine` pill), and an "Open developer window ⌘D" footer **only with developer options on**. The bell keeps **events**. The popover never reads `appNotifications`, never pushes a notification, and never repeats a bell row.

## Key finding: no backend slice
Every "Needs backend" item in the mockup's notes column is already served:

| Mockup note | Served by |
|---|---|
| Display asleep | `get_capture_health` → `screen.state = "waiting"`, `reason = "capture_display_unavailable"` (B2, `native_capture/capture_health.rs`) |
| System audio reconnecting + reason | `get_capture_health` → `systemAudio.state = "reconnecting"`, `systemAudioTap.lastReason` / `lastDetail` / `sampleRateHz` |
| "pauses below 5 GB" | `get_capture_health` → `disk.freeBytes`, `pauseThresholdBytes`, `resumeThresholdBytes` (null while stopped) |
| Deepgram unreachable | `get_processing_pipeline_status` → the `audio_transcription` lane's `retrying` (ADR 0048 requeues set a future `next_attempt_at`), plus `transcription_deepgram_auth_status` (key rejected vs unreachable) |

Also, the mockup's "Free space · `get_storage_location` free_bytes" note is wrong: that command returns a path string. Use `get_capture_health().disk.freeBytes`.

**Gating:** none of these commands is dev-gated. `ensure_window_allowed` (`windows.rs:431`) gates only *opening* a window. App commands have no permissions manifest, so the main window can call them. Reuse them as they are. Don't add a user-facing command.

---

## Standing rules (every slice)

### Repo rules
- Read `AGENTS.md` first. Run every command from the repo root (`/Users/shaikzeeshan/.t3/worktrees/mnema/t3-e6217028`).
- Don't commit unless the founder asks.
- **This plan has no Rust.** If you find you need a Rust change, stop and report it. Don't make it.
- **Never run bare `cargo fmt`** (it churns ~16 unrelated files). If an off-plan Rust change is approved, format with `rustfmt --edition 2021 <the files you touched>` only.
- Keep every file at **800 lines or fewer**. `StatusBar.svelte` is 384 lines today; SP3 must keep it under 800.
- **Don't edit the Developer window** (`lib/debug/**`, `routes/debug/**`, `lib/types/debug.ts`). Import from it only (`reasonPhrase`, `gb` from `lib/debug/capture-view.ts`; `CaptureHealthDto` and `ProcessorPipelineStatus` types). If a type field is missing, report it.
- **Logging:** the frontend adds no log lines. The rule in `AGENTS.md` "## Logging" still applies if anything is ever added: never per poll, never captured content.
- **Plain language:** say "Reading" (not OCR), "Meaning index" (not embeddings), "Transcribing". No raw reason codes, processor keys, ADR numbers or "new" chips in UI text.
- **Polling:**
  - Use the status bar's pattern: `setInterval` gated by `renderIdle()` (`lib/render-idle.svelte.ts`), with an in-flight guard, torn down by the `$effect` that started it.
  - Polls run only while they are needed (see the budget below).
  - Every `invoke` has `.catch(() => null)`, so a failed read hides its row and never throws inside an `$effect`.

### Visual rules
- **The mockup wins on visual vocabulary.** Copy its markup and `ss-*` class structure plus the kit's `mx-*` (`.mx-pop.mx-pop--up`, `.mx-pop__head`, `.mx-kicker`, `.mx-label`, `.mx-progress`, `.mx-btn --ghost --sm`, `.mx-flag`, `.mx-lane--waiting`). They already exist in `lib/styles/kit.css` and `shell.css`. Don't redefine kit classes.
- Reuse components only for **behaviour**: `use:tip` (`lib/components/tooltip.ts`), `trapTabKey` / `getFocusableElements` (`lib/keyboard.ts`), and the open/close/focus pattern of `lib/shell/NotificationsPop.svelte` and `ShortcutsPop.svelte`.
- **Tokens:** only `var(--app-*|--mx-*)` or `color-mix()` of them. Never hex.
- Mono (`.num`) only for numbers, sizes, times and kbd. Popover text is sans. The footer bar is mono and `nowrap`, so the popover must reset `font` and `white-space`. Check this in the render.
- **Icons:** lucide via `~icons/lucide/<name>` (chevron-up for the trigger, check for "All caught up"). Never unicode glyphs.
- **Where CSS lives:** the popover's `ss-*` rules go in `StatusPop.svelte`'s `<style>`. The trigger rules (`ss-anchor`, `ss-trig`, `ss-rectrig`, `ss-trig-flag`) go in `StatusBar.svelte`'s `<style>`. Copy them from `status.html` lines 43–76.

### Verify by RENDERING (mandatory for every UI slice)
Grepping class names produced three false "matches the mockup" verdicts on an earlier redesign. Render the page, screenshot it, and **look at the PNG with the Read tool**.

Set up a harness in your scratchpad (don't commit it):
1. Run `bunx playwright install chromium` once.
2. Start the dev server: `nohup bun --cwd=apps/desktop run dev > <scratch>/vite.log 2>&1 & disown`. It serves on **http://localhost:1420/**, the main window and Timeline route, where the status bar shows.
3. Stub Tauri IPC with `page.addInitScript`. Set the label to **`main`**, or the status bar won't mount (`+layout.svelte` `showStatusBar`):
   ```js
   window.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => STUB(cmd, args), transformCallback: (c) => c, convertFileSrc: (x) => x,
     metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } } };
   window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: async () => {} };
   ```
4. **Never return `null` for an unmocked command.** A `null` where an array is expected throws inside an `$effect` and silently freezes the page. Defaults: `[]` for `list_*` and `get_processing_pipeline_status`, `false` for `has_pending*`, `null` only for `drain_*` and `transcription_deepgram_auth_status`, `{}` otherwise. If a stray `.license-receipt` overlay swallows clicks, hide it. If the layout redirects (onboarding, license), stub those states as complete.
5. **Shape stubs from the real TS types**, using the mockup's numbers for each state. Wrong-shaped data silently renders the empty path:
   - `get_capture_permissions` (session + permissions) and `get_recording_settings` (`lib/types/recording.ts`; set `developerOptionsEnabled` for `dev-on`, and `transcription.provider = "deepgram"` for `engine-offline`)
   - `get_idle_debug` → `{ runtimeSources }` (`lib/types/inactivity.ts`)
   - `get_capture_health` (`CaptureHealthDto`, `lib/types/debug.ts`)
   - `get_quick_recall_index_status` (`lib/types/app-infra.ts`)
   - `get_processing_pipeline_status` (`ProcessorPipelineStatus[]`, processors `"ocr"` and `"audio_transcription"`)
   - `get_ai_runtime_status`
   - `get_usage_charts` → `{ timePerApp: [{ activeMs }] }`
   - `get_storage_usage`
6. Use `colorScheme: "dark"`, then a second pass with `"light"`. Viewport **880×600** (the mockup's window). Open the popover by clicking the trigger the state uses (REC pill, lane group, or flag).
7. Screenshot the mockup at the same state: `file://<repo>/docs/mockups/redesign/status.html?state=<state>` (`&theme=light` for light), using `page.locator('.ss-win').screenshot()`.
8. Compare the status bar and popover regions side by side. Fix overflow, clipping, wrapping, mono/sans mix-ups, spacing, contrast and missing rows. Report which states you rendered.

### Standing verify commands
- `bun run check` (repo root).
- `bun test apps/desktop/src/lib/shell` (pure helpers; `status-bar.test.ts` already lives there).
- No Rust check is needed: this plan changes no Rust. Known pre-existing Rust failures, if anyone does run the desktop suite: the `take_inflight` dead-code warning, 4 `resume_screen_from_inactivity_*` failures, and flakes in native_capture global state, the ask_ai pidfile and speaker_analysis helper start.

### Polling budget

| When | Calls | Cadence |
|---|---|---|
| Recording, popover closed | existing `get_idle_debug` + **new** `get_capture_health` | 2 s each |
| Popover open (any state) | `get_capture_health` (same subscription, ref-counted) + `get_quick_recall_index_status` + `get_processing_pipeline_status` | 2 s |
| Popover open, Deepgram lane retrying | + `transcription_deepgram_auth_status` | 2 s, only while the condition holds |
| Popover opens | the bar's existing `refresh()` (`get_ai_runtime_status`, usage, storage) | once |
| Render-idle (screens asleep or window hidden) | nothing | — |

ponytail: two 2 s capture polls while recording. The upgrade path is to add `writerActive` to `CaptureSourceHealth` and drop the `get_idle_debug` poll. That is not in this plan.

---

## Contracts (fixed here so parallel slices agree)

### SP1 exports (`lib/shell/status-bar.ts`, `lib/shell/status-live.svelte.ts`)
```ts
// status-live.svelte.ts: the bar's and popover's live feed + popover open state
export const captureHealth: { readonly value: CaptureHealthDto | null };
export function subscribeCaptureHealth(): () => void;   // ref-counted 2 s poll; renderIdle-gated; clears value on last release
export const statusPop: { open: boolean };               // single $state object, like keyboardHelp

// status-bar.ts (pure)
export const EARLY_WARNING_SEGMENTS = 12;                // ponytail: ≈1 h of recording at 5-min segments; tune here
export function diskSoon(health: CaptureHealthDto | null, running: boolean, lowDisk: boolean): boolean;
export function sysAudioReconnecting(health: CaptureHealthDto | null): boolean;
```
- `diskSoon`: `running && !lowDisk && freeBytes, pause, resume all non-null && freeBytes < pause + EARLY_WARNING_SEGMENTS × (resume − pause)`. `resume − pause` is one segment estimate (`disk_space.rs`).
- `sysAudioReconnecting`: `systemAudio.state === "reconnecting"`, **except** a silence-watchdog rebuild whose tap is up (`systemAudioTap.lastReason === "silenceWatchdog" && sampleRateHz != null`). That case is still recording; see founder decision 1.

### SP2 exports (`lib/shell/status-pop.ts`, pure)
```ts
export type Part = string | { num: string };             // { num } renders as <span class="num">
export interface PopRow {
  name: string;                                           // "Screen", "Reading", "Free space", "Ask AI"
  dot: "fill" | "ring";                                   // ring = neutral, in-progress, not a problem
  tone: "ok" | "warn" | "danger" | null;
  value: Part[];
  note?: Part[];                                          // the <small> second line
  action?: { label: string; settings: "storage" | "intelligence" | "transcription" };
}
export type CaptureGroup = { kind: "rows"; rows: PopRow[] } | { kind: "stopped" } | { kind: "starting" };
export type CatchingUp = { kind: "caught-up" } | { kind: "busy"; rows: PopRow[]; meaningPercent: number | null } | null; // null = couldn't read
export interface PopInput {
  running: boolean; loadingStart: boolean; lowDisk: boolean;
  health: CaptureHealthDto | null;
  sysAudioReconnecting: boolean;                          // from SP1
  diskSoon: boolean;                                      // from SP1
  index: QuickRecallIndexStatus | null;
  pipeline: ProcessorPipelineStatus[] | null;
  transcriptionProvider: string | null;                   // recordingSettings.transcription.provider when enabled
  deepgramAuthError: string | null;
  engine: { label: string; error: boolean; tip: string } | null;  // status-bar.ts engineFlag() result
}
export function captureGroup(i: PopInput): CaptureGroup;
export function catchingUp(i: PopInput): CatchingUp;
export function diskRow(i: PopInput): PopRow | null;
export function aiRow(i: PopInput): PopRow | null;
```

---

## Tiers and parallel groups

| Tier | Slices (parallel within a tier) | Waits for |
|---|---|---|
| 0 | **SP1** bar: live health, waiting lanes, disk-almost-full flag · **SP2** popover copy (pure) | — |
| 1 | **SP3** StatusPop component + triggers | SP1, SP2 |
| 2 | **SP4** integration, real app, full render pass, doc corrections | SP3 |

Expected size: about 1,000 changed lines across 4 slices.

### Conflict watch

| File | Owner | Others |
|---|---|---|
| `lib/shell/StatusBar.svelte` | SP1 (feeds health in, `waiting` tip, flag opens the popover) → SP3 (triggers, anchor, mount, trigger CSS) | Sequential by tier. SP2 never touches it. |
| `lib/shell/status-bar.ts`, `status-bar.test.ts` | SP1 | SP3 imports only |
| `lib/shell/status-live.svelte.ts` (new) | SP1 | SP3 subscribes only |
| `lib/shell/status-pop.ts`, `status-pop.test.ts` (new) | SP2 | SP3 imports only |
| `lib/shell/StatusPop.svelte` (new) | SP3 | — |
| `docs/mockups/redesign/SHELL.md` | SP1 (status-bar rows) | SP4 (founder-ruling edits only) |
| `docs/mockups/redesign/status.html` | SP4 (notes column only, never the drawing) | — |
| `lib/debug/**`, `lib/types/debug.ts`, `lib/capture-controls.svelte.ts`, `lib/styles/*.css` | nobody | Import only. Report a missing field instead of editing. |

---

# Tier 0

## SP1 — Bar: live capture health, waiting lanes, "disk almost full" flag

**Goal:** the status bar finally draws SHELL's "display asleep" and "system audio reconnecting" lanes, and gains the early low-disk warning flag from `?state=low-disk-soon`. This closes the lane half of SH-B1 and all of LD-B1 (PLAN.md Tier 5) without backend work. It also creates the shared live feed and the popover open state that SP3 uses.

**Read:**
- `status.html` `?state=display-asleep`, `sysaudio-rebuild`, `low-disk-soon` (the bar only) and the notes column.
- `SHELL.md` "Status bar — every recording state".
- `lib/shell/status-bar.ts` + test, `StatusBar.svelte`, and the poll pattern in `lib/capture-controls.svelte.ts:283-340`.
- `native_capture/capture_health.rs` (read-only, for semantics).
- `lib/debug/capture-view.ts` (`reasonPhrase`, `gb`).
- This plan's Contracts › SP1.

**Build:**
- **`lib/shell/status-live.svelte.ts`:**
  - `captureHealth` + `subscribeCaptureHealth()`: a ref-counted 2 s `get_capture_health` poll, renderIdle-gated, with an in-flight guard. It fetches once immediately on the first subscribe and clears the value on the last release. Model it on `subscribeRuntimeSources`.
  - `statusPop = $state({ open: false })`.
- **`status-bar.ts`:**
  - Add `"waiting"` to `LaneMod`.
  - `liveLanes(i, rec)` reads the new `StatusInput.health: CaptureHealthDto | null`:
    - screen `state === "waiting"` with reason `capture_display_unavailable` → `{ mod: "waiting", small: "display asleep" }` (via `reasonPhrase`).
    - `sysAudioReconnecting(health)` → system `{ mod: "waiting", small: "reconnecting" }`.
    - Existing privacy (`error`) and idle (`paused`) rules keep precedence.
  - `diskSoon()` and `sysAudioReconnecting()` per the contract.
  - `statusFlag`: a new lowest-precedence recording flag, after `low-disk`:
    `{ kind: "disk-soon", tone: "info", text: "disk almost full ·", action: "<gb> GB left", tip: null }`.
    Widen `StatusFlag.tone` to include `"info"`.
- **`StatusBar.svelte`** (minimal; SP3 owns the rest):
  - Subscribe to capture health while `captureControls.running`, next to the existing `subscribeRuntimeSources` effect.
  - Pass `health` (and what `diskSoon` needs) into `input`.
  - Add `waiting` to `LIVE_TIP` ("waiting · display asleep" / "reconnecting").
  - In `runFlag`, `case "disk-soon": statusPop.open = true`.
- **Tests** in `status-bar.test.ts`:
  - display-unavailable → screen lane waiting, mic and system still on.
  - Reconnecting with a device change → waiting; silence-watchdog with the tap up → on.
  - The `diskSoon` boundary (just above and just below), false while stopped or already suspended.
  - Flag precedence: privacy restart > privacy retry > low-disk > disk-soon.
- **`SHELL.md` › Status bar:**
  - Add one row: `recording · disk almost full (early warning)` | running | — | `button.mx-flag[data-tone=info]` "disk almost full · <b>6.1 GB left</b>", opens the status popover (`status.html`); once below the pause threshold it becomes the "suspended · low disk" row.
  - Add one sentence under the table: the REC pill and, while starting/recording, the lane group are buttons that open the status popover (`status.html`), with a chevron-up after the lanes. While stopped the lanes stay toggles.

**Don't:**
- Touch `capture-controls.svelte.ts` or replace the `get_idle_debug` poll.
- Build the popover, triggers or chevron (SP3).
- Edit the engine-status table in SHELL.md (founder decision 2).

**Files:** `lib/shell/status-live.svelte.ts` (new), `status-bar.ts`, `status-bar.test.ts`, `StatusBar.svelte`, `docs/mockups/redesign/SHELL.md`. About 250 lines.

**Verify:**
- Standing commands.
- Renders, dark and light, bar only, against `?state=display-asleep`, `sysaudio-rebuild`, `low-disk-soon` and `ok`. Clicking the flag sets `statusPop.open`: assert that in the harness, since there is no popover yet.

**Dependencies:** none. **Parallel-safe** with SP2 (no shared files).

---

## SP2 — Popover copy: plain-language rows (pure)

**Goal:** every word the popover says, as tested pure functions over the contract's `PopInput`. One home for the popover's copy: `lib/shell/status-pop.ts`. The Developer window keeps its own developer-voice copy (`debug_health/rules.rs`, `capture-view.ts`). The one shared piece is `reasonPhrase` / `gb`, imported from `capture-view.ts`, not moved.

**Read:**
- `status.html` Part 2: every `?state=` and the `S = {…}` object in its script (lines 281–344) for the exact strings.
- `lib/types/debug.ts` (`CaptureHealthDto`, `ProcessorPipelineStatus`), `lib/types/app-infra.ts` (`QuickRecallIndexStatus`).
- `status-bar.ts` `engineFlag` and `lib/insights/engine-state.ts` (tip text).
- This plan's Contracts › SP2.

**Build:**
- **`captureGroup`:**
  - `loadingStart && !running` → `starting`.
  - `!running` → `stopped`.
  - Otherwise three rows, Screen / Microphone / System audio:

  | source state | dot | tone | value |
  |---|---|---|---|
  | recording (or system audio `reconnecting` while `sysAudioReconnecting` is false) | fill | ok | "Recording" |
  | screen waiting, `capture_display_unavailable` | ring | null | "Waiting for the display." + " Mic and system audio continue." / " Mic continues." / " System audio continues." (only the audio sources that are recording) |
  | system audio reconnecting | ring | null | "Reconnecting after " + the phrase for `systemAudioTap.lastReason`: deviceChanged "the output device changed", deviceDied "the output device went away", sampleRateChanged "the headset changed mode", excludeListMoved "an excluded app started or quit", silenceWatchdog "a long silence"; no reason → "Reconnecting" |
  | paused, `user_paused` | ring | null | "Paused by you" |
  | paused, `inactivity` | ring | null | "Paused while you’re away" |
  | paused under a lowDisk suspension | fill | warn | "Paused until you free up space" |
  | screen waiting, privacy filter retrying | fill | warn | "Privacy filter couldn’t be applied, retrying" |
  | failed, privacy restart required | fill | danger | "Stopped. Restart recording to resume" |
  | failed (other) | fill | warn | "Not capturing right now" |
  | off, recording and not requested | ring | null | "Off for this recording" |

  The undrawn rows reuse the Developer window's wording (`debug_health/rules.rs`) so both surfaces agree.
- **`catchingUp`:**
  - `index` and `pipeline` both null → `null` (group hidden).
  - **Reading:** `queuedFrames > 0` → ring row, value `[{num: N}, " frames · ", eta]` ("1 frame" when N is 1).
    - `eta` = `queuedFrames × averageCompletedSecondsLast24h` of the `"ocr"` lane. Under 60 s → "under a minute"; otherwise "about " + `{num: "2 min"}` (hours as "2h 14m").
    - Omitted when avg is null.
    - ponytail: one job = one frame, serial throughput; the estimate ignores worker parallelism.
  - **Transcribing:** the `"audio_transcription"` lane, n = `queued + running` (queued already includes retrying), n > 0:
    - `retrying > 0 && transcriptionProvider === "deepgram"` → fill/warn, value `[{num: n}, " clips waiting"]`, note "Can’t reach Deepgram. Retries on its own."; or, when `deepgramAuthError`, note "Deepgram rejected your key." with action `{ "Transcription settings", "transcription" }`.
    - `retrying > 0`, any other provider → ring, note "Retrying on its own."
    - otherwise ring `[{num: n}, " clips"]`.
  - **meaningPercent** = `semanticIndexedPercent` when non-null and `semanticBuilding`, else null.
  - No rows and no meaning → `caught-up`.
- **`diskRow`:**
  - `freeBytes` null → `null`.
  - Normal → `{ "Free space", fill, null, [{num: "182 GB"}] }` (via `gb()` + " GB", binary like the backend thresholds).
  - `diskSoon` → warn, `[{num}, " left"]`, note "Recording pauses below " + `{num: pause}` + " and resumes when you free up space.", action `{ "Open Storage", "storage" }`.
  - `lowDisk` → warn, same value, note "Recording is paused. It resumes on its own once you free up space.", same action.
- **`aiRow`:**
  - `engine` null → `null`.
  - Otherwise `{ "Ask AI", fill, tone, [engine.tip], action { "AI settings", "intelligence" } }`, where tone = danger for "needs reconnect", warn for other errors, null for "not set up".
  - Reuse the tip. Never write a second engine sentence, and never "sign in again" for offline.
- **`status-pop.test.ts`:** one case per mockup state (input shaped from the mockup's numbers → expected rows), plus the undrawn rows above, the ETA boundaries and the Deepgram key-rejected branch.

**Don't:**
- Import Svelte, call `invoke`, or read `appNotifications`.
- Move or edit `capture-view.ts`.
- Show a Speakers backlog or failed-job counts. They aren't drawn; the Developer window has them.

**Files:** `lib/shell/status-pop.ts` (new, about 200 lines), `lib/shell/status-pop.test.ts` (new, about 200).

**Verify:** `bun test apps/desktop/src/lib/shell` and `bun run check`. No renders: there is no UI in this slice.

**Dependencies:** none. **Parallel-safe** with SP1.

---

# Tier 1

## SP3 — StatusPop component and its triggers

**Goal:** the popover itself, opened from the REC pill, the lane group and the disk flag, matching all eight mockup states.

**Read:**
- `status.html` Part 2, every `?state=`, its CSS (lines 43–76) and its trigger markup (`TRIG`, `REC`, the `low-disk-soon` flag, lines 270–355).
- `StatusBar.svelte`, `NotificationsPop.svelte` (open, close, Esc, focus return, `trapTabKey`), `ShortcutsPop.svelte`.
- SP1's `status-live.svelte.ts` + `status-bar.ts` and SP2's `status-pop.ts`.
- `lib/surface-windows.ts` (`openSettings`, `openDebugWindow`), `lib/global-shortcuts.ts` (`toggleRecording`, `openDebug`).

**Build:**
- **`lib/shell/StatusPop.svelte`:**
  - Markup: `div.mx-pop.mx-pop--up.ss-pop[role=dialog][aria-label="What Mnema is doing"]` with `data-open`, `inert` and `aria-hidden` when closed. Head: `.mx-pop__head.ss-pop__head` with kicker "What Mnema is doing".
  - Groups are `section.ss-grp` with a `.mx-label`, in this order:
    1. **Capture:** `ul.ss-rows`. `stopped` → `p.ss-quiet` "Not recording. Press Record or `<kbd>⌥⌘R</kbd>`." (key from `toggleRecording`; drop the "or …" part when unbound). `starting` → "Starting…".
    2. **Catching up:** rows, plus a `.ss-prog` `.mx-progress` "Building meaning index N%" when `meaningPercent` is set. `caught-up` → `.mx-inline.ss-quiet` with a check icon, "All caught up".
    3. **Disk.**
    4. **AI** (only when `aiRow` returns a row).
  - Each row is `li.ss-row[data-tone][data-dot]` (dot · name · value, `small` note, ghost-sm action → `openSettings(action.settings)` and close). `Part` `{num}` renders as `<span class="num">`.
  - Footer, only when `devEnabled`: `.ss-pop__foot` ghost-sm "Open developer window" + `<kbd>` from `openDebug` → close + `openDebugWindow()`.
  - Data:
    - While open, call `subscribeCaptureHealth()`.
    - Poll `get_quick_recall_index_status` + `get_processing_pipeline_status` at 2 s.
    - Fetch `transcription_deepgram_auth_status` only while the Deepgram lane is retrying.
    - Build `PopInput` from `captureControls`, `captureHealth`, SP1's `diskSoon` / `sysAudioReconnecting`, and the `engine` prop.
  - Behaviour:
    - Esc closes (window keydown, skip if `defaultPrevented`).
    - Outside pointerdown closes; ignore targets inside `[data-status-pop-trigger]`.
    - Keyboard-open focuses the first control.
    - Close returns focus to the element that opened it.
    - `trapTabKey` while open.
  - Move the mockup's popover CSS (`.mx-pop.ss-pop` … `.ss-pop__foot .mx-btn kbd`) into `<style>`. Add a `font` and `white-space: normal` reset, because it mounts inside the mono, nowrap footer.
- **`StatusBar.svelte`:**
  - Wrap REC pill + controls + `mx-sep` + lanes in `div.mx-pop-anchor.ss-anchor`, and mount `<StatusPop>` inside it.
  - The REC pill becomes `<button class="mx-rec … ss-rectrig" data-status-pop-trigger aria-haspopup="dialog" aria-expanded aria-controls>` in every state.
  - While `startingOrRunning`, the display lanes go inside one `button.ss-trig` with the lucide chevron-up (`.ss-trig__chev`). Its `aria-label` is "What Mnema is doing" followed by the lane summary. Drop the per-lane `role="status"`, which is invalid inside a button. Keep the per-lane tips.
  - The `disk-soon` flag gets `ss-trig-flag`, `data-status-pop-trigger` and `aria-expanded`.
  - Trigger CSS goes in a `<style>` block.
  - On open, call the existing `refresh()` once, so the AI row is current.
  - Pass `engine`, `platform` and `devEnabled` to `StatusPop`.

**Don't:**
- Read or render `appNotifications`, or add a notification.
- Add a keyboard shortcut (none is drawn).
- Put Restart / Retry actions in capture rows. The flag owns them.
- Change the bar's shape or any state SHELL.md defines.
- Edit `status-bar.ts`, `status-pop.ts` or kit CSS. Report gaps instead.

**Files:** `lib/shell/StatusPop.svelte` (new, about 230 lines), `StatusBar.svelte` (about +70). Keep both under 800.

**Verify:**
- Standing commands.
- Renders of all eight states, dark and light, against `.ss-win`:
  - `ok`, `dev-on` and `catching-up` opened from the lane group
  - `display-asleep` and `sysaudio-rebuild` from the lane group
  - `low-disk-soon` from the flag
  - `engine-offline` from the REC pill
  - `not-recording` from the NOT RECORDING pill
- In the harness, assert:
  - Esc closes and focus returns to the trigger.
  - An outside click closes.
  - Clicking the open trigger again closes.
  - Tab stays inside the popover.

**Dependencies:** SP1, SP2. **Not parallel** (owns `StatusBar.svelte` after SP1).

---

# Tier 2

## SP4 — Integration, real app, full render pass, doc corrections

**Goal:** one coherent bar + popover, honest docs, measured polling.

**Read:** `status.html` (every state + notes column), this plan's polling budget and the founder rulings (top of the reply that ships this plan), and `git diff 17634c2b --stat`.

**Build:**
- **Full render pass**, dark and light, every drawn state against `.ss-win`. Also render the undrawn states and check nothing overflows or contradicts the bar:
  - suspended · low disk
  - privacy retrying / restart
  - paused by you / idle
  - starting
  - STOPPED · disk full
  - AI needs reconnect / not set up
  - Deepgram key rejected
  - narrowest main window (check `tauri.conf.json` min width): the 340 px popover must not clip at the left edge
- **Polling audit:** with the IPC stub counting calls, sit 10 s in each case (recording with the popover closed, open, stopped with it open, render-idle) and report calls/s against the budget table.
- **Real app if runnable** (`bun run tauri -- dev`):
  - The popover opens from each trigger.
  - Switching the output device (or unplugging headphones) shows reconnecting in the lane and the row.
  - A quiet stretch of 2+ minutes does **not** flap the system lane to reconnecting (decision 1).
  - ⌘D and the footer link open the Developer window with dev options on.
  - With dev options off, there is no footer.
- **Docs:**
  - `status.html` notes column only: fix "Free space · `get_storage_location`" → `get_capture_health` `disk.freeBytes`. Move the four "Needs backend" items to "Available today" with their sources (table at the top of this plan). Don't touch the drawing.
  - `IMPLEMENTATION.md` §2 "Status-bar and bell gaps": mark the display-unavailable lane, the reconnecting lane and the low-disk early warning as served by `get_capture_health`.
  - If `PLAN.md` exists locally (it is git-ignored), narrow SH-B1 to the engine-status event and the `AppNotificationAction.tab` widening, and mark LD-B1 done.
  - Apply the founder's ruling on decision 2 to SHELL.md "Engine status".
  - `SUPPORTS.md`: no change expected (macOS-only capture semantics are unchanged). Confirm.
- **Dead code:** none expected. Confirm that nothing in `lib/shell/**` is unused.

**Don't:** redesign; touch the Developer window; add Rust.

**Files:** docs above, plus small fixes the pass finds (under 150 lines).

**Verify:** standing commands, the render report with every PNG pair, and the polling numbers.

**Dependencies:** SP1–SP3.

---

## Optional rows: what ships without them

| Row | Ships without it as |
|---|---|
| Reading "about 2 min" | "38 frames" (no OCR job completed in 24 h, so no average) |
| "Mic and system audio continue." | "Waiting for the display." alone (no audio source recording) |
| Rebuild reason ("after the output device changed") | "Reconnecting" alone (tap has no last reason) |
| Meaning index bar | hidden (meaning search off, or fully indexed) |
| Deepgram "rejected your key" variant | "Can’t reach Deepgram" (auth-status call failed) |
| Disk row | omitted (free space unreadable) |
| Catching up group | omitted (both reads failed) |
| "pauses below N GB" while stopped | not shown: thresholds follow the live session, so the stopped disk row is free space only |

## Decisions for the founder (only what the mockup leaves open)

1. **Silence-watchdog rebuilds.** `get_capture_health` reports "reconnecting" for 60 s after *any* tap rebuild. The silence watchdog rebuilds every 30 s to 10 min during any quiet stretch. Drawn literally, "system · reconnecting" would be lit for much of every quiet hour when nothing is wrong. **Plan default:** a silence-watchdog rebuild with the tap back up reads as "Recording" in both the lane and the row. "Reconnecting after a long silence" shows only while the tap is actually down. Say if you want it shown on every watchdog rebuild.
2. **Engine health in the bell.** SHELL.md › Engine status (and SH-B1) plans one `.mx-notif` row while the engine is unhealthy. The popover's AI row now shows the same condition as *now*, which breaks "never duplicate a row". **Recommend:** drop the standing engine bell row from SH-B1 (pill + popover cover it). If the bell gets anything, it should be a one-off event ("Lost ChatGPT at 14:02"). This plan builds no bell row either way. SP4 applies the ruling to SHELL.md.
