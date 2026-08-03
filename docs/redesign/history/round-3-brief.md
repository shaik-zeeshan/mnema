# Mnema redesign — round 3 brief

Round 2 (`06`–`10`) was rejected whole: *"don't like any of them… make sure they feel like a
native app."* The five round-2 directions were **graphic-design systems** — paper tone, hairline
rules, scrims, tone seams, pure typography. Competently executed, and none of them reads as a
Mac app. They read as designed documents rendered inside a window.

Round 3 ships `mockups/11`–`15`. Each direction is built on a **different native macOS idiom** —
a family of real system apps a Mac user already knows. The question each file answers is not
"what replaces the boxes" but "**which kind of Mac app is Mnema?**"

Everything settled stays settled:

- **Architecture** — `BRIEF-2.md` §2, binding and unchanged. Two surfaces (Timeline · Overview)
  in a switcher, user picks the main one; Search + Ask live in Quick Look (1120×720); same
  window set as ships; Settings is an in-main route.
- **The timeline is frozen** — `REVISIONS-2.md` R6, verbatim. Render
  `apps/desktop/src/routes/+page.svelte` (markup from line 6043) exactly as it ships, re-typed
  and re-spaced onto the shared roles only. **Your shell may not narrow it**: the timeline gets
  the full window width it has today. If your idiom has a sidebar, frames 01–03 show it
  collapsed (native full-height split-view collapse) or absent — say which.
- **`system.css` is the source of truth** — copy §1–§7 verbatim at the top of your `<style>`,
  consume it, list your direction's additions under `/* direction-specific */`. Objections go
  in Deviations, not silent overrides (REVISIONS-2 R7a).
- **De-boxing** — `BRIEF-2.md` §3 all eight rules still in force. Clarification for this round:
  a **single-level filled group** (the System Settings / source-list idiom — a rounded fill one
  step off the background, no border, never nested) is a *surface step* under rule 4 and is
  allowed. Stacked hairlines and bordered-inside-bordered remain banned.
- **Errors are toasts**; title bar degrades on the designed ladder; three window widths
  (800×600 / 1100×720 / 1440×900); grid geometry 3-up 349×196 in Quick Look — all per
  `REVISIONS.md`, unchanged.
- **Frame list unchanged** — `BRIEF-2.md` §4 frames 00–16 plus `REVISIONS-2.md` frames 17–19,
  then Deviations, Motion inventory, Border count. One change to frame 00: the *before* cell is
  now a **round-2 frame** (any of 06–10), and the callouts name what makes yours native where
  that one was graphic.

---

## The native bar — all five directions, non-negotiable

This is the actual founder complaint, so it is the shared spec. A frame fails the round if a
Mac user would not mistake it for a screenshot of a real macOS app.

1. **Window anatomy per HIG.** Traffic lights at their real size and position; a real title
   bar or unified toolbar at native height; content begins where AppKit would put it. No
   web-style page header inside the window.
2. **Controls are AppKit, drawn accurately.** Push buttons, segmented controls, checkboxes,
   switches, popup buttons at the metrics in `system.css` §4 — with native detailing: the
   accent-filled default button, the subtle top-light gradient on push buttons, the true
   focus ring (accent, outside the control), `:active` depress. A button that looks like a
   web pill fails.
3. **Menus are NSMenu.** Where a menu, popover or context menu appears: 13px rows, checkmark
   gutter, separators, right-aligned ⌘-shortcuts, SF-symbol-style glyphs, the 6px radius and
   material background macOS gives them.
4. **Materials where macOS uses them.** Sidebar vibrancy, toolbar blur over scrolling
   content, popover/HUD translucency — simulated honestly with layered rgba +
   `backdrop-filter`. Materials are for chrome; content sits on opaque surface.
5. **Selection is the system's.** Full-row accent selection (source lists, tables), accent
   highlight in menus. Never an underline, never a left accent bar.
6. **Native density.** 28px list rows, 24px menu rows, 13px default text (`--t-ui`). No
   marketing-page air, no 22px+ headers outside the one `--t-display` per screen.
7. **System accent colour** drives selection and primary controls, via one
   `--accent`-consuming layer, so the whole design retints the way a real app does.
8. **Overlay scrollbars**, default cursor on lists (`system.css` §7), `tabular-nums` on
   changing numbers, right-aligned shortcut columns.
9. **Nothing web-shaped.** No centered heroes, no card grids with big drop shadows as page
   structure, no full-width coloured banners.

Each file gets a short "**Native audit**" block after frame 16: the checklist above, one line
each, where in the file it is proven, and any place it deliberately deviates.

---

## Direction assignments — five kinds of Mac app

Distinct from each other **and** from 06–10: each is a shell idiom with a system-app family
behind it, not a texture. Study the family; steal its decisions.

| # | File | Direction | Idiom family | The one thing to remember | The risk to beat |
|---|---|---|---|---|---|
| 11 | `11-source-list.html` | **Source List** | Finder, Notes, Mail | A translucent source-list sidebar + unified toolbar: days and moments on the left of Timeline context, conversations and subjects on the left of Overview. The most literal "it's a Mac app". | With only two surfaces, a sidebar can become empty chrome — it must earn its width with real content nav, and collapse honestly at 800×600 and on the frozen Timeline. |
| 12 | `12-inspector.html` | **Inspector** | Final Cut Pro, Logic, QuickTime pro tools | Dark-first pro-media chrome: the footage is the document, a right-hand inspector carries metadata/OCR/transcript, precise dense controls. Mnema as the pro tool for your own footage. | Pro-tool intimidation — a non-editor opens it and feels they need a manual. The light theme must be first-class, not an afterthought. |
| 13 | `13-grouped.html` | **Grouped** | System Settings, contemporary Preferences panes | Calm single-level grouped fills organise every surface — settings-grade clarity applied to content. The quietest, most conservative direction. | Everything reads as a preferences pane — the big frame and the grid must still feel like *media*, not like a form about media. |
| 14 | `14-material.html` | **Material** | Music, Safari, Control Center | Content scrolls edge-to-edge *under* translucent chrome; toolbar and rails are material layers that blur what passes beneath; Overview modules feel like Control Center tiles. The most modern-macOS of the five. | Translucency as gimmick: legibility over blur, and honesty — say what the mockup fakes that AppKit would give for free. |
| 15 | `15-utility.html` | **Utility** | Activity Monitor, Disk Utility, Console | The data app: striped table views, a segmented-control toolbar, a status bar with live counts, mono machine columns. Closest to Mnema's calm-terminal voice — recording is monitoring, and this is the monitor. | Spreadsheet coldness — the captured frame is the point of the product and must stay the hero even inside a table-driven shell. |

Each direction states, in ≤80 words: which system apps it steals from, the one thing someone
will remember, and what it sacrifices. Honesty about the named risk, as before.

---

## Facts that keep getting wrong

Weekdays, August 2026: Sun 2 · **Mon 3 (today)** · Sat 1 · Fri 31 Jul · Thu 30 · Wed 29 ·
Tue 28 · Mon 27. Do not invent dates; derive from these.

Fake app screenshots: lift the `.shot--*` CSS from `../mockups/round-1/02-command-canvas.html`
(complete set: editor, browser, figma, chat, terminal, sheet, call, docs). Do not redraw, do
not ship grey rectangles. Known trap: percentage padding on a `.shot` child resolves against
containing-block *width*.
