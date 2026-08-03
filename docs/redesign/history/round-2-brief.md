# Mnema redesign — round 2 brief

Supersedes [`BRIEF.md`](round-1-brief.md) where they disagree. Round 1's five directions
(`mockups/01`–`05`) got the grid right and the information architecture wrong. This brief
fixes the architecture and adds a hard visual constraint. Everything in `BRIEF.md` §5 (tokens),
§8 (errors), §9 (scorecard), §10 (motion) still applies **except** where §3 below overrides it.

Round 2 ships `mockups/06`–`10`.

---

## 1. What changed, and why

Founder feedback, verbatim, with the decision it forces:

> "I don't like how the library and the timelines are over the surface. The library is not
> really a surface I want. The surface I want is the AI as a secondary surface, like having two
> surfaces with the user able to select one surface as its main."

**Library is deleted as a destination.** Search is not a place you navigate to — it is
something you summon. Round 1 made Library the front door; that was wrong.

**The app has exactly two surfaces: Timeline and Overview (AI).** They are peers in a
switcher, and the user picks which one the app opens on. This is close to what ships today
(`Timeline | Insights`), and that part of today's app was right — what was wrong was that AI
had a persistent rail and five sub-surfaces while search had no home at all.

> "I need you to create, with the same windows as the current app, a main window for the
> timeline and windows for the AI overview and quick look for the search and ask."

**Same window set as the app has today.** Round 1's "Settings gets its own window" is
**reverted** — Settings stays an in-main route, as it ships. No new windows are invented.

> "I don't like how the boxes are there, and the [nested] box has a darker border on the left
> side… Everywhere that the style is used, it doesn't look good at all."

**The box-in-box style is banned.** See §3. This is the single highest-priority constraint in
this brief; a direction that nails the architecture and keeps the boxes has failed.

---

## 2. Architecture — binding

### The two surfaces (main window)

| Surface | What it is | Weight |
|---|---|---|
| **Timeline** | Scrub your day. The big frame, the day scrubber with screen and audio lanes, the moment you are looking at. This is the product. | Primary by default |
| **Overview** | The AI layer: what your day/week amounted to, conversations you've had with your history, subjects, context. | Secondary by default |

- One switcher, two peers. Whichever is *not* current is one keystroke away (`⌘1` / `⌘2`).
- **The user chooses which surface the app opens on.** Show the mechanism explicitly — an
  affordance on the surface itself ("Open Mnema here" / "Make this my main surface") **and**
  the matching Settings row. A direction that only asserts this in prose has not designed it.
- Secondary-by-default is a *default*, not a cage: if a user makes Overview their main, the
  app opens there and Timeline is one key away. What must never happen is AI outranking
  recording — the record control and capture state stay in the chrome on both surfaces.
- Today's Insights sub-surfaces (journal, subjects, context, chat) collapse **into Overview**
  as sections or a single scrollable surface. They do not get a rail of their own.

### The Quick Look window (today's `quick-recall`, `⌘⌥Space`)

**Search and Ask both live here, and only here.** This is the "quick look" — summon it over
anything, get an answer or a moment, get out.

- Two modes in one window: **Search** (default) and **Ask** (`⌃↵`). Same window, same field,
  a mode you switch — not two products.
- **This is where the big-frame grid goes.** The 1120×720 window is exactly right for it: at
  20px insets and a 16px gutter, 3-up gives **349×196** cells — four times the pixel area of
  today's 150×94 list rows. All of `BRIEF.md` §6 (fixed 16:9, row-aligned, no masonry,
  metadata below, gutter == inset, hover ≠ select, temporal sections, ≤50 in the DOM) applies
  here now instead of to a Library page.
- Results hand off to the main window: opening a moment focuses Main on Timeline at that
  instant. Quick Look is a launcher, not a second app — but unlike round 1 it keeps its real
  size, because a grid needs the room.

### Windows — the exact set, unchanged from what ships

| Window | Route | Size | Change from today |
|---|---|---|---|
| **main** | `/` → Timeline · Overview | **1100×720 default, 800×600 min** | Today's *default* becomes the *floor*. A day scrubber plus a legible frame does not fit in 800×600, but nothing breaks there. |
| **quick-look** | `quick-recall` | **1120×720**, non-resizable, always-on-top panel | Unchanged. Now holds the grid. |
| **settings** | `/settings` **in main** | — | **Reverted to today's behaviour.** Not its own window. |
| onboarding | `onboarding` | 1120×800 | Unchanged, out of scope (already redesigned). |
| cli-access-request | `access/request` | 520×560 | Unchanged. |
| debug | `debug` | 980×680 | Unchanged, out of scope. |
| tray | — | native | In scope — for many users it is the app they touch most. |

Do not invent a window. Do not move Settings out of main.

---

## 3. The visual constraint — de-boxing

The complaint is specific and it is correct. In round 1 a search result is a bordered cell
containing a bordered caption strip; a detail pane is a bordered panel containing a bordered
OCR block. Where an inner border sits on an outer border the two hairlines stack into a
heavier, darker edge — most visibly down the left side. The pattern repeats everywhere, so the
artifact repeats everywhere.

**Rules, in force on every frame of every round-2 direction:**

1. **A bordered element may not contain another bordered element.** One border per visual
   group, maximum. If you need a second level of structure, use spacing or a surface step.
2. **No border may sit flush against another border.** If two edges would touch, delete one.
3. **Frame captions have no container.** App name, window title, time and match count sit
   directly on the surface under the image — no fill, no border, no rounded strip. The image
   is the object; the text is its label.
4. **Depth is surface step, spacing, or (for floating things only) a shadow.** Never an edge.
   The ladder is `--bg-0` window → `--bg-1` region → `--bg-2` raised. A region change should
   usually be *tone alone*, with no seam at all.
5. **No left-accent-bar callouts.** No `border-left: 2px solid` quote/callout/tooltip style,
   anywhere, in the app or in the mockup's own annotation chrome.
6. **Panels separate by one hairline or by nothing.** A split view gets a single seam, not a
   border on both sides of the gap.
7. **Rounded corners are for things that float or that clip an image.** A section of a page
   does not need a radius; a popover and a thumbnail do.
8. **Count your borders.** Each direction must state, in the file, how many bordered elements
   are on its busiest frame. If the number is above ~12 on a 1100px window, cut.

**Prove it.** Every direction opens with a **before/after strip**: one round-1 grid cell as
built, beside the same cell in this direction, with the removed edges called out. Two cells,
side by side, no prose longer than three lines. If a reviewer can't see the difference
instantly, the direction has not solved the problem.

**What replaces the boxes** is the direction's actual job. Five different answers are assigned
in §6.

---

## 4. Screens every round-2 mockup must contain

Fourteen frames, in this order. Main-window frames render at **1100×720**; Quick Look at
**1120×720**. 1:1, no scaling.

| # | Frame | Window | Must show |
|---|---|---|---|
| 00 | **Before / after** | — | Round-1 cell vs this direction's cell. Borders removed, called out. |
| 01 | **Timeline — the main surface** | 1100×720 | Big frame, day scrubber with screen + audio lanes, skimmer vs playhead, time labels, hover readout, recording chrome, the surface switcher |
| 02 | **Timeline — an audio moment** | 1100×720 | Transcript, speakers, the audio surface for a moment with sound |
| 03 | **Timeline — first run & loading** | 1100×720 | Nothing captured yet (with the one action that fixes it) *and* the loading treatment. Two states. |
| 04 | **Overview — the second surface** | 1100×720 | What the AI layer actually shows: the day/week, subjects, conclusions, entry to a conversation. Recording chrome still present. |
| 05 | **Overview — a conversation** | 1100×720 | Ask in progress: streaming answer, citations that point back to real moments, history |
| 06 | **Overview — engine not configured** | 1100×720 | The honest state when no AI provider is set up. **This must not read as a paywall or a nag.** Timeline must remain fully usable behind it. |
| 07 | **The surface switch** | — | How the two surfaces relate: the switcher in both positions, the "make this my main surface" affordance, and the Settings row that matches it. This frame is the answer to the founder's core ask — do not treat it as an afterthought. |
| 08 | **Quick Look — search results** | 1120×720 | The grid. 3-up, 349×196 cells, temporal sections, screen and audio results in one grid, filter chips, selection, match highlighting |
| 09 | **Quick Look — Ask mode** | 1120×720 | Same window, other mode. Scope chips, streaming answer, citations, hand-off to Overview |
| 10 | **Quick Look — empty, no-match, error** | 1120×720 | Three states. Orientation before you type ≠ no results for a query ≠ search failed. |
| 11 | **Recording chrome — all states** | strip | idle / starting / recording / paused-manual / paused-inactivity / low-disk suspension / display-unavailable (**info, not error — mic and system audio keep recording**) / source degraded / permission missing |
| 12 | **Settings — in the main window** | 1100×720 | As a route, the way it ships. Nav + one content column. At least Capture, Privacy, Intelligence. Include the "which surface opens by default" row, one destructive action, and its confirm. |
| 13 | **Tray menu + CLI access dialog** | native / 520×560 | Both, side by side. |
| 14 | **Error gallery** | full width | `BRIEF.md` §8 — four placements, one rule for choosing, everything archived in the bell. De-boxed. |
| 15 | **Component appendix** | full width | `BRIEF.md` §7 — every component, every state. **Rebuilt de-boxed** — this is where the new visual system is actually specified. |
| 16 | **UI/UX pattern scale** | full width | `BRIEF.md` §9 — per-screen scale (rows are frames 01–13) + per-control audit. |

Then: **Deviations**, **Motion inventory**, **Border count** (§3 rule 8).

---

## 5. Carry-over from round 1 — reuse it, don't rebuild it

- **Tokens**: `BRIEF.md` §5, verbatim.
- **The CSS-drawn fake screenshots are the best asset round 1 produced. Copy them.**
  `../mockups/round-1/02-command-canvas.html` and `../mockups/round-1/04-split-browser.html` both have a complete
  `.shot--*` set (editor, browser, figma, chat, terminal, sheet, call, docs) that reads as real
  apps at cell size. Lift the CSS wholesale, then adapt. Do not spend your budget redrawing
  them, and do not ship grey rectangles.
  Known trap from round 1: percentage `padding` on a `.shot` child resolves against the
  *containing block width*, so a side panel silently collapses. Verify by screenshot.
- **Errors, motion budget, scorecard rubric**: `BRIEF.md` §8/§9/§10 unchanged.
- Round-1 grid geometry rules move from Library to Quick Look, otherwise intact.

---

## 6. Direction assignments — five answers to "what replaces the boxes"

All five share §2's architecture. They differ in the visual system that replaces box-in-box,
and in how they express the two-surface switch.

| # | File | Direction |
|---|---|---|
| 06 | `06-paper.html` | **Paper** — zero borders anywhere. Structure is surface tone and generous whitespace only; regions meet with no seam at all. Light theme is the primary design, dark derived from it. The risk to beat: formless. |
| 07 | `07-rules.html` | **Rules** — one device: the horizontal hairline. Structure comes from rules and strict alignment the way a well-set document does; nothing is ever enclosed on four sides. The risk to beat: it reads as a spreadsheet. |
| 08 | `08-full-bleed.html` | **Full Bleed** — the frame *is* the surface. Content runs edge to edge; chrome floats over it, appearing on demand and getting out of the way. Containers don't exist because there is nothing to contain. The risk to beat: chrome that hides when you need it. |
| 09 | `09-panes.html` | **Panes** — large flat regions in different surface tones, meeting at a single seam. Native split-view logic: depth is tone, never edge. The risk to beat: tone steps too subtle to read as structure in dark theme. |
| 10 | `10-type-led.html` | **Type Led** — hierarchy carried by typography, weight and spacing almost alone; the only non-text object on screen is the captured frame itself. Closest to Mnema's calm-terminal voice, fully de-boxed. The risk to beat: everything the same visual weight. |

Each direction states, in ≤80 words: what it replaces boxes with, the one thing someone will
remember, and what it sacrifices. Be honest about the risk named above — a direction that
claims it fully dodged its own risk is not being honest.
