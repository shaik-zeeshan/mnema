# Plan: Onboarding revision 2 — one footer, four section screens, one control height

Follow-on to the #195 input-components plan, which this file replaces
(`git show 97bb04c4:PLAN.md`). That plan's slices 1–11 are implemented in the
working tree — the six components exist in `apps/desktop/src/lib/onboarding/` and
the screens are thin shells over them. This plan is about what a founder review
of that result found.

Design of record: **`docs/onboarding/mockups/revision-2.html`** — open it and
drive the controls; they are the real components, inlined live from
`input-components/parts/`. Source is `revision-2.src.html`, assembled by
`build-revision-2.py` (re-run it after editing the source). It supersedes
`chosen-cinematic-rewind.html` for six screens; Welcome, Permissions and Your
settings still come from that file.

## Problem

Three complaints, in the order they were raised, plus three duplicates found
while resolving them.

1. **A hole on the left of three screens.** Setup, Voice and the Finale put their
   content in a 360px column and pinned the action row to that column's floor
   (`margin-top: auto`). The content runs out after two lines, so the screen shows
   a column-height gap with a button parked under it. The same shape appears on
   Change settings, whose 150px section rail holds four links and then nothing.
2. **Four sections, one screen.** Change settings scrolled four sections past that
   rail. It is the densest screen in the flow, the sections are unrelated to each
   other, and a rule that fires at the bottom of a scroll ("AI features are on
   with nothing to run them") is invisible when it fires.
3. **Controls beside controls are different heights.** Measured on one screen:
   select `33.0`, text field `34.5`, the buttons next to them `20.0`, model option
   `30.5`, plain button `22.0`, switch `20.0`. Height came from padding plus a 1.5
   line-height, so several land on fractional pixels. Rows that mix a label and a
   button aligned on the text baseline, which hangs the button box below the text
   beside it.

Found while fixing those, all "one setting wearing two faces":

4. The transcription **engine** was offered twice once Engines and Models became
   separate screens — as cards, and again as a segmented family row.
5. Semantic Search **on/off** was offered twice — the switch chain, and an `Off`
   segment inside its family group.
6. The **OCR** line ("Reading on-screen text · Apple Vision") sat inside
   `Providers`, on a screen about speech engines, restating what the switch chain
   already says.

## Solution

One footer rule, sections as screens, one control height — and delete the three
duplicated controls rather than reconcile them.

- **Nothing is pinned inside a column.** Every working screen is content that
  fills, then one full-width footer: state on the left, actions on the right.
  Where two columns remain (Voice, Finale) they centre against each other. Setup
  loses its column entirely — one statement and a list of rows, at full width.
- **Change settings is four screens** behind a tab strip that replaces the rail:
  *What Mnema will do · Engines · Models · AI features*. Still step 4 of 8 in the
  phase bar — one round trip, not four new steps.
- **Two control heights, from tokens.** Row controls (fields, selects, any button
  beside one) share one height; inline controls (status-row buttons, the add-app
  chip, footer ghosts) share a smaller one. Rows that mix text and buttons centre
  rather than align baselines.
- **Storage as a section is gone.** It repeated screen 03's sentence exactly.
  Capture rate, retention and folder are settled once, on Capture & Storage, one
  Back away.

## User Stories

1. As a new user, I want each screen to look composed rather than half-empty, so
   that the app reads as finished and I trust it with my screen.
2. As a new user changing settings, I want one subject at a time, so that I am not
   scrolling a wall to find the two rows I came for.
3. As a new user, I want to see which section I am in and jump to another, so that
   I can go straight to AI features without walking the other three.
4. As a new user, I want a rule that blocks me to be next to the row it is talking
   about, so that "AI features are on with nothing to run them" is actionable.
5. As a new user, I want a button beside a field to be the same height as the
   field, so that the screen does not look assembled from parts.
6. As a new user, I want to answer each question once, so that meeting the same
   setting again in a different control does not make me doubt which one won.

## Implementation Decisions

**Scope**

- Six screens change: Capture & Storage (03), Change settings (now four), Setup
  (06), Voice (07), Finale (08). Welcome, Permissions and Your settings are
  untouched.
- No component is redesigned. `CaptureSentence`, `ExcludedApps`,
  `FeatureSwitches`, `Providers`, `ModelPickers` and `AiSetup` keep their
  mechanics; three lose a duplicated control, and their fields adopt the height
  token.

**The footer rule**

- `.ob-foot` is shell-owned and already exists. The change is that screens stop
  putting an action row inside a column: delete `margin-top: auto` from
  `SetupScreen`'s `.foot`, `VoiceScreen`'s and `FinaleScreen`'s equivalents, and
  move those rows into the screen-level `.ob-foot`.
- Two-column screens set `align-items: center` on `.split`. Setup drops `.split`.

**Sections as screens, not steps**

- The four sections are **internal state of the Change settings step**, not new
  flow steps. `onboarding-flow.svelte.ts` and the phase bar are untouched; Back
  and Continue keep their meaning; the round trip still returns to Your settings.
- `ChangeSettingsScreen.svelte` renders one section at a time behind a tab strip.
  That deletes the rail (`.idx`, `.idx-link`), the scroll pane, `jumpTo()` and the
  scroll-spy `spy()` — roughly 60 lines of scroll plumbing for a `$state` string
  and an `{#if}`.
- `onConnectAi` currently calls `jumpTo("ai")`; it becomes a section switch.
- Footer per section: ghost back to the previous section (or to Your settings on
  the first), primary to the next (or "Back to your settings" on the last). The
  tab strip is the jump affordance.

**One control height**

- Add to the onboarding shell: `--ob-ctl-h: 40px` and `--ob-ctl-h-sm: 28px` (the
  app's register is 13px type; the mockup shell uses 32/22 at 11px — same rule,
  different scale).
- `onboarding-shell.css:202` — `.ob-btn` becomes `inline-flex` + `align-items:
  center` + `min-height: var(--ob-ctl-h)` + `line-height: 1` + horizontal-only
  padding; `.sm` takes `--ob-ctl-h-sm`. `min-height`, not `height`, so
  deliberately tall controls (the AI choice cards) keep their size.
- Component fields take the same token: `AiSetup.svelte:890` (`padding: 9px 12px`
  → `height: var(--ob-ctl-h); padding: 0 12px`), and the equivalent field rules in
  `CaptureSentence` and `ModelPickers`. A button on a field's row takes
  `--ob-ctl-h`, not the inline height.
- Rows mixing a label and a button switch from `align-items: baseline` to
  `center`: `SetupScreen.svelte:448` and `:462`. Baseline stays where a row is
  text against text.

**One control per value**

- **Engine** is chosen on Engines only. Models names it as context ("Engine:
  Whisper — chosen on Engines") and draws no family row. The app already wires it
  this way (`ChangeSettingsScreen.svelte:195` passes `transcriptionFamilies={[]}`);
  the split makes the reason visible.
- **Semantic Search on/off** belongs to the switch chain. Drop the `Off` entry
  from `SEMANTIC_FAMILIES` (`model-budget.ts:91`); keep `SEMANTIC_OFF` as an
  internal family value so a disabled feature resolves to *no active segment*.
  `onSemanticEnabledChange` stays — the budget escape ("Turn Semantic Search off",
  offered when the disk is short) calls it, and that is a repair at the moment the
  shortfall is printed, not a picker.
- **OCR leaves onboarding.** Delete the `.resolved` block in `Providers.svelte`
  (~`:171`) with its `ocrProviders` / `ocrProvider` props and the two call sites
  (`ChangeSettingsScreen.svelte:190-191`); drop `ocrModelStatus` here if nothing
  else reads it. Apple Vision remains the resolved default and Tesseract remains
  reachable in Settings; the switch chain already states both.

**Frame size**

- The real onboarding window is **1120×800**, minimum 920×620
  (`src-tauri/src/windows.rs:200`). The old mockup drew 1040×680, which is
  neither; `revision-2.html` draws the real one. Every "verify at 1040×680"
  instruction in `docs/onboarding/` is stale and must be corrected with this work
  (`IMPLEMENTATION-BRIEF.md:94`, `onboarding-shell.css:10`,
  `VoiceScreen.svelte:349`).

**Assumptions**

- Tokens only — `var(--app-*)`, never a literal hex; both themes must work.
- No new dependency, no new Tauri command, no migration. Nothing in this plan
  changes a stored value.

## Testing Decisions

- **Verify by rendering and looking, in both themes.** Grepping class names has
  produced false "matches the mockup" verdicts in this repo. Screenshot each
  changed screen at **1120×800** and compare against the same frame from
  `revision-2.html`.
- **Also screenshot at the 920×620 minimum.** That is where the layout claims get
  tested: at 800px the switch chain fits with room to spare, at 620 it will
  scroll. Scrolling one section is acceptable; a clipped footer or a control row
  that wraps is not.
- Assert the height rule rather than eyeball it: one test that reads
  `getBoundingClientRect().height` for a field and the button beside it and
  requires equality. This is the class of bug that returns silently.
- Unit-test the two logic changes: `SEMANTIC_FAMILIES` no longer offers `Off`, and
  a disabled feature still resolves to a family with no active segment (update
  `model-budget.test.ts`).
- Section navigation is behaviour: entering Change settings lands on section 1,
  each tab renders its own section, the AI gate holds "Back" from the AI section,
  and `onConnectAi` from the chain switches sections.
- Do not test CSS values directly, and do not test tab markup — test the rendered
  height and which section is on screen.

## Slices

1. **One control height**
   - Goal: a button beside a field is the field's height; nothing lands on a
     fractional pixel.
   - Areas: `onboarding-shell.css:202-241` (tokens + `.ob-btn` / `.ob-btn.sm`);
     field rules in `AiSetup.svelte:890`, `CaptureSentence`, `ModelPickers`;
     `SetupScreen.svelte:448`/`:462` baseline → center.
   - Acceptance: the measured-height test passes for the AI field row; screenshots
     of Change settings and Setup in both themes; no control reports a fractional
     height.
   - Depends on: none. **Parallel: yes.** Ships on its own, before any layout work.

2. **Setup, Voice, Finale: the footer rule**
   - Goal: no action pinned inside a column.
   - Areas: `SetupScreen.svelte` (drop `.split`, stack percent + line + rule +
     work list, move actions to `.ob-foot`), `VoiceScreen.svelte` and
     `FinaleScreen.svelte` (`align-items: center` on `.split`, actions and state
     into `.ob-foot`).
   - Acceptance: screenshots at 1120×800 and 920×620 with no column-height gap and
     no clipped footer; Continue on Setup is still live on arrival and never
     disables.
   - Depends on: none. **Parallel: yes, with 1 and 4.**

3. **Docs: correct the stale frame size**
   - Goal: stop three files instructing a review at a window size that does not
     exist.
   - Areas: `docs/onboarding/IMPLEMENTATION-BRIEF.md:94`,
     `onboarding-shell.css:10`, `VoiceScreen.svelte:349`; point the brief at
     `revision-2.html` for the six screens it supersedes.
   - Acceptance: no `1040×680` remains outside `chosen-cinematic-rewind.html`.
   - Depends on: none. **Parallel: yes.**

4. **Change settings → four section screens**
   - Goal: one subject per screen, and the Storage duplicate gone.
   - Areas: `ChangeSettingsScreen.svelte` — replace the rail and scroll pane with
     a tab strip plus `active` section state; delete `jumpTo()`, `spy()`, `.idx*`,
     `.scrollpane`; per-section footers; `onConnectAi` switches section. Delete the
     Storage section and with it the screen's **second copy of the storage probe**
     (`seq`, `probedPath`, `probeState`, `runProbe`, its `$effect` and the
     `CaptureSentence` block) — Capture & Storage stays its only writer.
   - Acceptance: each section renders alone; tabs jump; the AI gate holds Back from
     the AI section and names the row it means; screenshots at both window sizes;
     the file shrinks.
   - Depends on: none, but touches the same file as 5 and 6 — sequence them.
     **Parallel: no with 5, 6.**

5. **Providers: OCR leaves onboarding**
   - Goal: the speech-engine screen is only about speech engines.
   - Areas: `Providers.svelte` `.resolved` block (~`:171`) and its
     `ocrProviders` / `ocrProvider` props; call sites
     `ChangeSettingsScreen.svelte:190-191`; drop `ocrModelStatus` if now unread.
   - Acceptance: `bun run check` clean with no unused props; the Engines screen
     matches the mockup; the switch chain still states OCR.
   - Depends on: 4. **Parallel: no with 4, 6.**

6. **ModelPickers: no `Off` segment**
   - Goal: Semantic Search is switched in one place.
   - Areas: `model-budget.ts:91` `SEMANTIC_FAMILIES`; the family group render in
     `ModelPickers.svelte:280`; `model-budget.test.ts`.
   - Acceptance: the group offers English and Multilingual only; with the feature
     off, no segment is active and the strip reads the off state; the budget escape
     still turns the feature off.
   - Depends on: 4. **Parallel: no with 4, 5.**

**Parallel groups:** `[1, 2, 3]` → `[4]` → `[5, 6]`.
Slice 1 is worth shipping on its own — it is the whole "uneven" complaint, and it
is independent of every layout change.

## Out of Scope

- Welcome, Permissions, Your settings — unchanged, and still specified by
  `chosen-cinematic-rewind.html`.
- The flow itself: eight steps, two hard gates, the attention model, the round
  trip. Settled in `fc31f7f7`.
- The components' mechanics. The ghost dial, the chain, the cards, the budget bar
  and the later-loudly AI path all stay as shipped.
- The Settings surface. `CaptureRateControl` and the Settings panels keep their
  own controls and their own density.
- A shared control-height token for the whole app. This plan scopes it to the
  onboarding shell; generalising it to `+layout.svelte` is a separate decision.
- Cloud transcription in onboarding (ADR 0047 stands), and the known live bug
  where a job against an absent model dies unretried.

## Further Notes

- **Risk: four tabs invite jumping, and the sections are not independent.**
  Turning the last audio source off on section 1 changes what section 3 costs. The
  running total on section 1 and the budget bar on section 3 are the same numbers
  from the same source, so they cannot disagree — but if review finds the jump
  confusing, the fallback is to keep the tabs as *indicators* and make the footer
  the only navigation.
- **Risk: 920×620.** Four sections fit comfortably at 800px of window height. At
  the minimum, the switch chain and the Engines cards will scroll inside their
  section. That is the acceptable outcome; a clipped footer is not. Test it.
- The Storage deletion means retention and capture rate can only be changed from
  Capture & Storage during onboarding. That is one Back from Your settings, and
  both are freely editable in Settings afterwards. If that turns out to be a
  complaint, the section comes back rather than the sentence being duplicated.
- `SUPPORTS.md` needs no change — nothing here is platform-conditional.
- Keep the mockups building while implementing:
  `docs/onboarding/mockups/build-revision-2.py` and
  `input-components/build.sh`. Both carry the components' `console.assert`
  self-checks, which fail loudly if the arithmetic drifts from the manifests.
