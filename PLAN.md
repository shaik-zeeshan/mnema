# Plan: Onboarding input components — every choice reports its own cost

Follow-on to the #195 onboarding rework (shipped in `fc31f7f7`; that plan is at
`git show fc31f7f7:PLAN.md`). The flow is settled. This plan is about the
*controls inside it*.

Design of record: `docs/onboarding/mockups/input-components/` — open `index.html`.
Each component there is live, carries the states you can drive, and names the
code it replaces. Two exploration rounds (55 variants across 8 input families)
narrowed to one design each; the mockups are the specification.

## Problem

Onboarding asks the user to choose a capture rate, a retention window, a storage
folder, eight feature toggles, two engines, two models, and an AI provider — and
answers almost none of them with a consequence. The controls are plain `Switch`,
`Segmented` and `Select`, so the user picks a word and learns what it cost much
later, or never.

Three concrete failures follow from that:

1. **The same value wears different controls on different screens.** Retention is
   a bespoke `role=group` on Capture & Storage (`CaptureStorageScreen.svelte:287`)
   and a `RetentionPicker` on Change settings (`ChangeSettingsScreen.svelte:401`),
   which disagree on option order and on whether "Forever" is special. Capture
   rate is a hand-drawn range on one screen (`:243`) and a `Select` on the other
   (`:416`). Two screens tell one user two stories about one setting.
2. **Dependencies are invisible until they fire.** Turning Microphone off
   silently cascades transcription and who's-speaking off; the rows that move are
   in a different section of a scrolling pane and are usually off-screen when
   they change. The Transcription → who's-speaking cascade has no copy anywhere.
3. **Costs are hidden or wrong.** The download total is spent without the 419 MB
   speaker model ever appearing in a picker; Tesseract's declared size overstates
   its download by ~57%; the disk gate never asks whether *capture* fits, only
   whether the models do.

The mockup round also surfaced three shipping bugs that are independent of any
redesign and should be fixed regardless — see Slices 1–3.

## Solution

Replace the inputs with six components that make the consequence move when the
user does, reusing the Settings capture-rate control's model: a ladder of
meaningful stops, a phrase rather than a raw number, a live consequence, and a
relative cost.

| component | what it does |
|---|---|
| **the sentence** | capture rate + retention + storage fold into one line; a ghost dial prints the alternatives faintly around each word |
| **excluded apps** | the summary line *is* the control — app names with inline icons are buttons, no editor, no disclosure |
| **feature switches** | the dependency graph drawn as a chain; hovering or focusing a row previews the cascade before it commits |
| **providers** | one recommended engine carrying *why*, alternatives carrying their price; the OCR selector leaves onboarding |
| **model pickers** | a toggle group over model *families* (not quality outcomes), plus a read-only download budget bar |
| **AI setup** | "later" is the primary path; behind it a key verifies as you type and the returned model list becomes the picker |

Four of these need a backend change to be honest rather than decorative; those
are separate, earlier slices.

## User Stories

1. As a new user, I want to see what a capture rate costs me per day before I
   accept it, so that I am not surprised by disk use a week later.
2. As a new user, I want to see how long my recordings are kept and how much is
   held at any time, so that "keep everything" is a decision rather than a default
   I never noticed.
3. As a new user, I want to see which apps are already excluded without opening
   anything, so that I can trust what the app decided on my behalf.
4. As a new user, I want to see what else turns off before I turn something off,
   so that I do not silently lose transcription by declining the microphone.
5. As a new user, I want to know what a model choice costs in download size and
   memory, so that I can pick a smaller one deliberately.
6. As a new user, I want to be told when my chosen folder cannot actually hold a
   day of recording, so that capture does not pause on the first segment.
7. As a new user, I want my AI key checked when I enter it, so that I do not
   finish setup with AI switched on and nothing behind it.
8. As a user who skips AI, I want to know exactly what stays dark and where to
   turn it on, so that skipping is an informed choice rather than an omission.

## Implementation Decisions

**Scope and reuse**
- The mockups in `docs/onboarding/mockups/input-components/parts/` are the spec.
  Port behaviour and copy from them; do not redesign during implementation.
- Components live in `apps/desktop/src/lib/onboarding/`, not in the screen files.
  Screens stay thin. Files stay under 800 lines.
- Reuse `$lib/components/` and `$lib/settings/` primitives (`Segmented`, `Switch`,
  `Slider`) where the mockup's mechanic allows; only the ghost dial and the
  sentence-as-control need new components.
- Tokens only — `var(--app-*)`, never a literal hex. Light theme must work.

**One control per value**
- The sentence is the single surface for capture rate, retention and storage
  location. Change settings drops its duplicates (`:401` `RetentionPicker`,
  `:416` rate `Select`) and renders the same component. This is the "one resolved
  settings path" premise of the branch applied to the controls themselves.
- Settings keeps its own richer `CaptureRateControl`; that is a different surface
  with different density, and it is not in scope.

**Motion**
- Onboarding's six working screens carry no ambient motion. The Settings
  capture-rate control's always-looping rAF sweep must not be ported. Movement is
  caused by the user's input or by real progress only.

**Numbers**
- Every figure is computed from the constants in
  `docs/onboarding/mockups/input-components/SPEC.md`, verified against the Rust
  manifests. Two corrections this round produced: the capture default is 2 s
  (405 MB/day, not the 270 MB/day 3 s anchor), and Parakeet's default is the
  **int8** build (670,619,803), so the honest delta from Whisper base is +523 MB.
- Any total containing nomic reads "about". Semantic Search has no cheap end —
  only *Off* is a real saving.
- Do not repeat "Apple Vision fast / Tesseract slower"; it has no basis in the
  repo. The defensible difference is that this build ships Tesseract English-only.

**Assumptions**
- The seeded privacy set and its recommended-app resolution keep working as today;
  only its presentation changes.
- No migration is required — every value already has a settings field. The one
  exception is privacy rules with an unresolved bundle id (Slice 5).

**Open questions**
- *Retention inside the sentence.* The sentence's strongest clause is "12 GB held
  at any time", and that clause is absent in the default case because `never` has
  no ceiling. The mockup solves it by always printing *rate · horizon* (a
  first-year projection when `never`, a steady ceiling otherwise). If that reads
  as strained in the real flow, the fallback is to lift retention out to its own
  row directly beneath the sentence, fed by the same numbers. Decide after
  Slice 6 is on screen, not before.
- *Excluded-apps list length.* The sentence reads well to 8 apps, becomes a list
  wearing sentence punctuation at 9–11, and should not be a sentence at 12+.
  Onboarding seeds three. If the seeded catalog ever passes eight, this becomes a
  chip list — not a smaller font.

## Testing Decisions

- **Pure logic gets unit tests, controls get screenshots.** The consequence
  arithmetic (`disk-estimate.ts`, `capture-rate.ts`, `feature-rules.ts`, the new
  gate term) is dependency-free and bun-testable; test it directly.
- **Verify UI by rendering, never by grepping class names.** Grepping CSS has
  produced false "matches the mockup" verdicts in this repo before. Screenshot at
  1040×680 and look at it, in both themes.
- Port each mockup's `console.assert` self-check into a real test — they already
  pin the manifest sums, the 270 MB/day anchor, and the escape arithmetic.
- Regression tests specifically for the bugs in Slices 1–3: an off-ladder legacy
  fps resolving to a real ladder stop; a volume that fits the models but not a day
  of capture; a re-probe after re-picking the same folder; a speaker model that is
  neither installed nor in the work-list rendering its real size.
- Cascade behaviour is tested through `applyToggle`/`cascadeOf` results, not
  through DOM order.
- Do not test the ghost dial's rendering geometry; test the value it commits.
- Manual drill: run onboarding on a small external volume and confirm the disk
  verdict, the escape, and the disconnect state all read correctly.

## Slices

Slices 1–5 are backend/logic and unblock the components. Slices 6–11 are the
components themselves. 1–3 are independently shippable today and fix live bugs.

1. **Correctness fixes in existing controls**
   - Goal: stop the current UI stating things that are false.
   - Areas: `ChangeSettingsScreen.svelte:179` (route capture rate through
     `nearestLadderIndex` so a legacy 7.5 fps stops rendering a blank `Select`);
     `:344` (pass a real fallback to `downloadNote("speakerSeparation", …)` so a
     419 MB download stops reading "no download"); `:177` + `format.ts:21`
     (one byte formatter, extend past GB so a 2 TB volume stops reading
     "1322.5 GB"); `CaptureRateControl.svelte:25` (`Math.floor((60 - ε)/s) + 1` —
     the 45 s stop holds 2 snapshots, not 1); `crates/ocr/src/lib.rs:998`
     (compute `byte_size` from the file list as `audio-transcription` does —
     declared 23,143,206 vs actual 14,675,815).
   - Acceptance: unit tests per fix; the OCR size test asserts declared == summed.
   - Depends on: none. **Parallel: yes.**

2. **Disk gate: add the missing capture term**
   - Goal: stop passing volumes that cannot record.
   - Areas: `apps/desktop/src/lib/onboarding/gates.ts:58` — `requiredBytes` is
     the model work-list only; add the 1 GiB reserve (`disk_space.rs:30`) plus at
     least one day at the chosen capture rate. Note the consequence: **the
     Semantic Search escape is illusory** — any volume too small for the 1.115 GB
     work-list is also too small for reserve + models + a day, so dropping nomic
     clears the check and leaves nowhere to record. The escape copy must change
     with the term.
   - Acceptance: a 1.3 GB volume fails; a volume that fits models but not a day
     fails with the capture-specific reason; `freeBytes: null` still never blocks
     (ADR 0040).
   - Depends on: none. **Parallel: yes, with 1 and 3.**

3. **Storage probe: make it re-runnable and legible**
   - Goal: let a user recover from a bad folder without restarting onboarding.
   - Areas: `CaptureStorageScreen.svelte:94` (the probe `$effect` keys off
     `draftSaveDirectory`, so re-picking the *same* folder after fixing its
     permissions is an equal `$state` write and nothing re-probes — add an
     explicit re-check); `:112`/`:144` (a failed probe sets `null` and renders as
     "checking…" forever — separate "not yet probed" from "probe failed"); guard
     the ancestor walk in `measure_free_space` so a disconnected
     `/Volumes/X/Mnema` stops reporting the boot disk's free space.
   - Acceptance: re-check re-probes an unchanged path; a failed probe shows an
     error; a disconnected volume never reports another volume's bytes.
   - Depends on: none. **Parallel: yes, with 1 and 2.**

4. **`feature-rules.ts`: expose the cascade**
   - Goal: make a cascade predictable instead of retrospective.
   - Areas: new `cascadeOf(before, after, touched)` and `preview(state, id)`
     exports — cheap, because `applyToggle` already returns fresh state and never
     mutates its input, so both are a diff. Add the missing
     `featureLockReason` case: turning Transcription on with no audio source is a
     **silent no-op today** (`applyToggle` sets it, `normalizeFeatures` unsets it,
     the switch bounces with no explanation). Add `lockFix`, which walks up to the
     first actually-flippable ancestor so a locked row never offers a locked fix.
     `onboarding-flow.svelte.ts:153` keeps the pre-toggle state for undo.
   - Acceptance: unit tests for each cascade pair, for the no-op case now
     reporting a reason, and for `lockFix` skipping locked ancestors.
   - Depends on: none. **Parallel: yes.**

5. **Privacy rules for apps that are not installed**
   - Goal: let a user exclude an app before installing it.
   - Areas: `uniqueBundleIds()` (`lib/app-privacy-exclusion.ts:86`) silently drops
     an entry with an empty bundle id, so a name-only rule stored today protects
     nothing and says nothing. Storage already fits
     (`ExcludedAppEntry {id, enabled, bundle_id, display_name}`). Accept a rule
     with empty `bundle_id` + typed `display_name`; resolve by display name on app
     list refresh or first sighting, filling the id in place; keep unresolved rows
     out of both the screen filter and the tap exclude list until they resolve.
   - Acceptance: an unresolved rule round-trips, never reaches either filter, and
     resolves on first sighting.
   - Depends on: none. **Parallel: yes.**

6. **Component: the sentence** (capture rate + retention + storage)
   - Goal: one line replaces three controls and their two duplicates.
   - Areas: new component in `lib/onboarding/`; replaces
     `CaptureStorageScreen.svelte` ~`:243-263`, ~`:273`, ~`:287-296` and deletes
     `ChangeSettingsScreen.svelte:401` and `:416`. Location token calls the
     existing `@tauri-apps/plugin-dialog` `open({directory:true})`. Ghost dial per
     the mockup: retention prints all four stops, rate uses a 5-wide clamped
     window, place shows one ghost (`~/.mnema`) at the end of the sentence.
     `role="spinbutton"` + `aria-valuetext`; ghosts are `aria-hidden` and out of
     the tab order, but every value a ghost reaches is reachable from the token.
   - Acceptance: all failure states render (probing · missing · read-only ·
     free-space-unknown, which must not block · downloads don't fit · a day
     doesn't fit · disconnected); screenshot review in both themes.
   - Depends on: 2, 3. **Parallel: yes, with 7–11 once 2 and 3 land.**

7. **Component: excluded apps**
   - Goal: what was decided on the user's behalf is visible without a click.
   - Areas: replaces `CaptureStorageScreen.svelte:304-327`; deletes
     `editingExclusions` (`:172`), the `Edit ▸/Done` button, the `.editor`
     wrapper, `comboboxListId` and `excludedSummary` (`:189`). Icons are 1em
     CSS-masked strokes on the baseline, not tiles — glyph + name is one
     unbreakable word. Striking reuses the existing
     `set_privacy_excluded_app_enabled(sourceId, enabled)`, which is why a struck
     name holds its slot and the sentence never reflows. Every audio claim stays
     qualified ("and not system audio **while that filter is on**" —
     `native_capture/privacy.rs:89`).
   - Acceptance: empty state, seeded state, striking, adding an installed app,
     adding one that is not installed; Enter in the add field must not
     immediately strike the app it just added (`preventDefault` — the mockup
     shipped this bug and caught it by rendering).
   - Depends on: 5. **Parallel: yes.**

8. **Component: feature switches**
   - Goal: cascades are visible before they commit.
   - Areas: the eight rows in `ChangeSettingsScreen.svelte` (`:236`, `:252`,
     `:271`, `:295`, `:330`, `:347`, `:380`, `:448`). Indentation means exactly
     one thing — child of the row above; Transcription's second parent is drawn
     as a gutter bracket, not a connector. Hover/focus dashes the branch, tags
     rows `GOES OFF`, previews both totals, and announces the same sentence on
     `aria-live`. Locked rows render the fix, not a dead switch. The AI row's
     soft gate renders in place, never pre-ticked.
   - Acceptance: the four drills (mic off cascades two, transcription off
     cascades one, granting the permission, the AI soft gate); running total moves
     on every flip; undo restores.
   - Depends on: 4. **Parallel: yes.**

9. **Component: providers**
   - Goal: one recommended engine that says why.
   - Areas: replaces `ChangeSettingsScreen.svelte:315`; **deletes `:289`** (the
     OCR selector leaves onboarding — Apple Vision has no download, no memory
     cost and no axis on which Tesseract wins in this build; OCR becomes a
     resolved read-only line and the choice stays in Settings). Deepgram stays
     filtered (`:146`, `onboarding.svelte.ts:379-382`) but is *named* in a
     `<details>` disclosure rather than shown as a disabled radio. Unbenchmarked
     axes render as empty hatched tracks, never an invented winner.
   - Acceptance: each engine selectable with its delta; return-to-recommended
     works; the cloud disclosure reads honestly.
   - Depends on: 1 (the OCR byte-size fix feeds the resolved line).
     **Parallel: yes.**

10. **Component: model pickers**
    - Goal: no model becomes unreachable, and the download feels like a budget.
    - Areas: replaces `ChangeSettingsScreen.svelte:322` and `:370`. The family
      group already exists at `:317`, so this is "delete the `Select`, add a
      second `Segmented`". A toggle group over quality *outcomes* is rejected: four
      segments over seven transcription models makes Whisper Small and both
      Parakeet builds unreachable. Variant sub-group appears only for families
      with more than one build, in a height-reserved row so the block never jumps.
      Budget bar is a read-only footer — the only place the silent 419 MB speaker
      model is visible.
    - Acceptance: all 7 transcription and 5 semantic models reachable; totals
      recompute; "about" appears and disappears with nomic; the over-disk escape
      lands under free disk.
    - Depends on: 2 (the escape copy changes with the gate term). **Parallel: yes.**

11. **Component: AI setup**
    - Goal: finishing setup with AI on and nothing behind it becomes impossible.
    - Areas: replaces `ChangeSettingsScreen.svelte:448`, `:463-471`, `:483-492`,
      `:516`, `:525`/`:539`, `:553`, `:570`. Needs a **new Tauri command**
      `verify_ai_provider(id) -> { models, latency_ms }` that lists models using
      the key already in `day.mnema.vault` — the frontend never receives the key.
      Readiness becomes *provider verified live this session AND that listing
      still contains the chosen model id*. That reworks four call sites:
      `onboarding-ai.svelte.ts:144-157` (stop treating "a string reached the
      keychain" as configured, and stop exempting non-cloud kinds from any
      endpoint check), `:189-193` (promote `aiUnverifiedNote` from hint to gate),
      `ChangeSettingsScreen.svelte:188-192` (auto-enable on the *verified*
      transition), and `syncFromSettings` (`:114-124`, re-validate the restored
      model against the fresh pool and clear it with a visible reason).
    - Acceptance: a rejected key blocks; an unreachable endpoint blocks; the local
      scan finds and fails gracefully; two same-kind providers coexist; the
      default-model picker stays disabled until a provider exists; the key never
      appears in the DOM.
    - Depends on: none of the above, but it is the largest slice. **Parallel: yes.**

**Parallel groups:** `[1, 2, 3, 4, 5]` → `[6 after 2+3, 7 after 5, 8 after 4, 9 after 1, 10 after 2, 11]`.
Slices 1–3 can ship on their own before any component work starts.

## Out of Scope

- The Settings surface. `CaptureRateControl` and the Settings panels keep their
  current controls; only onboarding changes.
- Cloud transcription in onboarding. Deepgram stays Settings-only behind its
  consent gate (ADR 0047) — this plan only makes its absence honest.
- The onboarding *flow* — screens, order, gates as concepts, the attention model.
  Settled in `fc31f7f7`.
- Reworking the permission screens or voice enrollment. Those are actions, not
  settings inputs, and have no consequence to report.
- The known live bug where a job against an absent model dies in ~6 min and is
  never retried. Slice 11 closes onboarding's path *into* it; the job-runner fix
  (claim-time lock gating) is separate work.
- A background download queue. The AI mockup's "arrives on its own" idea depends
  on one and is deferred with it.

## Further Notes

- **Risk: the sentence is dense.** Nine faint words on two lines at rest is
  inherent to the ghost-dial mechanic, not a bug to design away. If it does not
  survive contact with real users, the retention fallback in Open Questions is the
  first thing to try.
- **Risk: screen/audio privacy parity is convention, not type-enforced.**
  `privacy.rs` derives both lists from the same `decision.excluded_bundle_ids`
  through one unit-tested function, but the tap is a separate consumer handed the
  list at `lifecycle.rs:394`/`:633`. Slice 7's copy claims parity, so a future
  call site starting a tap with an empty vec would make the UI lie. Worth a type
  that makes the empty case unrepresentable.
- **`SUPPORTS.md`** needs no change — nothing here is platform-conditional.
- The mockups are self-verifying: each carries a `console.assert` block that fails
  loudly if its arithmetic drifts from the manifest constants. Keep them building
  (`docs/onboarding/mockups/input-components/build.sh`) as a reference while
  implementing; they are cheaper to consult than the diff.
- Sizes to re-check if the manifests move: speakrs 419,482,724; nomic 548,000,000;
  Whisper base 147,951,465; Parakeet int8 670,619,803. Only the test-only
  `semantic-search/src/models.rs:693` (250,000,000) is stale, deliberately.
