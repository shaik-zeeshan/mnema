# 01 — Bento Native

**The tile grid the founder liked stops being an Overview layout and becomes the whole app's
organizing idiom.** One cell unit, one 16px gutter, four legal footprints, and tile chrome that
is *identical everywhere* — an 18px header row (mono eyebrow left, meta right) on one baseline —
while the payload under it is completely free and may bleed past the inset to be clipped by the
tile radius. That single move is what stops a bento from reading as a form: a digest, a waveform,
a search result, a settings group and an AI answer are the same object wearing different contents.
Surfaces are grouped borderless fills (the System Settings idiom) on an opaque window, materials
only in chrome, one accent, native density — calm, and unmistakably a Mac app.

```
open docs/redesign/round4/01-bento-native/01-overview.html
```

| file | what it holds |
|---|---|
| `01-overview.html` | the bento Overview at 1100×720 and 800×600, plus the tile-anatomy and footprint system |
| `02-timeline.html` | Timeline with the readout-as-jump-control, the bento jump panel, the rail hover state, the AudioDrawer open, and the before/after of the secondary controls |
| `03-quick-access-search.html` | Quick Access Search at 1120×720 — results grid, empty state, no-match state |
| `04-quick-access-ask.html` | Quick Access Ask, then "ask about the current frame" in four states over a simulated display |
| `05-settings-general.html` | the new settings shell (toolbar tabs, top-anchored autosave) showing General and Capture |
| `06-settings-intelligence.html` | Intelligence — the custom-input showcase, plus the scrolled pane and a why-these-five table |
| `07-components.html` | every control and state, the bento footprint system, type + spacing specimen, the UI/UX self-audit, the nine-point native audit, and stated deviations |
| `shots/` | rendered verification screenshots, light and dark, for all seven pages |

## What it does with each founder ask

**Bento.** Kept, and promoted. The Overview is closest to the liked frame 04 — moments strip,
digest with frame citations, capture state, conversations (never minutes), context, subjects, an
Ask launcher — but the grid is now specified rather than drawn: `--cell-gutter: 16`,
`--tile-pad: 14`, `--tile-r: 12` with a *derived* inner radius of 6, and exactly four footprints
(1×1, 2×1, 2×2, 4×1). Search results, the Ask answer, and every settings group are laid out on
that same grid, so the app has one spatial rhythm instead of five.

**Timeline navigation.** The shipping layout is untouched — thin bar → stage → tick rail with app
bands, newest anchored right → sibling two-row mic/sys lane → pointer-following readout →
AudioDrawer. What changed is that **the readout became the jump control**: it gained a chevron and
opens a bento jump panel (three quick targets, seven day tiles carrying their own coverage bar and
hours, then the month, with days that hold no recording rendered disabled). Coarse movement is a
**zoom segmented control** (Hour/Day/Week), never a second date field, and hovering the rail shows
a ghost playhead with the readout following the pointer before you commit. The five peer buttons
in the bar collapsed to two clusters plus one overflow menu.

**Search vs Ask.** Same window, opposite posture — and no segmented control, because a segmented
control makes them peers fighting over one field. Search is quiet: neutral field, mono match count,
scope chips, one homogeneous grid of result tiles, and a bottom bar that always says what ⏎ will
do. Ask is the only accent-filled field in the app, the second row switches from *scope* to
*context* (what the model is being handed), the active model is a chrome pill, and the answer is a
heterogeneous tile composition rather than a grid. Ask is reachable from Search as the **first
result row**, ranked and dismissible.

**Ask about the current frame.** The 1120×720 palette collapses to a 560×40 control pill pinned
top-centre with a 620px panel 12px below it — a persistent pill so hiding the answer never hides
the stop control. What the model can see is outlined **on the screen itself** (dashed accent frame,
corner ticks, one label naming the freeze time) instead of being described in the panel, and the
captured frame is attached as a 26×16 **chip inside the prompt sentence**, so it costs no panel
width and you delete it the way you delete a word. The answering state grows downward from the same
anchor, with a mode tab overhanging the corner, and cites your own stored frames alongside the live
screenshot — the thing no screen-reading assistant can do.

**Settings navigation.** The rail is gone. Five toolbar tabs carry the top level; a sticky sub-bar
holds scoped ⌘F search. Groups are bento tiles in two columns.

**Autosave.** In that sticky sub-bar, at the *top* of the window, where a short window cannot clip
it — paired with a row-level "Saved" echo that fades after ~1.5s so you know *which* setting saved.
Failure is the only state that persists, and it also raises a toast. There is no save bar.

**Custom inputs.** Five, chosen by one rule: a custom input earns its weight only when the stored
value and the value you care about are different units. Frame-rate slider → `2 fps → ~1.4 GB/day ·
~42 GB/month`; retention ladder → today's 34.2 GB and the projected 92 GB on the same axis; model
rows → size · quantisation · RAM · **fit verdict on this machine**; OCR duty cycle → both halves of
the cycle as one split bar with the temperature it costs; shortcut recorder → real keycaps, a live
capture state, and the conflicting owner named inline. Everything else stays a switch, a popup or a
segmented control with a present-tense description line.

## Deviations from the brief

- **The AudioDrawer floats above the rail-wrap** (anchored 132px up) instead of covering it, so you
  can still scrub while listening. Same drawer, same anatomy, same order.
- **Retention is shown under Capture**, not Data — it is the second half of the frame-rate decision,
  and putting the cost and the keep-window on one pane is worth the IA move.
- Page 02 adds a standalone rail-wrap specimen and a before/after of the secondary controls, and
  pages 03/06 add a state or a table beyond the minimum, because the asks ("improved", "clearly
  different", "restrained") are comparative and need the comparison rendered.
- Tile radius 12 and tile inset 14 depart from the converged 10/16; both are justified in
  `07-components.html` under *Deviations, stated*.
