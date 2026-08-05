# 05 — Tactile Instruments

**The idea, in five lines.**
Mnema spends your disk, your CPU and your battery, and almost every screen in it is quiet about
that. This direction is System-Settings bones everywhere, with one exception: where a value has a
**physical** consequence, it gets an *instrument* — a small, machined face with a recessed well and
a readout that states the cost in your units, with a denominator. There are **six instruments in
the whole app**, and everything else is a stock native row. The contrast between the quiet rows and
the occasional instrument *is* the design; add a seventh casually and the direction dies.

**The rule an instrument has to pass:** can you name the physical quantity it changes, and can you
write the consequence as a fraction of something real? "2 fps → ~2.1 GB/day of a 3.0 GB budget,
214 GB free" passes. "Tell the model what time it is" does not — that is a switch, and page 06
draws it both ways so the cost of getting this wrong is visible.

**The six.** Capture rate (fps → GB/day against free space) · retention ladder (what survives, what
is culled, on a time axis) · segment length (how much a crash costs you) · OCR duty cycle (both
halves of the cycle, in frames/min and °C) · shortcut recorder (real keycaps, conflicts named) ·
model picker (size · RAM · a fit verdict computed against this Mac). Two more *readouts* — the
24-hour coverage strip and the day-budget gauge — appear on Overview, where an instrument only ever
reads and never turns.

## Files

| file | what is in it |
|---|---|
| `01-overview.html` | The bento at 1100×720 and 800×600, with two instrument-grade readouts |
| `02-timeline.html` | Timeline with the new date control + jump menu, polished rail controls, AudioDrawer open |
| `03-quick-access-search.html` | Quick Access in Search mode (results grid) + the no-match state |
| `04-quick-access-ask.html` | Ask mode, then "ask about the current frame" in three states + the anatomy |
| `05-settings-general.html` | The new settings shell (toolbar tabs, one scroll, always-visible save) — General + Capture |
| `06-settings-intelligence.html` | Intelligence — the custom-input showcase, and the restraint budget stated |
| `07-components.html` | Component sheet (every control and every instrument state), type + spacing specimen, UI/UX self-audit |
| `shots/` | Rendered proof: all seven pages, light and dark |

## What it does with each founder ask

**Bento (kept).** The tile grid, the moments strip, the digest with frame citations, the
Conversations tile and the Context tile are all intact. Two tiles gain an instrument *face*: Capture
shows a 24-hour coverage strip (which hours actually hold recording), Storage shows the day-budget
gauge with a 7-day-average notch and free space as the denominator. Neither is interactive — on
Overview an instrument reads; you turn it in Settings. At 800×600 the hero and the gauge bar stay;
the tick labels are what drop.

**Timeline navigation.** The layout is untouched — thin bar → stage → tick rail with app bands
newest-right → the sibling two-row mic/sys lane → pointer-following readout → AudioDrawer (shown
open). The date control is now one pill that both reads out your position and opens a jump menu
where **every day carries its own coverage bar and captured hours, and days with nothing recorded
are disabled** — you can never land on an empty day. Coarse movement is a Hour/Day/Week zoom, so
there is no second date input and no from/to pair; the rail gains a span readout (`8 px/frame ·
4h 12m visible`), a hover ghost-playhead with a time bubble, and a rerun control that shows its
progress in place without changing width.

**Search vs Ask.** Search returns *things*: monochrome chrome, scope chips, temporal section
headers, a 3-up grid of real frames, and a match meter that decomposes "14" into 9 screen / 3 audio
/ 2 documents. Ask returns *a claim*: an accent-tinted field with its own glyph, one prose column at
68ch, cited moments as evidence in a right rail, a live tool trace instead of a spinner, and a
follow-up composer. The bridge is Spotlight's: **"Ask Mnema about this" is the promoted first row of
the search results**, one ↓↵ away, as well as an explicit mode switch. The empty state is where
Search earns its trust — it prints what was actually searched (1,284,406 frames, 98.2 % with text,
412 h of 419 h of audio) rather than shrugging.

**Ask about the current frame.** ⌘⇧⏎ drops the 1120×720 window and leaves two *detached* floating
pieces top-centre: a control pill that survives Hide, and a panel below it. The frame is grabbed at
the instant of the collapse — no modal, no "attach screen?" — and the fact is stated twice: the pill
reads *Seeing your screen*, and the captured region is **outlined on the screen itself**. Attached
context is one small chip inside the composed sentence (the same cite chip the digest uses), never a
full-width thumbnail, so you edit context by editing the sentence. The one instrument here is the
freshness readout — `captured 0.4 s ago · 2560×1440 · 132 words of text` — plus its stale-frame
state at 46 s with a re-grab. The answer opens in the same panel under the same pill, so hiding the
answer never hides the controls.

**Settings navigation.** The left rail is gone. Top-level sections are a native toolbar of
icon+label tabs (General · Capture · Intelligence · Data · About) with a scoped search field; inside
a tab there is one 660px scrolling column with **opaque sticky group headers**. Navigation is
horizontal at the top and vertical in the content, never both at once. Both settings pages are
rendered genuinely mid-scroll so the pinned header is real, not asserted.

**Autosave.** Two placements, neither of which can be clipped: a chip in the *toolbar* — the strip
that is pinned to the top of the window at every size — and a row-level "✓ Saved" echo in the row
that changed, with the row tinted for ~1.5 s. Four states are drawn (idle with a timestamp, saving,
saved-with-Undo ⌘Z, failed-with-Retry). There is no Save button and no bottom bar anywhere in this
direction, so there is nothing to fall off a short window.

**Custom inputs.** Concentrated on 06 and counted out loud: twelve settings visible across two
windows, three of them instruments, nine of them stock switches, popups and rows. Page 06 ends by
drawing the same setting as a switch and as an instrument side by side, labelled *right* and
*wrong*, so the budget is enforceable by review rather than by taste.

## Deviations from the brief

- **`system.css` tokens are expressed with CSS `light-dark()`** rather than duplicated dark/light
  blocks. One declaration then answers `prefers-color-scheme` *and* the `[data-theme]` override on
  `<html>` (the override wins by flipping `color-scheme`). Values are unchanged from `system.css`.
- **Fonts are the `-apple-system` stack**, per the page-mechanics instruction, not `system.css`'s
  Hanken Grotesk / Spline Sans Mono — no external assets are allowed in a self-contained file.
- **The fake screenshots and the icon sprite are lifted verbatim** from
  `../mockups/design.html` rather than redrawn, plus fourteen added Lucide-style symbols
  (gear, disk, cpu, key, frame, send, warn, undo, refresh, sliders, trash, eye, plus, chevron-down).
- **Page 02 renders two windows** (drawer open; jump menu open) and pages 05/06 render two each,
  because one frame cannot show a scrolled sticky header *and* the section it scrolled past.
- **Retention lives under Capture › Privacy**, matching `design.html` frame 12, not under Data —
  the brief's section list puts Storage in Data, but the ladder belongs beside the thing it culls.
- **No microphone input-level meter.** It is the most tempting seventh instrument and it was cut on
  purpose: a live level is a readout with no setting attached, and the seed's warning about a
  themepark is a harder constraint than the completeness of the list.
