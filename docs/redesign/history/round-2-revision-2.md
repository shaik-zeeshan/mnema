# Revision 2 — applies to mockups 06–10

Founder review of Revision 1. Three corrections. This one is less about how the app looks and
more about whether the mockups are **usable as a build spec**.

---

## R6. Revert the timeline. I misread you.

> "I meant, don't change the timeline!"

Revision 1 said "rebuild on the shipping model and improve the execution." That was still a
redesign, and it was not what was asked. **The timeline is out of scope. Leave it alone.**

Concretely, for frames 01, 02 and 03:

- Render the timeline **exactly as it ships** — `apps/desktop/src/routes/+page.svelte`, markup
  from line 6043. Same structure, same proportions, same behaviour, same treatment:
  thin bar (jumper left; OCR chip / rerun / OCR toggle / refresh right) → stage → fixed-height
  rail-wrap (8px-per-frame tick rail, app-group bands with icons, sibling two-row audio lane
  with `mic` / `sys` labels, pointer-following readout with app icon + name + time + date) →
  AudioDrawer as a bottom sheet.
- **The only thing that changes is that it now uses the shared system**: the type roles from
  `system.css` §2, the spacing constants from §3, the control metrics from §4, and the
  primitives from §6. Nothing moves; nothing is added; nothing is "improved".
- Specifically **delete** anything Revision 1 invented on top of it: added time markers, band
  labelling schemes, skimmer/playhead pairs, texture on the audio bars, relative-age lines in
  the readout, minute rulers, "hold to scrub finely" hints. All of it. If it is not in
  `+page.svelte`, it is not in the frame.
- The frame's note under it should say, in one line, that this surface is unchanged and only
  re-typed and re-spaced — and list which type roles it now uses.

Everything else in the file stays in scope. The timeline is the one surface that is frozen.

---

## R7. The mockups must be a build spec, not a picture

> "designs are good, but I need it in a more app way. I don't see how they are going to be
> reused in the app."

Right now a developer looking at these cannot tell which piece is a reusable component, what it
is called, or which file it replaces. Fix that with three additions to every file.

### 7a. Adopt `system.css` verbatim

[`../system.css`](../system.css) is now the source of truth for type, spacing, control metrics,
elevation, motion and the shared primitives. Its colour tokens are **exactly the ones that ship
today**, so the whole block can be pasted into `+layout.svelte`'s `:root` without changing any
existing rule.

- Copy §1–§7 of `system.css` into your file's `<style>` **verbatim**, at the top, under a
  comment saying it is a copy and that `../system.css` wins.
- Then delete every token you had invented that duplicates one of these, and rewrite your
  direction-specific CSS to consume them. Your direction's *own* tokens (Paper's tone ladder,
  Panes' widened dark steps, Full Bleed's over-image plates) stay — they are what makes the
  direction — but they must be defined **in terms of** the shared ones where possible, and
  listed separately under a `/* direction-specific */` heading so a reader can see exactly what
  this direction adds to the system.
- If you believe a shared token is wrong, do not silently override it. Use it, and write the
  objection in your Deviations section.

### 7b. Add frame 17 — Component → code map

A table. One row per component in your appendix. Columns:

| Component | Class / proposed Svelte component | Replaces (file today) | Variants | States | Notes |

Ground it in what actually exists — the inventory below is real, from the current codebase:

- **There is no shared Button.** `.btn` / `.btn--ghost` / `.btn--sm` is a class convention
  re-declared in the scoped `<style>` of `routes/+page.svelte`, `lib/insights/Overview.svelte`,
  `Subjects.svelte`, `Context.svelte`, `ConclusionHero.svelte` and more, plus bespoke buttons
  (`.titlebar__record`, `.license-banner__btn`, `.quick-recall__state-btn`, `.gate-cta`, the
  settings buttons). Your Button row should say it consolidates all of them.
- Components that **do** exist and should be reused, not reinvented:
  `lib/components/{Input,Select,Combobox,Checkbox,RadioGroup,Switch,Slider,Stepper,Segmented,ActionSelect,FieldWarning}.svelte`,
  the `use:tip` tooltip directive in `lib/components/tooltip.ts`, and
  `lib/settings/ui/{SettingGroup,SettingRow,ButtonSpinner,ReloadButton}.svelte`.
  Say which of your appendix entries maps onto each, and where your design changes its
  appearance rather than its API.
- Things that are **new** and have no home yet: the Toast + toast stack, the surface switcher,
  the "make this my main surface" control, the capture-cluster degradation ladder. Say where
  each should live (`lib/components/`, `+layout.svelte`, etc.).
- Note anything you designed that **cannot** be built without a backend change, and say what
  the change is. Do not quietly assume data that does not exist.

### 7c. Say what is shared and what is yours

One short block, near the top of the file: "everything below the line is the shared system and
is identical in all five directions; here is the list of things this direction adds." A reader
choosing between the five should be able to see that the choice is narrow.

---

## R8. Typography and spacing — the actual complaint

> "I thought typography is weird. Some places the text is too large, some places the text is
> too short. I need to know because before implementing, we need to decide how and what looks
> where and how it looks."

The five files between them use roughly a dozen sizes and five weights, chosen per-frame. That
is the whole problem. `system.css` §2 fixes it by making size a **consequence of role**, not a
choice:

| token | px | line-height | tracking | weight | family | what it is for |
|---|---|---|---|---|---|---|
| `--t-label` | 10 | 1.4 | +.02em | 510 | mono | machine labels, column heads, source names, kbd, units. **Never a sentence.** |
| `--t-meta` | 11 | 1.35 | +.01em | 400 | either | timestamps under a title, counts, helper lines, frame captions |
| `--t-ui` | **13** | 1.25 | −.006em | 400 | sans | **the default** — buttons, rows, labels, nav, menus, chips |
| `--t-read` | 14 | 1.55 | −.008em | 400 | sans | prose only: transcripts, AI answers, error explanations. Max 70ch |
| `--t-title` | 17 | 1.3 | −.016em | 590 | sans | screen + section titles, moment title, dialog headings |
| `--t-display` | 22 | 1.2 | −.02em | 590 | either | **at most one per screen** — the readout clock, a hero number |

Hard rules, enforced: six sizes exist (no 12, 15, 16, 20, 26); max four sizes visible in one
region; three weights only (400 / 510 / 590 — **no 300, no 680**); 1.25 for UI and 1.55 for
prose, never 1.5 on UI; mono is the machine voice only and never exceeds `--t-title`;
`tabular-nums` on every number that changes in place.

Spacing gets the same treatment — named constants, not raw px: `--gap-inline` 6,
`--gap-label` 4, `--gap-row` 8, `--gap-group` 16, `--gap-section` 24, `--pad-window` 16,
`--pad-panel` 16, and **one inset per surface used for both its padding and its gutter**.

### What you must produce

**Frame 18 — Type & spacing specimen.** Not a swatch sheet. For each of the six roles:
the token name, its full spec, the rule for when to use it, and **three real examples pulled
from your own frames** rendered at true size. Then the spacing constants, each shown as a real
pair of elements at the real distance, labelled. A developer should be able to build a screen
they have never seen from this frame alone.

**Frame 19 — Where it's used.** Take your frame 01 (Timeline) and frame 04 (Overview) and
render them again with **every text element callout-labelled with its token**, and the four or
five key gaps dimensioned. This is literally the founder's ask — "what looks where" — and it is
the frame that decides whether this redesign is implementable.

**The audit.** Go through your whole file and force every text element onto the six roles and
every gap onto the named constants. Report the count of elements you had to change and the
sizes you removed. Any element that cannot be expressed in the system is either a mistake or a
gap in the system — say which.

---

## What to change in your file, concretely

1. **Revert frames 01/02/03** to the shipping timeline, re-typed and re-spaced only (R6).
2. Copy `system.css` in verbatim; rewrite your CSS to consume it; separate out your
   direction-specific tokens under their own heading (R7a).
3. Add **frame 17** — component → code map (R7b) and the shared-vs-mine block (R7c).
4. Add **frame 18** — type & spacing specimen (R8).
5. Add **frame 19** — the where-it's-used annotated Timeline and Overview (R8).
6. Re-run the audit across every frame; update frame 15 (appendix) and 16 (scorecard).
7. Append a **Revision 2** section: what you reverted, how many text elements you re-typed,
   which sizes you removed, and anything the system could not express.

Do not touch frame 07 (the surface switch) — still confirmed good.
