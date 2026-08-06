# 04 — Command Deck

**The idea in five lines**

1. Mnema is driven from the keyboard, and the interface says so out loud: a **keycap is a
   first-class visual element** on every actionable thing — switcher, tile header, menu row,
   settings row, result row, footer.
2. Every window carries a **deck**: a 28px bar fixed inside the window frame, context on the
   left, live shortcut hints on the right. It is the direction's one added piece of chrome, and
   it is why the settings save-state can never clip off-screen.
3. **Quick Access is the centrepiece.** One field, no mode toggle: you type once and *Ask AI* is
   the **top-ranked row** (⌘⏎ takes it). Taking it transforms the surface — accent header band,
   an `ASK` token pinned in the field, a single reading column — so you always know which mode
   you're in.
4. The main window is minimal chrome around content: two 28–38px bars and nothing else. Tiles,
   rails and rows do the talking.
5. Settings has **no sidebar**: five horizontal tabs over one continuous scroll with sticky
   section headers, and **⌘F is the real navigation** — every matching row in every section comes
   to you with its breadcrumb and its live control.

## Pages

| file | what it shows |
|---|---|
| `01-overview.html` | Bento Overview at 1100×720 and 800×600; every tile carries its jump-to key |
| `02-timeline.html` | Timeline with the `Now ▾` position pill + jump menu, zoom levels, AudioDrawer open |
| `03-quick-access-search.html` | Search results grid, empty and no-match — with the Ask row as top hit |
| `04-quick-access-ask.html` | Ask mode, and the full “ask about this screen” anatomy (collapse, frame attached, answering) |
| `05-settings-general.html` | The settings shell: tabs + ⌘F filter navigation, deck autosave, General & Capture |
| `06-settings-intelligence.html` | Intelligence — the six custom inputs, each with a consequence denominator |
| `07-components.html` | Component sheet, type/spacing specimen, per-page UI/UX self-audit, deviations |
| `08-journal.html` | The day as a keyboard-navigable river (bands, compact rows, away gaps, the live edge) + the receipt as a transport — scrub ticks per cited frame, filmstrip, transcript, ␣/←→/esc |
| `09-subjects.html` | Tiers by conviction with the sparkline as the row's hero, one row expanded to its conclusions and evidence, plus the subject detail: hero counts, conclusion strip, the story-over-time spine |
| `10-context.html` | The two kinds of knowing — standing statements you write on the left, the engine's counted side on the right — plus the dismissal archive and a wipe sheet that names everything it clears |
| `11-settings-complete.html` | The census: all 96 indexed rows across the five ⌃-tabs, every group drawn, conditional rows badged, and the five places a G-decision outranked pages 05/06 |

Each page is self-contained (inline CSS, inline SVG), designed in both themes, and responds to
`prefers-color-scheme` **and** the `[data-theme]` toggle top-right. Renders of every page in both
themes are in `shots/` (uncommitted, like `shots-app/` — artifacts, not deliverables).

**IA for 08–10.** The main window still has exactly two surfaces, Timeline ⌘1 and Overview ⌘2.
Journal, Subjects and Context are **destinations opened from Overview** — the tile carries the key
(⌃D, ⌃J, ⌃K), the title bar grows one breadcrumb chip, and `esc` returns. Each page draws its own
Overview inset as the way in. There is no Insights rail and no Chat; asking happens in Quick Access.

## What it does with each founder ask

**Bento Overview — kept, and given a job.** Same heterogeneous tile grid: moments strip as real
frames, digest with frame citations, conversations (not minutes), capture, storage, subjects,
context, ask launcher. What's new is that **every tile header carries the key that opens it**
(⌃M moments, ⌃C conversations, ⌃K context…), so the grid doubles as the app's shortcut map —
hold ⌃ and the badges raise contrast. At 800×600 This Week and the ask history drop; the `6:42`
hero stays.

**Timeline navigation — a position pill, not a date picker.** The layout is untouched (thin bar →
stage → tick rail, newest right → two-row mic/sys lane → readout → AudioDrawer, drawn open). Two
improvements: a **`13:07 · Mon, Aug 3 ▾` pill anchored to the playhead** that reads out position
*and* opens the jump menu — one control, two jobs, never in a different place from the thing it
reads; and **coarse movement as a zoom level** (Hour/Day/Week, ⌥1–⌥3) so the jump menu only has
to answer “when”. The menu is real NSMenu anatomy: each day carries its captured hours in the key
column, **days with no recording are disabled**, and “type a date” is a live field inside the
menu. Secondary rail controls (engine chip, rerun, text count, refresh) keep their shipping jobs
and gain keys; hovering the rail shows a ghost playhead and a time bubble before you commit.

**Search vs Ask — different answers to different questions, not two tabs.** Search is literal,
instant, media: a 3-up grid of real frames in day sections with match boxes on the image. Ask is
interpreted, costs a model call, and renders as prose with citations on a surface with its own
identity: accent-filled header band with an accent underline, an `ASK` token inside the field
(⌫ deletes it and you're back in search), one 66ch reading column, cited moments as a 280px media
rail, the active model as chrome. The bridge is ⌘⏎ from anywhere — and when search returns
nothing, the ask row is promoted to selection and its key becomes plain ⏎.

**“Ask about the current frame” — ⌘⇧A.** The whole window collapses to a 640px floating bar with
a detached control pill above it (dismiss the answer, keep the controls). The frame is captured
**at the instant of the keystroke, before the bar draws**, so Mnema is never in its own shot;
capture is implicit but the indicator is explicit — a green outline with corner marks and a tag on
the **display itself**, plus one muted line and a 42px thumb in the bar (“Seeing your screen ·
Slack — #mnema-dev · 15:41:02”). The answer arrives in a second panel 12px below, cites *“this
screen · 15:41:02”* alongside moments from history, and offers ⌘⌫ to drop the frame, ⌘O to grow
back into the full window.

**Settings navigation + autosave.** The rail is gone: five tabs (⌃1–⌃5) over one scroll with
sticky section headers, and ⌘F turns the pane into a ranked list of matching rows, each with its
section breadcrumb and its real control — you change the setting from the search result rather
than navigating to it. Rows are keyboard rows with full-row accent selection. Autosave shows in
**two places, neither of them a save bar**: a row-level “Saved ✓” beside the control you touched
(locality — it says *what* saved) and a persistent, timestamped state in the deck, which is a
fixed bar inside the window frame and therefore cannot clip at any window size. Failure turns the
same deck slot red and names the cause.

**Custom inputs — six, each with a denominator.** Cost slider (`2 fps → ≈1.4 GB/day · ≈42 GB/mo`),
retention ladder (segmented `7d·30d·90d·1y·∞` with the projection *and* your actual footprint on
one axis), OCR duty-cycle bar (both halves drawn, thermal outcome in prose), model rows with
`size · quant · fit verdict` computed against this Mac's RAM (a red verdict disables *Use*),
shortcut recorder with keycap badges and an inline conflict naming its owner, and consequence-
preview toggle rows whose helper line states the present tense. Everything else stays stock
AppKit on purpose.

## Deviations from the brief

- **The “ask about this screen” overlay is a 640px bar over the desktop, not the 1120×720
  window.** It is the same Quick Access window collapsing (same conversation; ⌘O restores it),
  which is the point of the feature — but it is the one place a rendered surface is not one of
  the two documented window sizes.
- **A 42×26 thumbnail sits beside the context label** in the HUD, where the research says resist
  the thumbnail. Justification on page 07: at 6% of the bar's width it answers “which display?”
  on a multi-display Mac, which a text label cannot.
- **No Search/Ask segmented control in Quick Access.** The brief's architecture rule (both live
  in one Quick Access window) is held; the segmented control that design.html frame 08/09 used is
  replaced by the ranked-row model. Full reasoning in 03 and 07.
- **The page masthead uses 26px**, outside the six-role type ramp. Masthead is review furniture
  outside the app window; nothing inside a rendered window leaves the ramp.
- Settings section keys are **⌃1–⌃5** (not ⌘1–⌘5) so they cannot collide with the ⌘1/⌘2 surface
  switcher, which stays global.
- **Page 11 draws each settings tab at its natural height**, not at 1100×720. It is a census, not a
  viewport study — pages 05 and 06 already show the real viewport. The chrome (tab strip, deck) is
  real; the height is the whole scroll unrolled so nothing can hide below a fold.
- **Page 09 does not copy four shipping behaviours** (the always-accent category dot, `0.82` vs
  `82%`, the index-spaced sparkline, the silent five-chip evidence cap) and **page 10 does not draw
  the "Steering your dossier" rail**. Each is argued on its own page and listed in `07-components.html`.
- **Page 10 corrects page 01's Context tile.** "142 facts about you" names an entity the backend does
  not have; the real counts are Activities, Conclusions, Subjects and standing statements.
