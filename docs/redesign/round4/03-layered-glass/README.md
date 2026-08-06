# 03 — Layered Glass

**The idea, in five lines.**
Depth comes from *layering*, not from edges: things float over the thing below them with a soft
shadow and a material, and a border is only ever the rim of a piece of glass. Material is allowed
exactly where macOS itself uses it — toolbars, rails, HUDs, popovers, overlays — and **content
always lands on an opaque plate**, so no paragraph's contrast ever depends on the wallpaper behind
it. Quick Access stops pretending to be a second app window and becomes a real HUD floating over a
dimmed desktop. The Overview bento reads as Sonoma widgets: identical plate chrome, a subject-tinted
glow from the top edge, and a free payload zone that may bleed off the tile and be clipped by its
radius. Everything else stays plain AppKit, on purpose.

Open any file directly — each page is self-contained (`file://`), both themes, toggle top-right.

| file | what it shows |
|---|---|
| `01-overview.html` | the bento Overview at 1100×720 and 800×600, plus the widget-footprint rhythm |
| `02-timeline.html` | Timeline with the AudioDrawer open; the jump menu open; the jump control's four readings |
| `03-quick-access-search.html` | Quick Access in Search mode over a live desktop, plus empty and no-match |
| `04-quick-access-ask.html` | Ask mode, then the current-frame feature: collapse → attach → answer, plus the bar's four states |
| `05-settings-general.html` | the new settings shell (floating rail, sticky headers) on General and Capture; autosave in four states |
| `06-settings-intelligence.html` | Providers/Ask AI and Transcription/Speakers, plus the five custom inputs as a gallery |
| `07-components.html` | the material ladder, every control and state, the state pill's nine states, type + spacing specimens, the per-page self-audit |
| `08-journal.html` | the day as a river of plates — the read, the four real statistics, banded activities, the away gap as the one row with *no* plate; the receipt open as a glass sheet, plus its other three viewports |
| `09-subjects.html` | tiers by conviction with the sparkline as the row's hero; one subject's detail — conclusion strip, the pinned belief, and the trajectory track whose spine position *is* the confidence |
| `10-context.html` | authored context beside the dossier it steers; the composer, the standing list, the dismissed archive, and the deletion sentence stated exactly as true as the backend is |
| `11-settings-complete.html` | settings whole — five groups, twenty-two sections, all 96 indexed rows plus the shortcut editor and the MCP connectors; the shell, ⌘F filtering, and each group's pane unrolled |
| `shots/` | rendered PNGs of all eleven pages in both themes — **generated locally, not committed** (`docs/redesign/round4/*/shots/` is excluded in `.git/info/exclude`). Regenerate with a headless chromium at 1280×800, full page, one shot per theme. |

## What it does with each founder ask

**Bento (kept, re-skinned as widgets).** Same tile-grid concept — digest, capture state,
conversations, context, moments strip — now on Apple's widget discipline: one cell unit, one 16px
gutter, and only four legal footprints (4×1, 2×2, 2×1, 1×1). Every tile is the same opaque plate
with the same 14px radius and the same shadow; only the *tint* changes, pulled from the tile's
subject (recording red for Capture, mic green for Conversations, accent for Subjects). The moments
strip is a padding-zero payload zone whose frames run off the tile edge and get clipped by its
radius — the single move that stops a bento from reading as a form. At 800×600 the grid genuinely
scrolls under the material toolbar and the 6:42 hero survives; This week and Storage drop.

**Timeline navigation (polished, not reinvented).** The layout is untouched: chrome → stage → tick
rail with newest anchored right → sibling two-row mic/sys lane → readout → AudioDrawer (drawn open
on 02). The two improvements the founder asked for: the **readout and the date picker become one
glass capsule anchored over the playhead** (Rewind's idiom — the answer to "where am I" and the
control for "take me somewhere else" are finally in the same place), whose chevron opens a jump menu
with recent days carrying their coverage as bars, a **day with no recording rendered disabled**, and
a month grid that dots the days that have frames. Coarse movement moves to a Hour/Day/Week zoom
segmented, so the capsule only ever handles "jump". The loose thin-bar buttons become one labelled
utility cluster (`Text 42` · `Re-run OCR` · refresh) with a busy state, and hovering the rail shows
a ghost playhead plus a time bubble before the click commits.

**Search vs Ask.** One window, two genuinely different surfaces. Search: graphite mode chip
overhanging the panel's top-right like a tab, an oversized query line, scope chips, and a 3-up grid
of opaque frame plates (349×196) — the material never touches a result. Ask: accent chip, accent
field rule, one reading column ≤70ch with a 238px cited-moments rail, and a composer at the *bottom*
like a conversation instead of a query field at the top. Bridging them, the Spotlight move: **"Ask
Mnema about 'webhook'" is the first row of the search results**, so the modes are one keystroke
apart without being one surface.

**Ask about the current frame (new).** `⌘⇧↵` collapses the whole 1120×720 HUD to a **720×48 glass
bar** at the top of the display and drops the dim — the point is to see your screen. The frame is
captured implicitly (collapsing *is* the gesture — no "attach screen?" dialog) and the indicator is
explicit and doubled: a 44×26 thumbnail plus `Screen · 14:32:08` in the bar, and an **outline drawn
on the screen itself** with the tag "Mnema is reading this screen", so "what can it see" is answered
where the answer lives. Privacy-excluded apps are named, not silently dropped. The answer arrives as
a **second, detached glass panel** 12px below the bar (Cluely's split): mode chip overhanging its
corner, one muted context line, prose on an opaque plate, dot-separated quick-action chips, and a
footer carrying the model, Stop, Dismiss and `⌘O` back into the full window. Dismiss the answer and
the bar survives; `⌥↵` asks without the frame.

**Settings navigation + autosave.** The "weird sidebar" becomes a **floating translucent rail** —
inset, rounded, shadowed, with a `⌘F` search field at the top, four labelled groups, and the live
cost of what you enabled pinned to its bottom (`Recording · all three sources / 270 MB today /
34.2 GB kept`, the LM Studio move). Under it is one opaque scroll pane with **sticky section
headers**. Autosave never touches the bottom of the window: a chip in the title bar (idle / saving /
saved / failed-with-retry) plus a **row-level "Saved" echo** in the row you actually changed, so
locality tells you *what* saved. Only failure also raises a toast, and it never auto-dismisses.

**Custom inputs (five, restrained).** A bespoke control only where a value has a physical
consequence a stock control cannot state: (1) the capture-rate slider — `2 fps → ≈ 1.4 GB/day ·
≈ 42 GB/month · of 494 GB free`; (2) the retention ladder, which marks the 34.2 GB you *already
have* on the same axis as the 126 GB you are choosing to keep; (3) the OCR duty cycle as a two-ended
split bar for both recording and paused, with the thermal consequence written beneath in prose;
(4) the shortcut recorder as keycaps, with the conflict named — with its owner — in reserved space
under the field; (5) the semantic-search budget, priced in megabytes of vectors against the index it
covers. Every model row carries `size · where it runs · a verdict computed against this Mac`
(green / amber / red / cloud-blue) — the verdict, not the number, is the control's real output.
Everything else stays a switch, a pop-up button or a segmented control.

## Deviations from the brief

- **Frames are rendered inside a simulated desktop on 03 and 04.** The Quick Access window itself is
  still exactly 1120×720 and the ask bar exactly 720×48; the desktop canvas around them (1360 wide)
  exists because a HUD that is not shown floating over something is not a HUD.
- **The current-frame preview is a 44×26 thumbnail, not a large one.** The inspiration explicitly
  warns against a big attached-frame thumbnail; the founder asked for the frame to be "previewed as
  attached context". This splits the difference: a small chip in the bar plus the full-size outline
  drawn on the screen itself.
- **The pointer-following readout now tracks the playhead's x position** instead of sitting at a
  fixed left edge. Same row, same height, same information — this is the "hour/readout affordance"
  polish the brief invited, but it is the one place the frozen surface visibly changes.
- **The retention ladder is drawn in the instrument gallery on 06, not inside a settings window.**
  It belongs to Data › Storage, which neither 05 nor 06 renders as a pane; showing it in the gallery
  keeps the nav semantics honest.
- ~~**`shots/` is committed** alongside the pages~~ — **not true, corrected 2026-08-06.** The repo
  excludes `docs/redesign/round4/*/shots/`, so the PNGs are a local render artefact. Open the pages
  directly (`file://`) to review; every page is self-contained and carries both themes.

### Added by the destination pages (08–11)

- **Journal, Subjects and Context are destinations, not surfaces.** The main window still holds
  exactly two surfaces (Timeline + Overview). Each destination replaces the pane under the same title
  bar and its only way back is the **‹ Overview** button in the chrome — the idiom Settings already
  used. Each of 08/09/10 opens with an inset of the Overview widget that is its door. The dissolved
  Insights rail is never drawn.
- **The away gap is the one row in the app with no plate.** Everything else in this direction floats
  on an opaque plate; a gap in the day is drawn as a hairline outline with nothing behind it, because
  a hole in the layer stack *is* what "no capture" means.
- **The receipt is a glass sheet whose every content region is a plate.** A sheet floats, so material
  is legal on it; the frame stage, the transcript and the filmstrip are still opaque, which is the
  direction's whole rule stated in one component.
- **Context's side column looks like a rail and deliberately is not one.** In this direction a rail
  is the thing that wears material. That column carries explanation and beliefs — content — so it is
  plates like everything else.
- **11's rail corrects 05's.** 05 drew a re-grouped eleven-item rail; the app has five groups and
  twenty-two sections, and twenty-seven rows will not fit 208px at native density. The rail now lists
  the five groups and the active one discloses its sections — the pane's sticky headers already carry
  the same names. The floating glass, the ⌘F field and the pinned cost footer are unchanged.
- **The four shortcut-editor cards on 11 are bare hairlines, not plates** — a plate inside a plate is
  the box-in-box this direction refuses. They are the reason 11's border count is 6 rather than 2.
- **The unrolled panes on 11 are the same pane at its natural height,** not extra windows. Drawing
  Intelligence's 45 rows inside a 720px frame would have hidden most of what the page exists to prove.
- **Category, focus and chart-grey tokens are lifted verbatim** from `routes/+layout.svelte` rather
  than re-invented, so the mockup's swatches are the app's actual `--cat-*` / `--focus-*` values.
