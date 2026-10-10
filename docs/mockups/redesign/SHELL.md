# Redesign shell spec (for page mockups)

Every page links `kit.css` + `kit.js` (kit imports `tokens.css` and the self-hosted fonts in `fonts/`); living reference `index.html`, type rationale `fonts.html`.
Colors **only** via `var(--app-*|--cat-*|--focus-*|--chart-*)` or `color-mix()` of them. Never hex.

## Skeleton (start every page from this)
```html
<!doctype html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8" /><title>Mnema — Timeline</title>
  <link rel="stylesheet" href="kit.css" />
  <script src="kit.js"></script>  <!-- in <head>, NO defer: applies saved theme pre-paint -->
  <style>/* page-only styles, prefixed with the page name, e.g. .tl-… / .in-… / .st-… */</style>
</head>
<body>
<!-- copy the <svg><defs> icon sprite from index.html (shell: i-timeline i-insights i-search i-sparkle i-chat i-bell i-gear i-pause i-stop i-play i-record;
     states: i-alert i-info i-check i-x i-retry i-cloud-off i-lock i-monitor i-disk i-external i-off i-inbox) -->
<div class="mx-desk">
  <div class="mx-window">
    <!-- titlebar: copy VERBATIM from below -->
    <main class="mx-body mx-texture"> … page … </main>
    <footer class="mx-statusbar"> … see Status bar … </footer>
  </div></div></body></html>
```
Window = grid rows `var(--mx-titlebar-h) (38px) / 1fr / 30px`, 1440×900 max. `.mx-body` scrolls; give it your own inner grid.
Pages with no recording context may drop the `<footer>`: set `grid-template-rows: var(--mx-titlebar-h) 1fr` on `.mx-window`.

## Titlebar (copy VERBATIM — identical on timeline / insights / chat / index)
```html
<header class="mx-titlebar" data-tauri-drag-region>
  <div class="mx-titlebar__lead">
    <div class="mx-lights" aria-hidden="true"><i></i><i></i><i></i></div>
    <nav class="mx-nav" aria-label="Main surface">
      <a href="timeline.html" aria-label="Timeline" data-tip="Timeline  ⌘1"><svg width="15" height="15"><use href="#i-timeline"/></svg></a>
      <a href="insights.html" aria-label="Insights" data-tip="Insights  ⌘2"><svg width="15" height="15"><use href="#i-insights"/></svg></a>
    </nav>
  </div>
  <label class="mx-cmd" for="mx-q" data-mode="recall">
    <button type="button" class="mx-cmd__mode" tabindex="-1" aria-label="Switch between recall and ask (Tab)">
      <svg class="mx-cmd__recall" width="14" height="14"><use href="#i-search"/></svg>
      <svg class="mx-cmd__ask" width="13" height="13"><use href="#i-sparkle"/></svg><b>Ask</b>
    </button>
    <input id="mx-q" placeholder="Recall anything…" aria-label="Recall anything — Tab to ask instead" autocomplete="off" spellcheck="false" />
    <span class="mx-cmd__keys" aria-hidden="true">
      <kbd class="mx-cmd__k-idle">⌘K</kbd>
      <span class="mx-cmd__k-recall"><kbd>⇥</kbd>to ask</span>
      <span class="mx-cmd__k-ask"><kbd>↵</kbd>ask in chat</span>
    </span>
  </label>
  <div class="mx-titlebar__trail">
    <a class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" href="chat.html" aria-label="Chats" data-tip="Chats"><svg width="15" height="15"><use href="#i-chat"/></svg></a>
    <div class="mx-pop-anchor">
      <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" data-pop="notif" aria-label="Notifications" data-tip="Notifications"><svg width="15" height="15"><use href="#i-bell"/></svg><span class="mx-badge" data-tone="warn"></span></button>
      <div class="mx-pop mx-notifs" id="notif" role="dialog" aria-label="Notifications"> … see States › Notifications … </div>
    </div>
    <a class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" href="settings.html" aria-label="Settings" data-tip="Settings  ⌘,"><svg width="15" height="15"><use href="#i-gear"/></svg></a>
  </div>
</header>
```
- Overlay titlebar: macOS draws the traffic lights in `.mx-lights` (78px inset) — never put content there. Whole strip is a drag region.
- **Icon-led.** Surface switch = two icon tabs in a pill; kit.js slides `.mx-nav__ink` under the current file and draws the green tick. Chat/Settings links get `aria-current="page"` (pressed look) automatically.
- **One text door.** `.mx-cmd` recalls by default; **Tab** flips to Ask (sparkle "Ask" chip, accent ring), Esc resets. ↵ in recall opens the Quick Recall window pre-filled (mockups: no-op); the Timeline only shows a `from Recall · "…"` landing chip + one mark when a hit is opened from Quick Recall (`?state=landed`). ↵ in Ask, or ⌘↵ in either mode → `chat.html?q=…`. Clicking the leading chip also toggles.
- **No second Ask button, no theme toggle.** The chat icon is the place (history), not a door. Theme lives in Settings → Appearance (`[data-theme-toggle]` or the Appearance seg); mockup shortcut ⇧⌘L.
- Shortcuts (kit.js): ⌘K focus (recall) · ⌘J focus (ask) · ⌘1 Timeline · ⌘2 Insights · ⌘, Settings · ⇧⌘L theme. **Never advertise ⌥Space or ⌥⌘Space on this field**: ⌥⌘Space is the global Quick Recall *window* (`TOGGLE_QUICK_RECALL_DEFAULT`, `src-tauri/src/keyboard_bindings.rs`). Global ⌥⌘R = start/stop recording (show it in the Record/Stop tooltips).
- Narrow (< 1000px container): key hints hide; the field shrinks (clamp 260–440px).

## Engine status — hidden when healthy
The Reasoning Engine is plumbing: when it works the user never sees its name. **Remove every "engine <model>" indicator from page chrome** (Insights' `.in-engine`). Only when it is not healthy, add ONE pill to the status bar (after `mx-spacer`) and one `.mx-notif` row:

| condition (reason code) | pill | page surfaces show |
|---|---|---|
| healthy, or **turned off by the user** (`ai_runtime_disabled`, `user_context_disabled`) | nothing | `mx-notice[data-tone="off"]` "… are off · Turn on in Settings" |
| not set up (`no_providers`, `no_default_model`, `no_provider_key:`, `provider_not_connected:`, `no_base_url`) | `<a class="mx-engine" …>engine <b>not set up</b></a>` | `mx-notice` / `mx-empty` + "Set up AI" |
| `needs_reconnect:<id>` (OpenAI rejected the grant) | `mx-engine mx-engine--error` "engine <b>needs reconnect</b>" | `mx-notice[data-tone="danger"]` + Reconnect |
| `provider_unreachable:<id>`, `local_endpoint_unreachable` | `mx-engine mx-engine--error` "engine <b>offline</b>" | `mx-notice[data-tone="warn"]` "resumes on its own" — never "sign in again" |

All pills link to `settings.html#s-providers`. Model choice is shown where it's chosen (Chat composer, Settings), not as status.

## States — the feedback vocabulary (living reference: index.html › States, Feedback parts)
Every page uses these and nothing page-local for empty / loading / error / offline / off / permission / result feedback.
**Tone** is one attribute on any of them: `data-tone="info|ok|warn|danger|off"` (absent = neutral). It sets `--tone --tone-bg --tone-bd`.
- `ok` = done / saved (green, small marks only) · `info` = heads-up · `warn` = transient or fixable (offline, unreachable, permission, low disk) · `danger` = failed / blocked / needs action · `off` = turned off by choice (dashed, no tint).
- Flat: tinted fill + 1px tone border, no shadows (toasts/popovers excepted), **no left bars**. All motion is off under reduced motion (skeletons go static, spinners stop, indeterminate bars dim, toast drain bar hides).

### Which one?
| situation | component |
|---|---|
| a region has nothing yet | `mx-empty` (pane: `--center`; list/panel: `--compact`) — glyph + title + one line + optional action |
| a region is loading | skeleton in the **shape** of the content (`mx-skel-lines/-row/-chart`, `mx-skel--thumb/--title/--circle`) + `aria-busy="true"` + `<span class="mx-sr">Loading …</span>`. No big spinners. |
| a region failed to load | `mx-empty[data-tone="danger"]` + Retry (`i-retry`) — replaces the region's content |
| offline / provider unreachable | `mx-notice[data-tone="warn"]` (`i-cloud-off`) above stale content, or `mx-empty[data-tone="warn"]` if there is none; say it resumes on its own |
| OS permission missing | `mx-notice[data-tone="warn"]` (`i-lock`) + `mx-btn mx-btn--sm` "Open System Settings" (`i-external`). System audio is *inferred* (`possibly_blocked`): `data-tone="info"`, hedged copy |
| feature turned off | `mx-notice[data-tone="off"]` (`i-off`) + ghost "Turn on in Settings →" deep link. Never a warning. |
| one line under a field / in a row | `mx-inline[data-tone]` (svg + text + optional ghost sm action); `mx-spin--sm` + "Checking…" for in-flight |
| a button's action is in flight | `aria-busy="true"` + `<span class="mx-spin mx-spin--sm"></span>` + "-ing…" label; never only a spinner |
| long work with known size | `mx-progress` `style="--v:.39"` + `mx-progress__meta` (`<b>label</b><span>312 / 800 MB · 2 min left</span>`); unknown → `--indeterminate` |
| result of the user's own action | toast: `mx.toast("Deleted the last 15 minutes", { action: "Undo" })` (ok, 5 s, drain bar) · `{ tone: "danger", action: "Retry" }`. Never for system events. |
| system event the user should see later | a `.mx-notif` row (backend `AppNotification`) |
| app-wide condition that limits the app | `mx-banner` under the titlebar (license / trial / low disk) |
| recording / source / engine condition | status-bar `mx-rec` / `mx-lane` / `mx-flag` / `mx-engine` (below) |

### Markup
```html
<div class="mx-empty [mx-empty--center|--compact]" [data-tone="danger|warn"]>
  <span class="mx-empty__glyph"><svg width="18" height="18"><use href="#i-inbox"/></svg></span>   <!-- 14px icon in --compact -->
  <h4 class="mx-empty__title">Nothing captured yet today</h4>
  <p class="mx-empty__text">Start recording and this fills in as you work.</p>
  <div class="mx-empty__acts"><button class="mx-btn mx-btn--sm">Start recording</button></div></div>

<div class="mx-notice" data-tone="warn"><span class="mx-notice__icon"><svg width="15" height="15"><use href="#i-lock"/></svg></span>
  <b class="mx-notice__title">Screen Recording is off for Mnema</b><p class="mx-notice__text">macOS blocks screen capture until you allow it.</p>
  <div class="mx-notice__acts"><button class="mx-btn mx-btn--sm">Open System Settings<svg width="12" height="12"><use href="#i-external"/></svg></button></div></div>

<div class="mx-helper" data-state="waiting"><span class="mx-helper__icon"><img src="…/128x128.png" alt=""></span>   <!-- permission helper: floats beside System Settings -->
  <div><div class="mx-helper__title">Turn on Mnema in the Screen Recording list</div><div class="mx-helper__state"><span class="mx-spin mx-spin--sm"></span>Waiting for the switch — closes by itself</div></div>
  <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Dismiss"><svg width="14" height="14"><use href="#i-x"/></svg></button></div>

<span class="mx-inline" data-tone="danger"><svg width="13" height="13"><use href="#i-alert"/></svg>OpenAI rejected this key.<button class="mx-btn mx-btn--ghost mx-btn--sm">Retry</button></span>

<div aria-busy="true"><span class="mx-sr">Loading moments</span>
  <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div></div>
<div class="mx-skel-chart"><div class="mx-skel"></div>…×N</div>   <div class="mx-skel mx-skel--thumb"></div>   <div class="mx-skel mx-skel--title"></div>

<div class="mx-progress__meta"><b>Downloading Parakeet</b><span>312 / 800 MB</span></div>
<div class="mx-progress" role="progressbar" aria-valuenow="39" aria-valuemin="0" aria-valuemax="100" style="--v:.39"><i></i></div>
```

### Banner (window top)
Direct child of `.mx-window`, between `<header>` and `<main>`; the window grid adds the row automatically. ONE at a time. Precedence = `licensing-banner.ts`: read-only → revoked → activation lapsed → provisional (≤3 d) → trial (≤7 d), then low disk.
```html
<div class="mx-banner" data-tone="warn" role="status"><p><b>Free trial ends in 3 days.</b> Your history stays searchable after; only new recording pauses.</p>
  <div class="mx-banner__acts"><a class="mx-btn mx-btn--sm" href="…">Buy Mnema</a>
    <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" data-dismiss aria-label="Dismiss"><svg width="13" height="13"><use href="#i-x"/></svg></button></div></div>
```
| state | tone | dismiss | actions |
|---|---|---|---|
| trial 7–4 days | info | yes (re-shows at the next day-count) | Buy Mnema |
| trial 3–2 days | warn | yes | Buy Mnema |
| trial last day | danger | yes | Buy Mnema |
| activation pending ≤ 3 days | warn | yes | — |
| read-only (trial ended) / revoked | danger, `role="alert"` | no | Buy a license · ghost Enter license |
| activation lapsed (offline verification) | danger, `role="alert"` | no | Re-check license (`aria-busy` "Checking…") · Enter license |
| recording paused — low disk | warn | no (clears on resume) | Open Storage |
Licensed, update window ended: **no banner** (nothing stops working; the Update window owns it). Banner buttons are secondary `--sm`, never primary.

### Status bar — every recording state
Left: `mx-rec` · its controls (22px: `style="--_h:22px"`) · `mx-sep` · three `mx-lane`s. Right of `mx-spacer`: at most one `mx-flag` + an `mx-engine` (each only when unhealthy), then `today 6h 12m · 1.2 GB · local only`, then the keyboard-help **?** — a 22px ghost icon button (`i-help`, tip "Keyboard shortcuts · /") that opens the shortcuts `mx-pop` upward; always present, the last thing in the bar. `/` and `?` open the same pop.
| state (source of truth) | `mx-rec` | controls | lanes / flag |
|---|---|---|---|
| off, ready | `mx-rec` "NOT RECORDING" | `mx-btn --sm` Record (`i-record`, tip ⌥⌘R) | **toggles**: `<button class="mx-lane mx-lane--screen" aria-pressed="true">screen</button>` (kit.js flips) |
| off, permission missing (`get_capture_permissions`) | same | Record | lane `mx-lane--blocked` `screen <small>no permission</small>`; `<a class="mx-flag" data-tone="warn">Screen Recording off · <b>Open System Settings</b></a>` |
| off, read-only (license blocks capture) | same | Record `aria-disabled` | lanes `disabled`; `mx-flag[data-tone=danger]` "read-only · <b>buy a license</b>" |
| starting (`loadingStart`) | `mx-rec--starting` + `<span class="mx-spin mx-spin--sm">` "STARTING…" | Stop disabled | `mx-lane--starting` until each source's writer is active |
| recording | `mx-rec--running` "REC <span class=num>02:14:07</span>" | Pause (`i-pause`) · Stop | `mx-lane--on` / `--off` (not requested) |
| one source idle (`runtimeSources.X.paused`) | running | same | `mx-lane--paused` `screen <small>idle</small>` |
| paused · idle (`isInactivityPaused`) | `mx-rec--paused` "PAUSED · idle" | Pause stays **enabled** (locks the session) · Stop | lanes `--paused`; right: "resumes when you're back" |
| paused · by you (`isUserPaused`) | `mx-rec--paused` "PAUSED 00:12" | Resume (`i-play`) · Stop | lanes `--paused` |
| suspended · low disk (`isLowDiskSuspended`) | `mx-rec--suspended` "PAUSED · low disk" | Resume **disabled** · Stop | `mx-flag[warn]` "free up space · <b>resumes on its own</b>" → Storage; banner too |
| stopped by the system (disk full / trial ended / revoked) | `mx-rec--stopped` "STOPPED · disk full" | Record (disabled if license) | `mx-flag[danger]`; the `AppNotification` carries the detail |
| display asleep / disconnected (screen only) | stays running | — | `mx-lane--waiting` `screen <small>display asleep</small>`; mic + system keep `--on` |
| privacy filter failed, retrying | running | — | `mx-lane--error` `<small>stopped</small>` on screen/system; `<span class="mx-flag" data-tone="warn">privacy filter failed · <b>retrying</b></span>` |
| privacy filter, restart required | running | — | `<button class="mx-flag" data-tone="danger">restart to resume screen · <b>Restart</b></button>` |
| system audio possibly blocked (inferred) | running | — | `mx-lane--blocked` `system <small>possibly blocked</small>` + warn flag "no system sound yet · Open System Settings" |
| system audio reconnecting (tap rebuild) | running | — | `mx-lane--waiting` `system <small>reconnecting</small>` |
Lane modifiers: `--on --off --paused --starting --waiting --blocked --error` (+ `--screen|--mic|--sysaudio`); `<small>` = short reason. Recording failures to start are a native dialog (`@tauri-apps/plugin-dialog`), not a toast.

### Notifications (titlebar bell)
Badge = worst unread severity: `<span class="mx-badge">` info · `data-tone="warn"` · `data-tone="danger"`; no notifications = no badge (the bell stays). Popover:
```html
<div class="mx-pop mx-notifs" id="notif" role="dialog" aria-label="Notifications">
  <div class="mx-notifs__head"><span class="mx-kicker">Notifications</span><span class="num">2</span><span class="mx-spacer"></span>
    <button class="mx-btn mx-btn--ghost mx-btn--sm" data-clear-all>Clear all</button></div>
  <!-- load failed: <div class="mx-notifs__error"><span class="mx-inline" data-tone="danger">…Couldn’t load notifications.<button …>Retry</button></span></div> -->
  <ul class="mx-notifs__list">
    <li class="mx-notif" data-tone="warn"><b class="mx-notif__title">Capture paused — low disk space</b><time class="mx-notif__time">08:41</time>
      <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm mx-notif__x" data-dismiss aria-label="Dismiss"><svg width="13" height="13"><use href="#i-x"/></svg></button>
      <p class="mx-notif__text">Free up space and recording resumes automatically.</p>
      <div class="mx-notif__acts"><button class="mx-btn mx-btn--sm">Open storage settings</button></div></li>   <!-- optional -->
  </ul>
  <div class="mx-empty mx-empty--compact mx-notifs__empty"><span class="mx-empty__glyph"><svg width="15" height="15"><use href="#i-bell"/></svg></span>
    <b class="mx-empty__title">You’re all caught up</b><p class="mx-empty__text">Capture, model and update warnings land here.</p></div>
</div>
```
`data-tone` = backend severity (`info|warning→warn|error→danger`). The list, Clear all and the empty state swap by CSS; kit.js `[data-dismiss]` / `[data-clear-all]` remove rows and drop the badge with the last one. Use real titles from `src-tauri` (`Capture paused — low disk space`, `Recording stopped — disk full`, `Recording stopped — trial ended`, `Screen capture paused for privacy`, `Transcription model unavailable`, `Speech detector unavailable`, `OCR engine unavailable`, `Mnema update available`, `Global shortcuts unavailable`, `Microphone VAD fallback`).

## Buttons — ONE system
`.mx-btn` + optional variant + optional size + optional `--icon`. Heights `--h-md` 32 (default) / `--h-sm` 26. Radius 6/5. Padding 12/9. Icon gap 7/6. Sans 500 13/12.
**Flat**: one uniform 1px border (none on primary/ghost), no inset highlight, no bottom-heavy border, no drop shadow. Primary is a solid fill edge to edge.
```html
<button class="mx-btn mx-btn--primary">Save changes</button>               <!-- the only green fill; ONE per view -->
<button class="mx-btn">Export<kbd>⌘E</kbd></button>                          <!-- secondary (default): surface + hairline -->
<button class="mx-btn mx-btn--ghost mx-btn--sm"><svg …/>show OCR</button>   <!-- text-only, wash on hover -->
<button class="mx-btn mx-btn--danger">Delete last 15 min</button>          <!-- neutral, danger ink, tints on hover -->
<button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="…" data-tip="…"><svg width="15" height="15">…</svg></button>
<a class="mx-btn mx-btn--ghost"><svg class="mx-btn__accent" …><use href="#i-sparkle"/></svg>Ask about this week<kbd>⌘J</kbd></a>  <!-- in the header row -->
```
- States: hover = fill shift, press = 1px down (primary also darkens), `:focus-visible` = 2px accent outline at 2px offset (no glow), `aria-pressed="true"` (toggle on = `--mx-selected` fill), `disabled` / `aria-disabled`, `aria-busy="true"` (in flight). `<kbd>` inside = plain muted mono text, no box.
- Ask actions ("Ask about this week", "Ask about Ana") are **ghost** with the `.mx-btn__accent` sparkle (the titlebar's Ask mark), placed in the view's header row beside the scope they ask about (Overview: Insights tab bar, left of the range capsule; subject detail: right end of the breadcrumb). Never a bordered button floating in the content. There is no pill/chip button. `.mx-chip` is status only.
- Icon sizes: 15px in md/sm icon buttons, 13–14px beside a label.
- Status-bar mini buttons: add `style="--_h:22px"`.

## Segmented
`<div class="mx-seg" role="group" aria-label="Range"><button aria-pressed="true">Week</button>…</div>` — same track/thumb as the titlebar switch; sans labels. `--md` (32px), `--mono` for data-valued options (`1m 2m 5m`, `0.5 fps`). Emits `mx-change` (detail = value/text). Use for ≤4 short enums without descriptions.

## Choice — options with descriptions (replaces every radio group)
```html
<div class="mx-choice" role="radiogroup" aria-label="Microphone voice detection">     <!-- tiles in a row (≤4 options) -->
  <label class="mx-choice__opt"><input type="radio" name="vad" checked />
    <span class="mx-choice__icon"><svg width="15" height="15"><use href="#i-wave"/></svg></span>  <!-- optional -->
    <span class="mx-choice__title">Silero <span class="mx-choice__tag">default</span></span>
    <span class="mx-choice__desc">Neural speech detector. Falls back to WebRTC.</span></label>
  …
</div>
<div class="mx-choice mx-choice--list" role="radiogroup">…same children…</div>  <!-- stacked rows, long descriptions / >4 options -->
```
The radio is visually hidden but real (← → keys, form state). Selected = accent hairline + tint + check badge (tiles) / `--mx-selected` fill + check (list). Pure CSS, no JS. No radio circles anywhere.

## Date input — scrubbable capsule + calendar popover
```html
<div data-date-input="day"   data-value="2026-10-09" data-min="2026-08-20" data-today="2026-10-09"></div>
<div data-date-input="range" data-unit="week" data-value="2026-10-05..2026-10-11" data-align="end"></div>
<div data-date-input="period" data-unit="week" data-value="2026-10-09" data-align="end"></div>  <!-- Insights Overview -->
```
- Capsule: calendar icon · label (`Fri, Oct 9` / `Oct 5 – 11`) + relative line (`today`, `This week`, `Last 7 days`) · a **ruler** (one tick per day, height = capture density, selection in accent, future days absent).
- Timeline's jumper is the compact form: one 28px control (calendar · day · mono time), no relative line, no ruler; the relative day lives in its tooltip.
- Step: **drag the capsule** horizontally (tape follows; right = earlier), wheel/trackpad, ←/→, Home/End = latest. Click/Enter = popover.
- Day popover: month calendar with density dots, today underlined, future disabled, "Today". Range popover (Chat scope only — Insights has no custom range; use period): presets (This week, Last week, Last 7/30 days, This/Last month) + range calendar — **drag across days** or click start then end. Step unit follows the selection (week / month / custom span length).
- **Period** (Insights Overview — the app's `RangeMode`: one whole day | week | month, no custom span): capsule = label (`Oct 9` / `Oct 5 – 11` / `October 2026`) + relative line (`today`, `last week`, `3 months ago`) + ruler (one tick per period) + an attached **D · W · M** unit switch (one click; lands on "now" if the current period was showing). Steps exactly one period, never past the current one. Popover leads with Day | Week | Month: Day = calendar, click a day; Week = calendar whose rows are the hit target (hover lights the row, ISO week number, per-day density dots); Month = the year as 12 months with year stepping and per-month density. Footer reset `Today` / `This week` / `This month` (disabled when already current). No presets, no drag-select. `value` is any day inside the period; `el.mxDate.setUnit(u)`.
- Attributes: `data-min` (capture start, default today−120d), `data-max` (default today), `data-today`, `data-unit` (range: week|month · period: day|week|month), `data-align="end"` (popover right-aligned), `data-date-open` (render open — gallery only).
- Events/API: `mx-change` `{start, end, unit}` (ISO, bubbles) on every step/commit; `el.mxDate.get() / .step(±1) / .latest()`; set `el.mxHeat = (Date) => 0..3` before DOMContentLoaded to supply real density. Timeline's jumper reuses the capsule skin only (see above).

## Type roles (Hanken Grotesk = interface · Spline Sans Mono = record)
| role | class | spec |
|---|---|---|
| display | `mx-display` | sans 600 40/1.04, −0.034em (Overview headline) |
| title | `mx-h1` / `mx-title` | sans 600 28/1.12, −0.026em |
| heading | `mx-h2` / `mx-heading` | sans 600 20/1.25, −0.018em |
| subheading | `mx-h3` / `mx-subheading` | sans 600 16/1.3 |
| prose | `mx-prose` | sans 400 14/1.6, muted, 64ch |
| body | (default) | sans 400 13/1.55 |
| small body | `mx-body-sm` | sans 400 12/1.5, muted |
| label | `mx-label` | **sans** 500 12/1.35, subtle (was mono) |
| data | `mx-data`, `.num` | mono, tabular-nums, −0.01em |
| kicker | `mx-kicker` (+`--live`) | mono 500 10, 0.12em caps, green `//` |
| stat | `mx-stat__value` | mono 400 28, tabular, −0.05em |
Tokens: `--font-sans`, `--font-mono` (kit rules use these; `--mx-sans`/`--mx-mono` remain as aliases). Mono never carries sentences, buttons, nav or field text.

## Migration list (old → new)
| old | new |
|---|---|
| `mx-icon-btn` | `mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm` (old class still aliases it) |
| `mx-btn` 30px / `mx-btn--sm` 24px | same classes, now 32 / 26 — drop inline height/padding overrides |
| `mx-ask`, `mx-search--cmd` | gone from pages; titlebar uses `mx-cmd` (legacy CSS kept only for topbar-options.html) |
| theme toggle in titlebar (`mx-theme` + `data-theme-toggle`) | removed; Settings → Appearance only |
| `.in-askme` (Ask about this week pill) | `mx-btn mx-btn--ghost` + `svg.mx-btn__accent` + `<kbd>`, in the header row |
| `.in-engine` (engine claude-sonnet-4.5) | delete; unhealthy-only `mx-engine` in the status bar |
| `.in-step` (‹ Oct 5 – 11 ›) + `#j-prev/#j-next` + Day/Week/Month segmented | Overview: `<div data-date-input="period" data-unit="week" data-align="end">` / journal: `data-date-input="day"`; listen to `mx-change` |
| `.tl-jump` + `.tl-jpop` + `.tl-cal*` | `data-date-input="day"` capsule (+ page-local time field) |
| `.st-radio` radiogroups | `mx-choice` (≤4 options) or `mx-choice--list` |
| `.ch-opt` selected tick lists in popovers | keep as menus (they are menus, not choices) |
| `.tl-glass-btn`, `.tl-latest`, other page button skins | `mx-btn` variants (ghost/secondary, sm) — no page-level button styling |
| `mx-seg` with numbers/units | add `mx-seg--mono` |
| `mx-label` used for times/counts | add `num` (or use `mx-data`) — `mx-label` is now sans |
| `font: … var(--mx-mono)` on labels/buttons in page CSS | `var(--font-sans)`; keep mono only for data/time/kicker |
| `mx-h1` as a hero headline | `mx-display` |

## Class index
- Frame: `mx-desk mx-window mx-titlebar mx-titlebar__lead mx-titlebar__trail mx-lights mx-nav mx-cmd mx-spacer mx-body mx-texture mx-statusbar mx-sep`
- Recording: `mx-rec` + `--starting|--running|--paused|--suspended|--stopped` + `mx-rec__dot`; lanes `mx-lane` (span live / `button[aria-pressed]` stopped) `--screen|--mic|--sysaudio` + `--on|--off|--paused|--starting|--waiting|--blocked|--error` + `<small>`; `mx-flag[data-tone]`; `mx-engine(--error)`
- Type: `mx-display mx-h1 mx-h2 mx-h3 mx-prose mx-body-sm mx-label mx-data mx-kicker(--live) .mono .num`
- Buttons: `mx-btn` + `--primary|--ghost|--danger` + `--sm` + `--icon`; `mx-btn__accent`; `mx-badge`
- Controls: `mx-seg(--md|--mono)`; `mx-choice(--list) > mx-choice__opt (__icon __title __tag __desc)`; `[data-date-input]`; `mx-switch`; `mx-input`
- Chips (status only): `mx-chip` + `--screen|--mic|--sysaudio|--accent|--warn|--danger|--info|--cat`
- Search (in-page): `mx-search`, `mx-search--pill`; matches `<mark>` / `mx-match`
- Surfaces: `mx-panel` + `--raised|--inset|--glass|--interactive`; `mx-panel__head mx-panel__body mx-rule`
- Data: `mx-stat` > `__label|__value(+small)|__delta(--up)`; `dl.mx-spec` > `div` > `dt dd dd.mx-spec__note`
- Float: `data-tip` (+`data-tip-pos="top"`); popover `[data-pop=id]` + `#id.mx-pop` in `.mx-pop-anchor`; `mx-menu`
- States (all take `data-tone`): `mx-empty(--center|--compact) > __glyph __title __text __acts`; `mx-skel(--line|--title|--thumb|--circle)`, `mx-skel-lines|-row|-chart`, `mx-sr`; `mx-spin(--sm|--lg)`; `mx-progress(--indeterminate) > i` + `mx-progress__meta`; `mx-inline`; `mx-notice > __icon __title __text __acts`; `mx-banner > p + __acts`; `mx-toasts > mx-toast(+__timer)` / `mx.toast()`; `mx-notifs > __head __list(li.mx-notif > __title __time __x __text __acts) __empty __error`; `mx-badge[data-tone]`; `[data-dismiss] [data-clear-all]`; `mx-helper > __icon __title __state` (permission helper — its own non-activating window, drawn fixed bottom-centre); `mx-pop--up` (opens upward, for the status bar) + `mx-keys > __cols > __grp > ul > li > span + __k > kbd` (the shortcuts popover)
- Motion: `mx-reveal` + `--i`

## Scales
- Spacing `--s-1..8`: 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64. Control heights `--h-sm` 26 · `--h-md` 32.
- Radius `--r-xs 4 · --r-sm 6 · --r-md 8 · --r-lg 12 · --r-xl 16 · --r-pill`. Controls sm/md, panels lg.
- Motion: `--ease-expo` for movement, `--ease-quart` for color/opacity; `--t-fast 120 · --t-med 220 · --t-slow 420`. Transform/opacity only.

## Do
- Depth = surface step + 1px border. Flat: no inset highlights, no bevels (`--mx-highlight` is a dead no-op kept for topbar-options.html — don't use it). Washes: `--mx-wash` / `--mx-wash-strong`.
- **Selected = `--mx-selected` fill + stronger text.** One style for every list row, nav item, preset, sub-section, choice row, journal/turn row.
- Green only for: live/now, matches, selected marks (choice check, calendar selection, ruler, nav tick), ONE primary button, focus. ~10%.
- Left-aligned, asymmetric layouts; spec rows before card grids; one dominant idea per view. Check both themes.

## Don't
- No hex/rgb, no CDNs (fonts are in `fonts/`), no box-shadow elevation except popovers/tooltips.
- No page-level button, radio, segmented or date-stepper skins — use the kit. No radio circles.
- **No left-border / left-bar selection indicators** (no `border-left`, `inset 2px 0 0`, or `::before` bar on a selected row). Use `--mx-selected`.
- No large green fills, no green glow except live.
- No bounce, no layout-property animation, and **no looping animation of any kind** — not the REC dot (steady dot + static ring), not skeletons (static blocks), not spinners (static ring beside text), not indeterminate bars (static hatch). Motion is one-shot only: enter/exit, state change, progress that tracks real data.
- No page-level empty/error/toast/banner/notice/progress skins (`.tl-toast`, `.rc-toast`, `.ac-note`, `.ac-empty`, `.up-progress`, `.ob-inline.is-warn` …) — use States.
- No texture inside panels; don't redefine kit classes — extend with page-prefixed ones.
