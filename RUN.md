# Direction 05 — Tactile Instruments

One of five whole-app redesign candidates. This branch **never merges to main**;
it exists so you can *use* the direction as a real app before picking one.

Spec: `docs/redesign/round4/05-tactile-instruments/` (README + 7 mockup pages).
Binding amendments: `docs/redesign/round4/DECISIONS.md` (G1–G11) — where a
G-decision and the mockup disagree, the G-decision won. See "Deviations" below.

## The idea in one paragraph

System-Settings bones everywhere — quiet, flat, native — with one exception:
where a value has a **physical** consequence it gets an **instrument**: a
machined face with a header (name + live value), a recessed **well** (the
control), and a **readout** that states the cost in your units, with a
denominator. There are **six instruments in the whole app** plus two read-only
readouts on Overview. The contrast between the quiet rows and the occasional
instrument *is* the design. Depth is a surface step — a fill, a recess, or a
shadow on something that genuinely floats — never a container border.

**The rule an instrument must pass:** name the physical quantity it changes, and
write the consequence as a fraction of something real. *"2 fps → ~2.1 GB/day of
214 GB free"* passes. *"Tell the model what time it is"* does not — that is a
switch.

## Launch

```
bash scripts/prepare-mnema-cli-sidecar.sh debug     # once, on a clean checkout
bun install                                          # once
bun run tauri -- dev
```

For UI-only iteration (no Rust, much faster), the plain web dev server is enough:

```
bun --cwd=apps/desktop run dev
```

## What to look at, per surface

**Settings (⌘,) — the direction's densest statement.**
The **left rail is gone**. Top-level sections are a native toolbar of icon+label
tabs which also carries the scoped search field and the autosave chip — the one
strip pinned at every window size, so neither can clip. Inside a tab there is one
660px scrolling column with **opaque sticky group headers**: navigation is
horizontal at the top and vertical in the content, never both at once. **Scroll
the pane** to see the header pin. Groups are fills with hairline separators; the
whole window has exactly one bordered container — the window.

Find the instruments, and notice how few there are:

| where | instrument | the consequence it states |
|---|---|---|
| Capture › Video | capture rate | GB/day against the free space on this Mac, with your 7-day average as the notch |
| Capture › Capture | segment length | what an unexpected quit costs you; the 5-min cap drawn as a disabled stop |
| Data › Storage | retention ladder | what survives and what is culled, on a time axis — old left, kept right |
| General › Shortcuts | shortcut recorder | real keycaps in a well; at rest / armed / conflicting |
| Intelligence › OCR | duty cycle | both halves of the cycle, in frames and backlog — and **no temperature** |
| Intelligence › models | model picker | size · RAM · a fit verdict computed against this Mac |

Everything else on those pages is a stock row. That is the point.

**On a fresh machine the instruments deliberately show no numbers.** Mnema has
not measured a capture day yet, so instead of a plausible-looking guess you get a
sentence saying so. Record for a day and the denominators appear. This is G8
working, not a bug.

**Timeline (⌘1).** Layout is frozen — stage → rail → audio lane → readout →
drawer. Look at the restyle: the readout clock is the loudest number on the
surface, the rail is the only saturated object, recording red appears only on the
active tick and the selected audio bar. Open the **position pill** for the jump
menu: every day carries its own coverage bar, and days with nothing recorded are
disabled, so you can never land on an empty day. Hover the rail for the ghost
playhead and its time bubble.

**Overview (⌘2).** The bento. Tiles are fills, media leads the grid, exactly one
display-size number on the page. Two read-only readouts: the 24-hour coverage
strip on Capture, and the day-budget gauge on Storage. **On Overview an
instrument reads — it never turns; you turn it in Settings.**

**Quick Access (⌘⌥Space).** One field, no Search/Ask segmented control. Search
returns *things* — monochrome chrome, a 3-up grid of real frames, and a match
meter that decomposes the count instead of printing it. "Ask Mnema about …" is a
ranked row in the results; selecting it flips to Ask mode, whose one identity is
the accent-tinted field. The no-match state prints what was actually searched.

**Ask about the current frame (⌘⇧⏎).** The same window collapses to the bar —
never a second window. Context is a chip inside the sentence, so you edit it by
editing the sentence. The answer is a detached second piece, so dismissing it
never hides the controls.

**Both themes.** Every surface is drawn in light and dark; the light theme is
where the "System Settings bones" claim is easiest to judge.

## Verifying a change

Render it and look at it — do not grep for class names.

```
nohup bun --cwd=apps/desktop run dev -- --port 1425 --strictPort false > /tmp/vite-05.log 2>&1 & disown
PORT=1425 bun scripts/rd4-shot.mjs /settings dark 1100x720 /tmp/shots
```

`scripts/rd4-shot.mjs <route> <light|dark> <WxH> <outDir>` stubs the Tauri IPC,
carries a timeline/search fixture, and writes a PNG. Stub toasts are hidden by
default; `SHOW_TOASTS=1` brings them back.

## Deviations from the mockup, and what forced each

- **No temperature anywhere**, and no minute-precise ETAs — durations round
  coarsely ("about 3 weeks"). *G8.*
- **Every denominator is read from the machine**, never the mockup's figures
  (214 GB free, 2.1 GB/day, 1,284,406 frames). A fact that is null renders no
  number at all. *G8.*
- **No type-a-date field** on the timeline. *G6.*
- **No Search/Ask segmented control** — ask is a ranked row. *G4.*
- **No bottom save bar and no settings Undo**, though the mockups draw an Undo
  affordance. *G7.*
- **The semantic coverage meter renders only when semantic search is enabled**;
  the off state states the price first. *G10.*
- **Open Threads is digest prose**, not a structured tile. *G11.*
- **Shortcut-conflict copy never names an external app** ("taken by another app
  — try a different combination"); in-app conflicts do name their owner. *G9.*
- **Retention lives under Data › Storage** where the app already had it, rather
  than being moved under Capture › Privacy as the mockup's own deviations list
  proposed — moving a settings row is a behaviour change, and this phase skins.
