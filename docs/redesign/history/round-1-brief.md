# Mnema full-app redesign — shared brief

**Status:** input to the five mockup directions in `mockups/`. Everything in this file is
binding on every direction. Where a direction disagrees it must say so in its own
"Deviations" note, with a reason.

Date: 2026-08-02. Sources: repo inventory + a design-research sweep (competitors, Apple HIG,
Fluent, Linear/Zed/Raycast shipped CSS, motion/GPU evidence). Research brief is quoted inline
where a number matters; the full sweep lives in the session scratchpad and its URLs are
reproduced in §11.

---

## 1. What is wrong today (the thing we are fixing)

Measured from the current app, not vibes:

1. **The app never says what it is.** The main window opens on a timeline scrubber with no
   statement of the three things Mnema does. A new user sees a grey stage and a tick rail.
2. **Search has no home.** Search lives *only* in a separate always-on-top 1120×720 panel
   summoned by ⌘⌥Space. The main window has a "Search" button in the titlebar that opens a
   different window. The app's second-most-important capability is not in the app.
3. **Results are a list with 150×94px thumbnails.** At that size a screenshot is unreadable —
   which is the whole point of a visual recall product. The right column detail pane is fixed
   at 700/420 in a window that can be 640px wide.
4. **AI outweighs the product.** `lib/insights/` is 17,736 lines vs 4,466 for timeline and
   2,788 for quick-recall. Insights is one of only *two* top-level tabs, owns a persistent
   240px rail and five sub-surfaces. Recording gets three titlebar buttons.
5. **No shared Button.** `.btn` is a class convention re-declared in ~6 files' scoped styles,
   plus a dozen bespoke buttons (`.titlebar__record`, `.license-banner__btn`,
   `.quick-recall__state-btn`, `.gate-cta`, settings buttons…). Same for badges, cards,
   skeletons. This is why it "looks poorly managed" — it *is* unmanaged.
6. **No spacing, radius or elevation scale.** Colors are fully tokenized (good, dual-theme,
   reusable). Every padding/gap/radius is a hard-coded literal per component.
7. **100% monospace at 10–20px.** `body { font-family: var(--app-font-mono) }` globally. Mono
   is the machine voice; using it for human copy at 11px is why the app reads as a debug tool.
8. **Five different error mechanisms.** Native OS dialogs for capture errors, a notification
   bell popover, inline `role="alert"` strips, license banners, and a transient stage box.
   A capture failure opens a modal OS dialog *over* the app.
9. **Main window default is 800×600, min 640×480.** Quick Recall assumes 1120. Nothing was
   designed for one window size.

## 2. Product truths (do not redesign these away)

- Three primary jobs, in this order of screen weight: **Record → Timeline → Search**.
- **AI is a second layer, not a peer.** It may be summoned from anywhere and may annotate
  anything, but it never occupies a primary destination slot, never gates a primary surface,
  and never appears before the user has results.
- Everything is local. Privacy is a *design feature*: on-device status, local paths, exclusion
  state should be ambient and verifiable, never a badge wall.
- The brand voice is terminal-native, calm, precise. Green accent on dark; light theme is
  first-class. Type split: **sans = human voice, mono = machine voice** (labels, timestamps,
  IDs, data, kbd, status).
- Rewind was "superbly executed and well designed" and still lost users over 15%→30% CPU and
  double battery drain. **The resource story is a UI surface**, and the redesign itself must
  not add GPU load (this app has a known WindowServer %GPU issue).

## 3. Information architecture — binding on all five directions

One main window. **Three primary destinations, one secondary layer, one utility surface:**

| Slot | Name | What it is |
|---|---|---|
| Primary 1 | **Timeline** | Scrub your day. Big frame + day scrubber with screen and audio lanes. |
| Primary 2 | **Library** (search) | The grid. Everything you saw and heard, searchable, 3–4 large frames per row. **This is the app's new front door for anything older than "right now."** |
| Primary 3 | **Recording** | Not a page — persistent chrome. State, sources, cost, and the stop/pause control are visible on every screen without navigating. |
| Secondary layer | **Ask** | Summonable over any surface. Scoped by what you are looking at. Never a top-level tab of equal weight. Chat history is a drawer inside Ask, not a rail on the shell. |
| Utility | **Settings** | Its own window (Zed's argument: the workspace window is about your work, except suddenly it isn't). |

**Insights/journal/subjects** — the current AI analytics layer — is demoted to a *section
inside Ask*, or to a card surface reachable from Library. It does not get a shell rail.

Deleted from the shell: the `Timeline | Insights` segmented toggle as the app's spine.

## 4. Screens every mockup must contain

Each direction ships **one HTML file** containing these frames, in this order. Frame sizes are
fixed so the five are comparable. Every frame is rendered at 1:1 pixel size (no scaling).

| # | Frame | Size | Must show |
|---|---|---|---|
| 01 | **Library — populated** (the hero) | 1280×800 | Search field, scope, grid of large frames 3-up, time-bucketed sections, selection state, density control |
| 02 | **Library — searched** | 1280×800 | A real query, match highlighting, screen vs audio results, filter chips, result count, sort/scope |
| 03 | **Library — empty & no-match** | 1280×800 (may be split into two half-height panels) | First-run empty (nothing captured yet) *and* zero-results-for-query. Different states, different copy, different CTA. |
| 04 | **Library — loading** | 1280×800 | Content-shaped skeletons, and the refetch-with-stale-results treatment |
| 05 | **Timeline — scrubbing** | 1280×800 | Big frame, day scrubber with screen + audio lanes, skimmer vs playhead, time labels, hover readout, OCR overlay affordance |
| 06 | **Timeline — audio moment** | 1280×800 | The transcript/speaker surface for a moment with sound |
| 07 | **Recording chrome — all states** | 1280×(as needed) | idle / starting / recording / paused-manual / paused-inactivity / suspended-low-disk / suspended-display-unavailable / source-degraded / permission-missing. One strip per state, annotated. |
| 08 | **Ask — the secondary layer** | 1280×800 | Summoned over a surface. Scope chips (`@screen @today`). Streaming answer, citations back to frames, "continue in chat". Show *why* it is secondary. |
| 09 | **Quick Recall panel** | 720×460 | The global ⌘⌥Space launcher, redesigned. Not a second app — a launcher that hands off to the main window. |
| 10 | **Settings window** | 900×620 | Own window. Nav + one content column. Show at least: Capture, Privacy, Intelligence sections. Include a destructive action and its confirm. |
| 11 | **Menu-bar / tray menu** | native size | The status item menu, since for many users it is the app they touch most. |
| 12 | **CLI access request dialog** | 520×560 | The consent surface an agent triggers. |
| 13 | **Error gallery** | full width | Every error placement pattern, side by side, in context. See §8. |
| 14 | **Component appendix** | full width | See §7. Non-negotiable, and it is the last thing in the file. |
| 15 | **UI/UX pattern scale** | full width | See §9. Per-screen scorecard + per-button audit. |

Onboarding is **out of scope** — it was redesigned separately (`docs/onboarding/mockups/`,
direction "Cinematic / Rewind"). Each direction must state in one line how its shell inherits
or contradicts that onboarding.

### Window sizing decision (binding)

The current 800×600 default cannot hold a 3-up grid of legible frames. All directions assume:

- **Main window: 1280×800 default, 1000×680 minimum.** Below 1000 the grid drops to 2-up and
  the sidebar (if any) auto-collapses — Apple explicitly sanctions auto-hiding a sidebar on
  window resize.
- **Quick Recall: 720×460**, non-resizable, anchored at `top: 13vh` of the screen (Linear's
  command menu geometry). The current 1120×720 always-on-top panel is a second app; a launcher
  should be a launcher.
- **Settings: 900×620** own window. **CLI access: 520×560** (unchanged).
- Never apply your own `border-radius` to the main window — macOS Tahoe draws 10px itself.

## 5. Design tokens — paste this block verbatim, then extend

Colors are inherited from the shipping app (they are good, and dual-theme). Spacing, radii,
type, motion and elevation are **new** and are the main thing this redesign adds.

```css
:root {
  /* ---- surfaces: elevation is a surface STEP, not a shadow (Linear, Zed) ---- */
  --bg-0:#0c0c0e; --bg-1:#0e0e16; --bg-2:#13131a; --bg-3:#1a1a2a;
  --surface-hover:#1a1a2a; --surface-active:#131320;

  /* ---- text: four steps, no more ---- */
  --fg-0:#e2e2e8;  /* strong  */ --fg-1:#c0c0d0; /* body   */
  --fg-2:#9696ae;  /* muted   */ --fg-3:#7e7e98; /* subtle */
  --fg-faint:#45455a; /* decorative only, sub-AA — never for text a user must read */

  /* ---- borders: three steps + hairline rule ---- */
  --border:#1e1e2e; --border-strong:#2a2a3a; --border-hover:#3a3a5a;
  --hairline:1px;

  /* ---- one accent ---- */
  --accent:#3dffa0; --accent-strong:#2a8a60; --accent-bg:#0d1f15;
  --accent-border:#1a4a30; --accent-contrast:#07120c;
  --accent-glow:rgba(61,255,160,.18);

  /* ---- semantic ---- */
  --danger:#ff6b7a; --danger-strong:#ff4455; --danger-bg:#2e0f14; --danger-border:#4a1a20;
  --warn:#d6a14a;   --warn-strong:#c47a30;   --warn-bg:#1a1208;   --warn-border:#7a4a18;
  --info:#60b0ff;   --info-strong:#4a6aaa;   --info-bg:#0c1a2e;   --info-border:#1a3050;
  /* recording red is its own thing, never the danger red */
  --rec:#ff3148; --rec-fg:#ff8a96; --rec-bg:#1a0f12; --rec-border:#3a1820;

  /* ---- capture sources (screen / mic / system audio) ---- */
  --src-screen:#c0b0ff; --src-screen-bg:#1a1a3a; --src-screen-border:#2a2a5a;
  --src-mic:#80d0a8;    --src-mic-bg:#0f2e1f;    --src-mic-border:#1a4a30;
  --src-sys:#b0c080;    --src-sys-bg:#2a2010;    --src-sys-border:#4a3a18;

  /* ---- type: sans = human, mono = machine ---- */
  --font-sans:"Hanken Grotesk",-apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI Variable",system-ui,sans-serif;
  --font-mono:"Spline Sans Mono","Berkeley Mono",ui-monospace,"SF Mono",Menlo,monospace;
  /* macOS Body is 13pt/16pt = lh 1.23. Web's 1.5 is the loudest "not a Mac app" tell. */
  --t-micro:10px;  --lh-micro:1.4;   --ls-micro:.02em;   /* mono labels, kbd */
  --t-caption:11px;--lh-caption:1.35; --ls-caption:.01em;
  --t-body:13px;   --lh-body:1.25;   --ls-body:-.006em;  /* THE default UI size */
  --t-prose:14px;  --lh-prose:1.55;  --ls-prose:-.008em; /* only for reading: transcripts, AI answers */
  --t-title:15px;  --lh-title:1.3;   --ls-title:-.016em;
  --t-h2:20px;     --lh-h2:1.25;     --ls-h2:-.02em;
  --t-h1:26px;     --lh-h1:1.2;      --ls-h1:-.022em;
  --w-regular:400; --w-medium:510; --w-semi:590;   /* three weights. no 700. */

  /* ---- spacing: Zed's ramp — finer under 8, coarser above ---- */
  --s-0:0; --s-1:1px; --s-2:2px; --s-3:3px; --s-4:4px; --s-6:6px; --s-8:8px;
  --s-12:12px; --s-16:16px; --s-20:20px; --s-24:24px; --s-32:32px; --s-40:40px; --s-48:48px;

  /* ---- radii: 4–8 controls, 12 containers, 10 = OS window (never ours) ---- */
  --r-sm:4px; --r-md:6px; --r-lg:8px; --r-xl:12px; --r-pill:999px;

  /* ---- control heights (hit target: 28px min, 44px for anything you press often) ---- */
  --h-xs:20px; --h-sm:24px; --h-md:28px; --h-lg:32px; --h-xl:40px;
  --row:28px;          /* list row; AppKit's modern default is 24 + 17px intercell */
  --titlebar:38px;

  /* ---- motion: two durations, that is all ---- */
  --dur-quick:100ms; --dur-regular:250ms; --dur-out:150ms; --dur-in:0ms;
  --ease:cubic-bezier(.4,0,.2,1); --ease-out:cubic-bezier(0,0,.2,1);

  /* ---- elevation: exactly two shadows, and only for things that float ---- */
  --shadow-popover:0 8px 24px rgba(0,0,0,.32);
  --shadow-modal:0 24px 64px rgba(0,0,0,.48);
  --ring:0 0 0 2px var(--accent-glow);
  --ring-danger:0 0 0 2px color-mix(in srgb,var(--danger) 30%,transparent);
  --disabled-opacity:.4; --busy-opacity:.6;
}

[data-theme="light"] {
  --bg-0:#f6f6f4; --bg-1:#ffffff; --bg-2:#fbfbfa; --bg-3:#eeeeec;
  --surface-hover:#eeeeec; --surface-active:#e8f1ea;
  --fg-0:#14141a; --fg-1:#2a2a32; --fg-2:#5a5a6a; --fg-3:#5e5e6a; --fg-faint:#9a9aa4;
  --border:#d8d8d4; --border-strong:#c4c4c0; --border-hover:#a4a4a0;
  --accent:#1f7a4a; --accent-strong:#155a36; --accent-bg:#e6f4ec;
  --accent-border:#9bd3b4; --accent-contrast:#ffffff; --accent-glow:rgba(31,122,74,.16);
  --danger:#c43a48; --danger-strong:#b42332; --danger-bg:#fff0f2; --danger-border:#e4b6be;
  --warn:#9a5a12;   --warn-strong:#7f4300;   --warn-bg:#fff1df;   --warn-border:#dfbc8a;
  --info:#2b78c5;   --info-strong:#225fa3;   --info-bg:#eef5ff;   --info-border:#bdd3ef;
  --rec:#d62236; --rec-fg:#c81d2e; --rec-bg:#ffffff; --rec-border:#ecbcc2;
  --src-screen:#6f5ed1; --src-screen-bg:#f1edff; --src-screen-border:#cdc3f2;
  --src-mic:#2f8e59;    --src-mic-bg:#e8f5ec;    --src-mic-border:#afd8bf;
  --src-sys:#8b7a2c;    --src-sys-bg:#faf4df;    --src-sys-border:#dbc98a;
  --shadow-popover:0 8px 24px rgba(21,28,38,.14);
  --shadow-modal:0 24px 64px rgba(21,28,38,.22);
}

@media (min-resolution:2dppx) { :root { --hairline:.5px } }
@media (prefers-reduced-motion:reduce) {
  *,*::before,*::after { animation-duration:1ms!important; transition-duration:1ms!important }
}
```

Rules that go with the block:

- **Nothing outside `:root` may declare a colour literal, a px padding, or a radius.** If you
  need a value that is not a token, add a token.
- Fonts: load Hanken Grotesk + Spline Sans Mono from Google Fonts in the mockup, with the
  system stack as fallback. In the shipped app they will be bundled.
- Elevation ladder: `--bg-0` window → `--bg-1` panel → `--bg-2` card → `--bg-3` raised/hover.
  Shadows are for *floating* things only (popover, menu, modal, toast). No card shadows.

## 6. The grid — the single most important surface

The user's complaint is precise: results are a list of tiny thumbnails. Geometry:

- **Fixed 16:9 cells, `object-fit: cover`, row-aligned. No masonry** — Eagle's own docs say
  waterfall is "sore to the eyes when searching", it fights lazy-loading, and row position
  carries rank/recency here.
- Target cell height ~180px; let the column count fall out of the width:
  **2-up < 1000px · 3-up 1000–1500px · 4-up ≥ 1500px.** At a 1280 window with a 220 sidebar
  and 20px insets, 3-up gives cells of ~325×183 — an actual legible screenshot.
- **Gutter == container inset.** Pick one number (12 or 16) and use it for both (Flickr ships
  10/10).
- Metadata lives **below** the cell, max two lines: line 1 = app icon + app/window title,
  line 2 = time + match count. Nothing over the image except (a) a duration/type chip for
  audio and (b) one hover-only corner affordance.
- Category/source is a **background tint behind the label**, never a coloured dot
  (Lightroom, Resolve).
- **Density = 3 discrete steps** (Comfortable / Default / Compact), not a slider. One key
  toggles info density (Lightroom's `J`).
- **Hover ≠ select.** Hover reveals the corner affordance and a border, instantly, no
  transition. Click selects. Selection is a 2px accent ring + a checkable state. Enter/space
  opens the moment.
- **Sections are temporal** (Today / Yesterday / Mon 28 Jul), sticky headers, with a count.
  Uniform cells mean exact section heights → an honest scrollbar with no estimation pass.
- Audio results share the grid: same cell box, waveform/energy render instead of a frame,
  speaker + quoted line below. They are not a separate list.
- Show at most 50 cells in the DOM; annotate that in the mockup rather than faking 500.

### Faking screen content (required, and it is what makes or breaks the mockup)

A grid of grey rectangles proves nothing. Build a small reusable set of **CSS-drawn fake
screenshots** and use them throughout — no external images, no `<img>` placeholders, no
lorem-grey blocks:

`.shot--editor` (code lines, sidebar, tab bar, syntax colour bands) · `.shot--browser` (chrome
bar, article column, sidebar) · `.shot--figma` (canvas, frames, layer panel) · `.shot--chat`
(message bubbles, member list) · `.shot--terminal` (mono lines, prompt, one green line) ·
`.shot--sheet` (grid cells, header row) · `.shot--call` (participant tiles, control bar) ·
`.shot--docs` (title, paragraph rules, comment margin).

Each is ~8–20 elements of pure CSS at small scale; at 325×183 they read instantly as "that is
VS Code / that is a browser". Reuse the same eight everywhere, varying the accent hue. This is
also the honest thing to do: it shows the design working on real-shaped content.

## 7. Component appendix (frame 14) — required contents

The bottom of every mockup file is a component sheet. This is what turns five pictures into
five design *systems*. For each component show **every state**, labelled, in a row:
`rest · hover · active/pressed · focus-visible · disabled · loading/busy` (plus `selected`
where it applies, plus `error` for inputs).

Required components:

1. **Button** — variants: `primary`, `secondary`, `ghost`, `danger`, `icon-only`; sizes `sm`
   (24) / `md` (28) / `lg` (32). One shared class, variants as modifiers. Show the loading
   state as a spinner *replacing the icon, not resizing the button*.
2. **Record control** — the special-cased primary action: idle→record, recording→pause+stop.
   It is the only element allowed to use `--rec`.
3. **Input** — text, search (with leading glyph + clear), with error, with helper, disabled.
4. **Select / Combobox**, **Segmented**, **Switch**, **Checkbox**, **Radio**, **Slider**,
   **Stepper** — rest + focus + disabled at minimum.
5. **Chip / filter pill** — removable, static, with count. **Badge** — neutral/accent/warn/
   danger/info. **Source chip** — screen/mic/system-audio in every capture state.
6. **Card** — the grid cell itself, in rest/hover/selected/error(thumbnail failed to load).
7. **Kbd** — box min 20×20, radius 4, 13px label, 4px gap, modifier keys `min-width:48px`
   left-aligned so shortcut columns align optically (Linear).
8. **Tooltip**, **Popover/menu**, **Modal**, **Toast** — one each, with their placement rules.
9. **Skeleton** — content-shaped, for a grid cell and for a text row. Static, no shimmer.
10. **Empty state** — the template: glyph, one-line lead, one-line sub, one action.
11. **Status pill** — capture state; **Progress** — determinate + indeterminate.
12. **List row** — 28px, hover background, no `cursor:pointer` (native lists don't).
13. **Section header**, **Divider/hairline**, **Scrollbar** treatment.
14. **Tab / nav item** — rest/hover/selected, with the selected indicator you chose.

Underneath the sheet, a short **"rules"** list: what each variant is *for*, when never to use
it, the one-primary-button-per-view rule, and the hit-target floor (28px; 44px for record).

## 8. Error handling (frame 13) — placement is the design

Today there are five uncoordinated mechanisms. Replace with **four placements and a rule for
choosing between them.** The user's constraint: errors must be *visible* without *getting in
the way*.

| Placement | Use when | Behaviour |
|---|---|---|
| **Inline, at the control** | The user's last action failed and the fix is right there (bad filter syntax, invalid path, key rejected) | `role="alert"`, red hairline on the field, one sentence + the fix. Never a toast for this. |
| **Surface strip** (top of the content area, under the toolbar) | The *surface* is degraded but usable (search index rebuilding, audio lane failed to load, OCR unavailable) | Persistent, one line + one action, dismissible only if it is safe to ignore. Pushes content down — does not overlay it. |
| **Toast** (bottom-trailing, stacked, max 3) | An async background action finished or failed and the user has moved on (frame export failed, retention run failed) | Auto-dismiss 6s for success, **never auto-dismiss an error**; has one action + a close. Never covers the record control. |
| **Modal dialog** | Only when the app cannot continue and needs a decision (destructive confirm, permission lost mid-recording, disk full) | Two buttons max, destructive on the right in `danger`, focus on the safe choice. |

Plus: **the notification centre** (bell) is the *archive* — every strip and toast is also
written there, so nothing is lost when a toast dismisses. That is what makes non-blocking
errors safe.

Rules to render in the frame:

- Error copy is **what happened → why → what to do**, in that order, one sentence each, no
  stack traces in the UI (the detail goes behind "Show details", which reveals a mono block).
- Never use the recording red for errors: `--rec` is a *state*, `--danger` is a *problem*.
- A failed capture must never open a native OS dialog over the app (today it does).
- Every error has an owner surface. Show at least: capture start failed, permission revoked
  mid-session, disk full (suspension, not failure), display disconnected (liveness, **not** an
  error — show it styled as *info*), search parse error, model missing, cloud provider auth
  failed, thumbnail decode failed (a per-cell state), transcription provider offline (transient,
  requeued — must read as "retrying", not "failed").
- Show one *degraded but working* case, because that is the most common real state: recording
  continues on mic + system audio while the screen is unavailable.

## 9. UI/UX pattern scale (frame 15) — required scorecard

Two tables, rendered in the mockup (not in prose).

**A. Per-screen scale.** Rows = frames 01–12. Columns = the ten checks below. Cell = a 1–5 dot
scale + a ≤10-word justification. Be honest; a 3 with a reason is worth more than a wall of 5s.

1. **Hierarchy** — is the single most important thing on this screen unmistakable in 1s?
2. **Signifiers** — can you tell what is clickable without hovering?
3. **States** — hover/focus/active/disabled/loading/empty/error all designed?
4. **Spacing** — every value from the ramp; consistent inset == gutter?
5. **Type** — ≤3 weights, ≤4 sizes on this screen; sans for human, mono for machine?
6. **Contrast** — body text ≥4.5:1, large ≥3:1; no accent text on translucent material?
7. **Motion** — transform/opacity only, ≤250ms, nothing animating unfocused?
8. **Error clarity** — right placement per §8, non-blocking, actionable?
9. **Keyboard** — full path without a mouse; focus order sane; shortcut discoverable?
10. **Density** — legible at the minimum window size; does it degrade gracefully?

**B. Per-control audit.** Every interactive element on frames 01–12, one row each:
`screen · element · variant · size/hit-target · all 6 states? · keyboard name · destructive?`
Flag anything under 28px hit target or missing a focus-visible style.

## 10. Motion budget (binding)

The app runs all day and already has a WindowServer %GPU problem. Hard rules:

1. **Only `transform` and `opacity` animate.** No animated `box-shadow`, `filter`,
   `backdrop-filter`, `width/height`, `top/left`, or `background-position`.
2. **Two durations: 100ms quick, 250ms regular.** Fades out at 150ms, **fade-in at 0ms**.
   Nothing exceeds 250ms.
3. **No hover animation in the grid** — hover is an instant background/border change. Apple:
   "avoid adding motion to UI interactions that occur frequently."
4. Frame preview on hover requires a **120–200ms dwell**, and ships an off switch.
5. **The playhead is never transitioned** — it is a `transform: translateX()` driven by
   pointer/rAF.
6. **Nothing animates when the window is unfocused.** No pulsing record dot in the WebView —
   a static dot in-app; if something must breathe, it is the native tray icon.
7. At most **one** blurred surface on screen, and it goes opaque when the window is inactive.
   Prefer an opaque surface step over translucency for anything that is not transient.
8. `prefers-reduced-motion` → 1ms override, and hover-scrub preview off entirely.

Every mockup must include a visible **"Motion inventory"** note listing each animation it uses,
its property, its duration, and where it fires. If the list is longer than ~8 entries, cut.

## 11. Mockup file conventions

- **One self-contained HTML file** per direction in `docs/redesign/mockups/`, no build step, no
  external assets except the two Google Fonts. Opens straight in a browser.
- Page chrome around the frames: dark neutral background, a title block, a short direction
  statement (≤80 words), then each frame preceded by `## NN — Name` and a ≤2-line note on the
  decision it demonstrates. Frames are `.frame` elements at true pixel size with a 1px border
  and the window's own chrome drawn inside.
- Include a **theme toggle** at the top that flips `data-theme` on `<html>`, and render at
  least frames 01 and 05 correctly in both themes (check it, don't assume).
- No lorem ipsum. Every string is real Mnema copy: real app names (VS Code, Figma, Slack,
  Safari, Zoom, Notion, Terminal, Mail), real-shaped window titles, real timestamps for
  2026-08-02, real query strings a user would type ("stripe webhook error", "what did Priya
  say about the launch date").
- Tabular numerals wherever a time or a count is shown (`font-variant-numeric: tabular-nums`).
- Accessibility is not deferred to the real implementation: `role`, `aria-*`,
  `:focus-visible`, and live regions appear in the mockup markup.
- End of file: **Deviations** (what you did differently from this brief and why) and
  **Motion inventory**.

## 12. Direction assignments

Five directions, deliberately different in *shell shape and navigation model*, all satisfying
§3–§10. Every direction gets the same content, so they are comparable.

| # | File | Direction |
|---|---|---|
| 01 | `01-quiet-sidebar.html` | **Quiet Sidebar** — macOS-native. Inset sidebar (220px, auto-collapse <1000), unified toolbar, content extends under the sidebar. The safe, obviously-Mac answer done impeccably. |
| 02 | `02-command-canvas.html` | **Command Canvas** — no sidebar at all. One unified toolbar, a scope switcher, full-bleed content, keyboard-first with an omnipresent command bar. Linear-calm. |
| 03 | `03-cockpit.html` | **Cockpit** — the day scrubber is the app's permanent spine, docked at the bottom on every surface (NLE model). Content above switches between viewer and grid. |
| 04 | `04-split-browser.html` | **Split Browser** — grid + persistent detail pane (Photos/Mail model). Time-bucketed sections; selection always has a preview; search is the front door. |
| 05 | `05-terminal-native.html` | **Terminal Native** — Mnema's own brand identity elevated hard: mono machine chrome, green-on-dark, data-dense status surfaces — but with the big legible grid and a real sans for human copy. Proves the brand can be calm. |

Each direction must, in its statement, name **the one thing someone will remember** about it,
and be honest about what it sacrifices.
