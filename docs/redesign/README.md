# Full-app redesign

Three rounds, converged. **`mockups/16-converged.html` is the current candidate**; its
binding decision record is [`DECISIONS.md`](DECISIONS.md). Round 1 got the grid right and the
architecture wrong; round 2 fixed the architecture but was rejected whole on feel
(*"don't like any of them… make sure they feel like a native app"*); round 3 rebuilt the
visual system on five native macOS idioms and was narrowed to 13 + 14, which 16 merges.

```
open docs/redesign/mockups/16-converged.html               # CURRENT — the candidate
```

Layout: `system.css` (the implementable system) and `DECISIONS.md` (current, binding) live
here; `history/` holds the round briefs and revision work orders. **Only the final design is
kept on disk** — the fifteen round mockups (01–15) were deleted on 2026-08-03 once 16
converged; the briefs below record what each direction was and why it lost.

| | Brief | Mockups | Status |
|---|---|---|---|
| Round 1 | [`history/round-1-brief.md`](history/round-1-brief.md) | `01`–`05` | Superseded on architecture. Tokens (§5), errors (§8), scorecard (§9), motion (§10) still binding. |
| Round 2 | [`history/round-2-brief.md`](history/round-2-brief.md) + [`history/round-2-revision-1.md`](history/round-2-revision-1.md) + [`history/round-2-revision-2.md`](history/round-2-revision-2.md) | `06`–`10` | **Rejected on feel** (2026-08-03). Its architecture, constraints and build-spec frames all carry into round 3 and stay binding. |
| Round 3 | [`history/round-3-brief.md`](history/round-3-brief.md) (+ everything round 2 binds) | `11`–`15` | Narrowed to 13 + 14 ([`DECISIONS.md`](DECISIONS.md), 2026-08-03). |
| Converged | [`DECISIONS.md`](DECISIONS.md) | [`16`](mockups/16-converged.html) | **Current.** 13 Grouped × 14 Material, per the founder's keeps. |

**`16-converged.html` is the live candidate**: 13's grouped calm as the base system with 14's
scroll-under-material chrome; Overview is 14's bento carrying 13's digest-with-frame-citations,
the important-moments strip, and a **Conversations tile** replacing audio-minutes (the
competitor-research outcome in `DECISIONS.md` — the unit is the conversation, not the
recording); 13's chat UI, toasts and CLI dialog; and the **state pill** — one calm capsule +
native popover replacing the title-bar button cluster — **adopted 2026-08-03** as the
recording chrome everywhere (frame 11 specs it; the old cluster is kept there only as the
replaced treatment). Settings icons follow the shipping monochrome convention. The pill
primitive and icon rule live in `system.css` §6. Timeline frozen as ever, mic/sys lane kept
as a coverage indicator.

---

# Round 3 — five kinds of Mac app

Round 2's directions were graphic-design systems — paper tone, hairlines, scrims, tone seams,
typography. None read as a Mac app. Round 3 asks a different question: **which kind of Mac app
is Mnema?** Each direction is a native idiom with a family of real system apps behind it.
All five pass the nine-point **native bar** in `history/round-3-brief.md` (HIG window anatomy, accurately
drawn AppKit controls, NSMenu anatomy, materials only where macOS uses them, system-accent
selection, native density, nothing web-shaped) and each carries a Native audit block after
frame 16 saying where every point is proven.

| # | Direction | Idiom family | The one thing to remember | Busiest-frame borders |
|---|---|---|---|---|
| 11 | **Source List** | Finder, Notes, Mail | Translucent source-list sidebar + unified toolbar; collapsed on the frozen Timeline, earning its width on Overview and Settings | 9 |
| 12 | **Inspector** | Final Cut, Logic | Footage is the document: a capture LCD dead-centre in the toolbar, a 280px ⌘4 inspector of dense key/value groups | 11 |
| 13 | **Grouped** | System Settings | Calm single-level grouped fills organise every surface; media stays naked, groups organise words | 11 |
| 14 | **Material** | Music, Safari, Control Center | Content scrolls edge-to-edge *under* translucent chrome; Overview is a Control-Center tile grid | 9 |
| 15 | **Utility** | Activity Monitor, Console | The monitor: striped tables, segmented toolbar, a 22px status bar of live mono counts on every window | 9 |

Each states its named risk and reports honestly on whether it beat it: Source List vs *empty
sidebar chrome*, Inspector vs *pro-tool intimidation*, Grouped vs *everything reads as a
preferences pane*, Material vs *translucency as gimmick*, Utility vs *spreadsheet coldness*.

**How to review round 3:** same order as round 2 (frame 00 before/after — the "before" is now
a round-2 frame; 01 Timeline; 07 the surface switch; 08 the Quick Look grid; 06 no-engine;
14/15/16 errors/appendix/scorecard) **plus the Native audit block after frame 16** — it is the
round's acceptance test. The de-boxing clarification this round: a single-level filled group
(System Settings style — rounded fill, no border, never nested) counts as a surface step and
is allowed.

## Start here if you are implementing

**[`system.css`](system.css) is the source of truth** and the thing that actually ships. Its
colour tokens are byte-for-byte the ones in `+layout.svelte` today, so the whole block can be
pasted into that `:root` without changing any existing rule. What it adds is everything the app
has never had: a type ramp where **size is a consequence of role**, named spacing constants, a
control-metric ladder, an elevation rule, and the shared `.btn` / `.input` / `.toast` / `.kbd`
primitives that replace `.btn` being re-declared in six files' scoped styles.

Every mockup carries a verbatim copy of it, then a clearly marked `/* direction-specific */`
block listing exactly what that direction adds. In each file:

- **Frame 17 — Component → code map.** Every component, the class or Svelte component it
  becomes, the file it replaces today, its variants and states. Also flags what needs a backend
  change before it can be built.
- **Frame 18 — Type & spacing specimen.** Each of the six type roles with its full spec, the
  rule for when to use it, and three real examples pulled from that file's own frames at true
  size — plus a "if you want 12/15/20/26px, use this instead, because…" table, and every
  spacing constant shown as a real labelled pair at its real distance.
- **Frame 19 — Where it's used.** Timeline and Overview re-rendered with every text element
  callout-labelled with its token and the key gaps dimensioned.

### The type ramp

| token | px | lh | tracking | weight | family | for |
|---|---|---|---|---|---|---|
| `--t-label` | 10 | 1.4 | +.02em | 510 | mono | machine labels, column heads, kbd, units. Never a sentence. |
| `--t-meta` | 11 | 1.35 | +.01em | 400 | either | timestamps, counts, helper lines, frame captions |
| `--t-ui` | **13** | 1.25 | −.006em | 400 | sans | **the default** — buttons, rows, labels, nav, menus |
| `--t-read` | 14 | 1.55 | −.008em | 400 | sans | prose only: transcripts, AI answers, errors. Max 70ch |
| `--t-title` | 17 | 1.3 | −.016em | 590 | sans | screen and section titles, dialog headings |
| `--t-display` | 22 | 1.2 | −.02em | 590 | either | **one per screen** — the readout clock, a hero number |

Six sizes, three weights (400/510/590), 1.25 for UI and 1.55 for prose. Gone from every file:
12, 15, 16, 20, 26px and weights 300/680. The app's default body size moves 12 → 13 (macOS
Body) and gains a line-height and tracking it never had.

**The timeline is frozen.** `routes/+page.svelte` is rendered as it ships — same structure,
proportions and treatment. The only change is that it now draws on the shared type roles and
spacing constants. Ideas for improving it are recorded in each file's Deviations as proposals,
not applied.

Every file is self-contained, has a light/dark toggle, and contains every frame plus a
component appendix and a UI/UX scorecard.

---

# Round 2 — two surfaces, de-boxed

## What the feedback changed

1. **Library is deleted as a destination.** Search is not a place you navigate to. Round 1
   made it the front door; that was wrong.
2. **The app has exactly two surfaces — Timeline and Overview (AI) — and the user picks which
   one it opens on.** Peers in a switcher, one keystroke apart. Recording chrome lives on both,
   so AI can be your main surface without ever outranking the record control.
3. **Search and Ask both live in the Quick Look window** (today's ⌘⌥Space panel, 1120×720).
   That is where the big-frame grid went: 3-up, **349×196** cells.
4. **Same window set as ships today.** Round 1's "Settings gets its own window" is reverted —
   Settings is an in-main route, as built. No window was invented.
5. **Box-in-box is banned.** The founder's read was exact: a bordered cell containing a
   bordered caption, and where the inner edge meets the outer one the hairlines stack into a
   heavy dark edge, most visibly down the left. Round 1's busiest frame carried ~38 bordered
   elements. Round 2's carry **0 to 12**, and each file opens with a before/after cell.

`history/round-2-brief.md` §3 has the eight de-boxing rules. The short version: one border per group, never
flush against another, captions have no container, depth is tone or space — never an edge.

## Revision 1 — what changed after the second review

[`history/round-2-revision-1.md`](history/round-2-revision-1.md) is the work order. Four corrections, applied to all five files:

1. **The timeline is the shipping one again.** Round 2 invented NLE lanes, hour rulers, zoom
   levels and density bars. All deleted. Every direction now rebuilds on what
   `routes/+page.svelte` actually does — one big stage, a fixed-height rail-wrap under it with
   an 8px-per-frame tick rail and app-group bands (newest frame anchored right), a *sibling*
   two-row audio lane (mic over sys), and a pointer-following readout — then improves the
   execution. Stage heights landed at **514–558px of a 720px window (71–77%)**, up from
   round 2, mostly by cutting side columns that were duplicating the readout.
2. **Errors are toasts.** The surface strip is deleted from the system — it inserted a row
   under the toolbar and reflowed the stage, so the image you were looking at jumped when a
   background job failed. Toasts sit bottom-right, stack to three, overlay content, never
   reflow, and never auto-dismiss on error. Between them the five directions found and removed
   **43 flow-inserting error elements**, and each deleted the strip class outright so it can't
   come back. Frame 14 is now toasts *in situ* over live surfaces.
3. **The title bar degrades on a designed ladder** instead of by accident: cost/GB readout →
   elapsed timer → three source glyphs collapse to one → icon-only pause/stop. The state dot,
   Stop and the surface switcher never go. The capture cluster is now 20–28% of the bar, from
   ~36%. Rendered as its own strip in frame 11.
4. **Three window widths.** Timeline and Overview are rendered at **800×600** (the floor),
   **1100×720** (default) and **1440×900** (wide). Quick Look stays fixed at 1120×720, as it
   ships. The grid runs 2-up under 1000px, 3-up to 1500, 4-up above.

The two-surface switcher and "make this my main surface" were confirmed good and left alone.

## The five

| # | Direction | Replaces boxes with | Remember | Borders, busiest frame |
|---|---|---|---|---|
| 06 | **Paper** | Surface tone and air. Zero borders; light is the primary design, dark derived | A captured frame is the only hard edge on screen | **0** |
| 07 | **Rules** | One horizontal hairline, repeated, all at the same two x-positions | The day's time axis is a hairline, and every other rule is its sibling | 12 (Settings) |
| 08 | **Full Bleed** | The image itself, a scrim, and a floating plate. Nothing is inset | The screenshot running under the traffic lights | 6 (Settings); 0 on Timeline, Overview, Quick Look |
| 09 | **Panes** | Three tones meeting at a single seam — one hairline in dark, none in light | The seam | 8 (state gallery); 1–5 on real screens |
| 10 | **Type Led** | Size, weight, tone, space. Five weights, four tones, sans=human / mono=machine | No container anywhere, and you still always know where to click | **1–2** (the window edge) |

Each names its own risk and reports honestly on whether it beat it: Paper vs *formless*, Rules
vs *spreadsheet*, Full Bleed vs *chrome that hides when you need it*, Panes vs *tone steps
invisible in dark*, Type Led vs *everything the same weight*.

## How to review round 2

1. **Frame 00** in all five — the before/after cell. If you can't see the difference instantly,
   that direction didn't solve your complaint.
2. **Frame 01** — Timeline as the main surface.
3. **Frame 07** — the surface switch and "make this my main surface". This is the answer to
   what you asked for; judge it hardest.
4. **Frame 08** — Quick Look search results. The grid, at 349×196.
5. **Frame 06** — Overview with no AI provider configured. It must read as honest, not as a nag.
6. **Frames 14/15/16** — errors, the de-boxed component appendix, the scorecard.

The five differ only in the visual system and how the switch is expressed. Architecture,
tokens, grid geometry, error taxonomy and motion budget are shared — so picking one is picking
a look, not a rebuild.

---

# Round 1 — superseded (five shell directions)

Kept for the research and the component work. **Its information architecture is dead**: it
made Library a top-level destination and gave Settings its own window. The grid geometry,
tokens, error taxonomy, scorecard rubric and motion budget all carried into round 2, as did
its CSS-drawn fake app screenshots.

## The three decisions every round-1 direction shares

1. **Record → Timeline → Library(search)** are the three primary jobs. AI ("Ask") is a
   summonable second layer scoped to what you're looking at. It never owns a destination slot.
   Today's app has the opposite balance: Insights is one of two top-level tabs with a
   persistent rail and five sub-surfaces, while search lives in a *different window*.
2. **The grid is the front door.** Fixed 16:9 cells, row-aligned, 3-up at 1280 — cells between
   285×160 and 405×228 depending on direction, against today's 150×94 list rows.
3. **Main window becomes 1280×800** (min 1000×680, from 800×600/640×480). Quick Recall shrinks
   from a 1120×720 second app to a 720×460 launcher. Settings gets its own 900×620 window.

## The five (round 1)

| # | Direction | Shell | Cell size @1280 | The thing you remember | What it costs |
|---|---|---|---|---|---|
| 01 | **Quiet Sidebar** | 220px inset sidebar + unified toolbar; collapses to a 64px rail under 1000px | 328×185 | Conventions done impeccably — it's a Mac app and you already know how to use it | 220px of a 1280px window; pinned searches and the cost readout vanish when collapsed |
| 02 | **Command Canvas** | No sidebar. One 44px toolbar carries nav + search + record state; ⌘K is the real navigation | 405×228 | The search field lives in the title bar and never leaves | Mouse-only users lose the browsable rail; discoverability rides on the toolbar and ⌘K |
| 03 | **Cockpit** | Day scrubber permanently docked at the bottom of every surface (NLE model), screen lane above audio lane | 405×228 | The audio lane keeps going when the screen goes dark — liveness is legible on the timeline itself | 172px of height on every screen = one grid row of visibility |
| 04 | **Split Browser** | Grid + persistent 360px detail pane (Photos/Mail); time-bucketed sticky sections | 285×160 | The preview never moves — Library and Timeline use the same split | One grid column, permanently: 3-up instead of 4-up, 33% fewer candidates per screen |
| 05 | **Terminal Native** | No sidebar; chrome collapses into a mono status line top and bottom | 405×228 | The readout bar — an honest always-on report of what recording costs you right now | Mono is ~13% wider per character, and `cpu 3.8%` means nothing to a non-technical user on day one |

## How to review these (round 1)

Compare on the things the redesign is actually for, in this order:

1. **Frame 01** in all five, side by side. Is the grid legible? Would you find a moment in it?
2. **Frame 07** (recording chrome, nine states). Recording is the product; if its states are
   muddy, the direction fails regardless of how the grid looks.
3. **Frame 08** (Ask). Can you tell in one second that AI is subordinate? If not, that
   direction lost the core ask.
4. **Frame 13** (error gallery). Errors are visible without blocking — four placements, one
   rule for choosing between them, and everything also lands in the notification archive.
5. **Frame 14** (component appendix). This is the direction's actual system. Today the app has
   `.btn` re-declared in six files and no shared Button at all — the appendix is the fix.
6. **Frame 15** (scorecard). Every direction scored itself against ten checks per screen and
   audited every control for hit target, states, and keyboard name. The 3s are the honest part.

Directions are not mutually exclusive at the component level — the token block, the grid
geometry, the error taxonomy and the component appendix are shared. What you are choosing is
the **shell**: sidebar, no sidebar, permanent dock, or permanent detail pane.
