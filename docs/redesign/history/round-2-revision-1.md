# Revision 1 — applies to mockups 06–10

Addendum to [`BRIEF-2.md`](round-2-brief.md). Four changes from founder review. Everything else in
`BRIEF-2.md` and `BRIEF.md` stands. **These are revisions to the existing five files, not a new
round** — each direction keeps its identity (Paper / Rules / Full Bleed / Panes / Type Led) and
its de-boxing answer.

---

## R1. The timeline goes back to the shipping model, improved — not reinvented

> "the timeline has to have one big image which shows the frame and a scrubber, just like the
> one we have. Use the one we have, but improve on it. The current one is great!"

Round 2 invented NLE lanes, density bars, hour rulers and filmstrips. Delete all of that.
**The shipping timeline is the design.** Reproduce its structure and its interaction model,
then improve the execution.

### What ships today — reproduce this structure

Read `apps/desktop/src/routes/+page.svelte` (markup starts line 6043) if anything below is
ambiguous. The shape is:

```
section.timeline                                   ← wheel anywhere scrubs
├─ header.timeline__bar                            ← thin, timeline-specific controls only
│    left:  TimelineJumper  (jump to date & time popover, + "jump to latest" when scrolled back)
│    right: [OCR provider chip] [rerun] [OCR toggle + count] [refresh]
│    NOTE: recording controls are NOT here — they live in the app title bar, deliberately,
│          so the record affordance survives a route change (comment at +page.svelte:6046).
├─ div.timeline__stage                             ← THE BIG IMAGE. flex:1. This is the product.
│    ├─ .timeline__preview      background-image, `contain`, centred
│    ├─ .timeline__ocr-overlay  positioned boxes, text chip revealed on hover
│    ├─ .timeline__ocr-status   one status line (idle/running/empty/missing/error/success)
│    ├─ .timeline__stage-actions  hover cluster, top-right: ⏻ play-this-moment (P), ⋯ menu
│    │                            (copy C · download D · copy text · open {host})
│    └─ .timeline__stage-status   transient ack/error box, bottom-right, dismissible
└─ div.timeline__rail-wrap                         ← FIXED HEIGHT, ALWAYS RENDERED
     ├─ .timeline-rail  role="slider", horizontally scrolled
     │    └─ .timeline-rail__track   width = frameCount × 8px
     │         ├─ .timeline-rail__app-group   band per contiguous app run, positioned by
     │         │                              right/width px, carries the app icon
     │         └─ .timeline-rail__slot > __tick   one 8px slot per frame;
     │                                            majors at app-group boundaries
     ├─ .timeline-rail__audio-lane-wrap          ← SIBLING of the rail, never overlaid
     │    ├─ labels: "mic" (top row) · "sys" (bottom row)
     │    └─ track: .timeline-rail__audio-bar per segment, positioned right/width,
     │              click plays it, aria-pressed, selected state
     └─ .timeline-rail__tooltip                  ← the readout: app icon + app name + time + date
                                                    follows the pointer; pins to centre when not hovering
AudioDrawer                                       ← bottom sheet, opens when an audio bar is clicked
```

### Invariants — do not break these, they are load-bearing

1. **Slot 0 is the newest frame and it is anchored to the RIGHT.** Scrolling left goes back in
   time. `index = (maxScrollLeft − scrollLeft) / 8`.
2. **8px per frame.** The rail is frame-indexed, not time-indexed. Keep it.
3. **`.timeline__rail-wrap` is always rendered at a fixed height, even with zero frames**, and
   the loading indicator lives outside the rail — specifically so that pagination, the
   empty→populated swap, and load states can never resize the stage. The codebase already
   solved the layout-shift problem the founder is complaining about in R2; honour it.
4. **The audio lane is a sibling of the rail, not inside it and not on top of it.** Two reasons
   in the source: interactive buttons must not nest inside `role="slider"`, and overlapping
   bars made click targets ambiguous.
5. **Two audio rows** — microphone on top, system audio below.
6. The rail is a real slider: keyboard, click-to-seek, wheel-to-scrub on the whole section.

### What "improve on it" means — this is your actual job

Legibility and craft, not a new model:

- **Give the stage more of the window.** The bar and the rail-wrap are chrome; the frame is the
  product. State the height you gave the stage and what you cut to get it.
- **The readout.** Today it is a tooltip with app icon, name, time, date. Make it excellent —
  it is the only thing telling the user *where in their day they are*.
- **The app-group bands.** Today they are thin bands with an icon. This is the app's best idea
  and its weakest execution — an 8px-per-frame rail with app runs is genuinely informative if
  you make the runs readable. Improve the treatment (label the wide ones, tint the run, handle
  a run one frame wide) without changing the model.
- **Time legibility.** The rail has no time labels at all today — the tooltip is the only clock.
  Adding sparse, quiet time markers along the rail is a legitimate improvement. Adding an hour
  ruler with zoom levels is not: that is the NLE model you were told to drop.
- **A skimmer/playhead distinction** is allowed *only* if it stays inside this model: hovering
  previews, the active frame is where the rail is scrolled to.
- **The audio lane** — bars are currently featureless rectangles. Make mic vs system legible,
  make the selected bar obvious, make a 3-second segment still clickable.
- **De-box it** per `BRIEF-2.md` §3, which still applies.

Frames 01, 02 and 03 are all rebuilt to this. Frame 02 (audio moment) keeps the AudioDrawer as
a **bottom sheet over the timeline**, which is what ships — not a split pane.

---

## R2. Errors are toasts. The strip is deleted.

> "They trade error handling as another row after the controls, but that shouldn't be like
> that! Changing that would make the layout shift in the whole page. It should rather be a
> toast which is in the middle or at the bottom, bottom right."

**`BRIEF.md` §8's "surface strip" placement is removed from the system.** It inserts a row
under the toolbar, which reflows everything below it — including the stage, which means the
image the user is looking at *jumps* when a background job fails. That is the defect.

The revised taxonomy is three placements:

| Placement | Use when | Behaviour |
|---|---|---|
| **Toast — bottom-right, stacked, max 3** | **The default for everything non-blocking.** Load failed, retry exhausted, export failed, audio lane unavailable, OCR failed, provider unreachable, background job failed | Overlays content — **never reflows anything**. One line + one action + close. Success auto-dismisses at 6s; **an error never auto-dismisses**. Stack grows upward; the oldest collapses to "+2 more". |
| **Inline at the control** | Input validation only, where the fix is in the field | Allowed **only in reserved space** — the field's helper row exists in the layout at rest and fills with the message — or in a popover anchored to the field. **Adding a row is forbidden.** |
| **Modal** | The app cannot continue without a decision: destructive confirm, all capture sources lost mid-session, disk full | Two buttons max, destructive on the right, focus on the safe choice. |

Plus the **notification bell as the archive** — every toast is also written there, which is what
makes a dismissible toast safe.

Rules:

- **Toasts must never cover the record control or the scrubber.** State where your toast stack
  sits relative to both, and show it in a frame with the timeline behind it.
- Bottom-right is the placement. Do not scatter toasts to three corners by severity — severity
  is carried by the toast, not by its position. If something genuinely cannot be missed, it is
  a modal, not a centre-screen toast.
- Toast enters with `transform: translateY()` + opacity, ≤250ms, per the motion budget. It does
  not push, slide, or resize anything else.
- The existing `.timeline__stage-status` box (bottom-right of the stage, dismissible) is
  already the right idea — either fold it into the toast system or say why it stays separate.
- Frame 14 (error gallery) is rebuilt: every case rendered **in situ over a real surface**, not
  as a row of specimens. Show at least one toast stack of three.
- Audit your own file: any remaining element that inserts a row into the flow to report a
  problem is a bug. Fix it and say how many you found.

---

## R3. The title bar must not be dominated by any one element

> "make it so that the [record cluster] consumes too much space on the bar"

*(Reading: something on the title bar is eating it. In every round-2 direction the capture
cluster — state + elapsed + three source glyphs + pause + stop + a cost/GB readout — is the
widest thing up there. If the founder meant a different element, the rule below still applies
to it.)*

- **No single element may take more than ~⅓ of the title bar** at the default window width.
- The capture cluster **degrades progressively** as the window narrows, in this order:
  drop the cost/GB readout → drop the elapsed timer → collapse the three source glyphs into one
  combined indicator → icon-only pause/stop. **The recording state dot and the stop control are
  the last things to go, and they never go.**
- Render the degradation ladder as its own strip in frame 11, so the order is a designed
  decision and not an accident of flex-shrink.
- The surface switcher stays visible at every width — it is the founder's confirmed-good
  element (see R4).

---

## R4. Window sizing is the user's, not ours

> "we need to also make sure that the window sizing will change according to the user"

The main window is resizable and people will drag it anywhere. Designing one 1100×720 frame and
calling it done is the failure this rule exists to prevent.

> **Correction (all five directions caught this).** As first written, R4 asked for frame 08
> at three widths while also fixing Quick Look at 1120×720 non-resizable — a contradiction.
> Resolved: **the three widths apply to the two main-window frames, 01 and 04.** Frame 08 stays
> at the shipped 1120×720; its 800/1440 renders, where a direction chose to keep them, are
> labelled as proofs of the column rule (2-up / 3-up / 4-up), not as window sizes a user can
> produce. Every direction handled this the same way and said so in its file.

- **Every main-window frame is designed at three widths**, and frames 01 (Timeline) and 04
  (Overview) must each be *rendered* at all three:
  - **800×600** — the minimum. Everything must still work: stage, rail, audio lane, readout,
    record control, switcher.
  - **1100×720** — the default.
  - **1440×900** — wide. Prove the layout uses the extra space instead of stretching one
    column to an absurd measure. Cap prose at ~70ch and put the surplus into the frame.
- The grid's column count follows width: **2-up under 1000px · 3-up 1000–1500 · 4-up ≥1500.**
- **Quick Look stays fixed at 1120×720 and non-resizable** — that is what ships
  (`windows.rs:809`, `resizable(false)`), and a launcher with a fixed layout is a feature.
- Nothing may depend on a hard-coded pixel width that exceeds the minimum window. Round 1's bug
  was a 700px results column in a window that could be 640px wide; do not repeat it in kind.

---

## R5. Confirmed good — do not touch

> "the switching between the two surfaces makes it good"

The two-surface switcher and the "make this my main surface" mechanism are settled. Keep your
direction's treatment as-is; spend the revision budget on R1–R4.

---

## What to change in your file, concretely

1. Rebuild frames **01, 02, 03** to the shipping timeline model (R1).
2. Delete every surface-strip error; rebuild frame **14** as toasts in situ (R2).
3. Add the title-bar degradation ladder to frame **11**; retune your chrome (R3).
4. Add the three window widths for frames **01**, **04**, **08** (R4).
5. Re-run frames **15** (component appendix: toast replaces strip) and **16** (scorecard rows
   for the rebuilt frames).
6. Append a **Revision 1** section: what you changed, the stage height you achieved, how many
   flow-inserting error elements you found and removed, and your title-bar degradation order.
