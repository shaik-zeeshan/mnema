# Plan: Onboarding redesign — feature-toggle stacked accordion + wider Segmented adoption

Reference mockup: `docs/onboarding/mockups/stack-a-polished.html` (this is the approved visual + interaction target).

## Problem

Mnema has grown well past what the current onboarding was built for. The existing flow (`apps/desktop/src/routes/onboarding/+page.svelte`, an 8-step linear wizard: about → permissions → sources → video → storage → privacy → processing → done) treats every capability as a mandatory step and walks the user through all of them in a fixed-height stage frame. That worked when there were a handful of settings; now there are many features (screen capture, microphone, system audio, OCR, transcription, privacy exclusions, storage/retention, Ask AI) and the linear wizard is long, undifferentiated, and gives the user no way to say "I don't want that one."

Users need onboarding to let them **pick the capabilities they want and skip the ones they don't**, and to **open any capability to configure it** without being marched through all of them. Foundational pieces that the product can't run without must be clearly distinct from optional ones. The UI must also feel like the rest of the app (terminal/green, monospace, the shared control family) and be minimal and modern.

Secondary problem: the app already ships a `Segmented` control (the "radio-select" the user likes — accent-green active segment) but no settings panel uses it; seven panels still render single-choice options as vertical `RadioGroup`s or ad-hoc chip buttons. The same control should be adopted in onboarding and across the settings spots where it fits.

## Solution

Replace the linear wizard with a **single-column stacked accordion**. Each capability is a full-width row: an icon + name on the left, a toggle on the right. Clicking a row's body expands it in place into one large configuration panel (accordion — exactly one open at a time); the toggle enables/disables the feature independently of expansion. Required capabilities (Permissions, Screen capture, Storage) appear as rows with a locked toggle and an on-theme lock "Required" marker; optional capabilities (Microphone, System audio, OCR, Transcription, Privacy, Ask AI) are freely toggleable. A slim welcome header sits above the stack and a footer carries the live "N features on" hint and the Finish/Start CTA.

Technically: decompose the giant `+page.svelte` into a presentational accordion shell + a feature model + small per-feature config bodies that reuse the shared control family (`Segmented`, `Switch`, `Select`, `Combobox`, sliders) and the existing permission-request and model-download logic. Reuse the real settings section icons (extracted from `SettingsRail.svelte` into a shared module). Persist through the existing `recording_settings` Tauri commands with no schema change. Apply the minimal/modern styling system proven in the mockup (4px spacing scale, 5-step type scale, reduced decoration). In parallel, adopt `Segmented` in the settings panels where a single choice has a few short options.

## User Stories

1. As a new user, I want to turn capabilities on or off during setup, so that Mnema only records and processes what I actually want.
2. As a new user, I want to open any capability and adjust its settings inline, so that I can configure it without a multi-screen wizard.
3. As a new user, I want foundational capabilities (permissions, screen capture, storage) to be clearly marked as required and not accidentally turn-off-able, so that I can't break the core recorder during setup.
4. As a new user, I want to see at a glance which capabilities are on and whether anything still needs attention (a permission to grant, a model to download), so that I know when I'm ready to record.
5. As a returning user in Settings, I want single-choice options shown as the same compact segmented control used elsewhere, so that the app feels consistent.

## Implementation Decisions

### Onboarding shell & interaction
- **Layout = scrolling stacked accordion**, not the prior fixed-height per-step stage frame. This is a deliberate departure from the existing onboarding paradigm (and from the "steps must fit a fixed-height stage frame / no per-step scroll" note in memory — that note becomes obsolete; update it after this lands).
- **Accordion: exactly one row expanded at a time.** Expanding a row collapses the previously open one. Screen capture is open on first load so the page reads like the mockup.
- **Interaction model M1: toggle and expand are independent.** The right-side toggle enables/disables the feature (`stopPropagation` so it never expands the row); clicking the row body expands/collapses it. A feature can be ON while collapsed, or OFF while expanded (its body controls dim when off).
- **Required set = Permissions, Screen capture, Storage & retention.** Rendered with a locked toggle (shown on, disabled, dimmed) and an on-theme lock glyph + "Required" marker beside the toggle (monochrome SVG in the app icon style — no orange status dot, no Unicode glyph, no title badge). The marker shows in both collapsed and expanded states.
- **Welcome & finish:** a slim inline welcome header (eyebrow + one line) above the stack and a footer CTA that reads "Finish setup →" / "Start recording" with a live "N features on · M need attention" hint. The old standalone "About" and "Ready" full screens are folded into this header/footer rather than kept as separate steps. The old `ProgressArc` phase model (capture/index/recall) is removed.

### Components & decomposition
- Decompose `apps/desktop/src/routes/onboarding/+page.svelte` (currently 2,674 lines) into: an accordion shell (`FeatureStack` + `FeatureRow`), a feature-model/state module, and one small body component per capability. Every resulting file stays under the 800-line code-org limit.
- Reuse existing onboarding helpers where still relevant (`AdvancedReveal.svelte`, `ArmStatus.svelte`). Retire `ProgressArc.svelte` and `SceneShell.svelte` if the new shell supersedes them.
- **Icons:** extract the section-icon SVGs currently inlined as the `navIcon` snippet in `apps/desktop/src/lib/settings/SettingsRail.svelte` into a shared, importable module (e.g. `lib/settings/section-icons.ts` or a small `Icon.svelte`), and consume it from both the rail and onboarding (and any settings panel that wants a section icon). Add a `lock` glyph in the same 24×24 / 1.7-stroke style. The per-row icon chip tints to accent when the feature is enabled/armed or its row is open.
- **Single-choice controls use `Segmented`** (`apps/desktop/src/lib/components/Segmented.svelte`) for resolution, bitrate, OCR provider + recognition mode, transcription provider + model size, etc. Keep `Select`/`Combobox` for long/open lists (language, input device, AI model lists, the privacy app typeahead) and `Switch` for booleans.
- **Retention: reuse `RetentionPicker.svelte`** (parity with Settings, keeps custom-day support) — not `Segmented`.

### State, persistence, backend
- No `RecordingSettings` schema change. Map feature toggles onto existing fields (screen always on; microphone/system-audio/ocr/transcription/askAi booleans; excluded apps; resolution/bitrate/fps/segment; retention; etc.).
- Load via the existing `get`/`recording_settings` command into a draft; persist via the existing `update_recording_settings` command. **Save only on finish** — a single atomic write of the full draft when the user confirms. Toggles and in-row edits mutate the in-memory draft only; nothing is persisted mid-flow (so abandoning onboarding leaves prior settings untouched).
- Permission requests reuse the existing `request_capture_permission("screen"|"microphone"|"systemAudio")` flow and status surface already present in `+page.svelte`.
- Model availability/download for OCR + transcription reuses the existing model-status/download logic (status: available / downloading + progress + cancel / missing + download).
- Ask AI stays OFF by default; onboarding surfaces the enable toggle + a disclosure that the Reasoning Engine (cloud Anthropic/OpenAI or local Ollama/Llamafile) must be configured, with a pointer/affordance into Settings → Intelligence. Deep provider/key configuration is NOT duplicated in onboarding beyond the lightweight provider/model + keychain affordance shown in the mockup.

### Segmented adoption in Settings (parallel workstream)
- Migrate single-choice controls that currently use `RadioGroup`/chip buttons to `Segmented` where there are a few short options: `capture/Video.svelte` (resolution mode, bitrate mode), `capture/Audio.svelte` (input mode), and the small provider/mode selects in `intelligence/Ocr.svelte` and `intelligence/Transcription.svelte`. Keep `RadioGroup` (or `Select`) where options are many or need per-option descriptions (`capture/Privacy.svelte`, `intelligence/UserContext.svelte`, provider/model lists). Preserve binding and persistence exactly; this is a visual/markup swap, not a behavior change.

### Front-end gotchas to respect (from prior work)
- Onboarding mount `$effect` must `untrack` its loaders or editing a draft re-runs init and reverts the edit.
- WKWebView doesn't focus `<button>` on click; any keyboard handling (accordion arrow nav, segmented arrows) needs a window capture-phase listener, not element `onkeydown`.
- Any memoized rendered output must live in a non-reactive `WeakMap`, not on `$state`, to avoid `state_unsafe_mutation`.
- Fill surfaces with `flex:1 1 auto` on a flex-column parent; avoid `height:100%` against a flex-stretched parent in WebKit.

## Testing Decisions

- **Type/compile gate:** `bun run check` must stay green after every slice (this is the primary frontend check in this repo).
- **Onboarding behavior to verify (component/integration where a harness exists, otherwise manual):**
  - Accordion opens exactly one row; expanding a second collapses the first.
  - Toggle flips enable/disable without expanding/collapsing (event isolation); disabled feature shows dimmed body.
  - Required rows: toggle is locked/non-interactive; lock + "Required" marker renders collapsed and expanded; no off-theme color.
  - Live footer count and "needs attention" reflect toggles, ungranted permissions, and undownloaded selected models.
  - Settings round-trip: a fresh onboarding produces the same `RecordingSettings` the old flow would for equivalent choices (map-and-save correctness).
- **Permissions & models:** verify grant → status transitions, and model missing → downloading (progress/cancel) → available transitions drive the "needs attention" state.
- **Segmented adoption:** for each migrated settings control, verify the bound value still reads/writes and persists, and that default selection matches the prior `RadioGroup` behavior; visual parity check in dark + light.
- **Manual walkthrough:** run the app (`bun run tauri -- dev`, sidecar prepared per CLAUDE.md), complete onboarding with a mixed on/off selection, confirm settings persisted and the recorder arms.
- **Do not test:** internal accordion/animation ordering or private helpers except through observable row state and the persisted settings; no new backend pipeline tests (no backend behavior changes here).

## Slices

1. **Shared section-icon module**
   - Goal: extract `SettingsRail` icon SVGs into an importable module and add a `lock` glyph; consume from the rail with no visual change.
   - Areas: `lib/settings/SettingsRail.svelte`, new `lib/settings/section-icons.ts` (or `Icon.svelte`).
   - Acceptance: rail renders identically; module exports each icon by id; `bun run check` green.
   - Depends on: none.
   - Parallel: yes (independent of everything else).

2. **Onboarding accordion shell**
   - Goal: presentational `FeatureStack` + `FeatureRow` (collapsed/expanded, icon chip, toggle, locked/required marker, accordion behavior, welcome header, footer CTA + live hint). Styling per the minimal/modern system in the mockup.
   - Areas: new components under `routes/onboarding/`.
   - Acceptance: a static feature list renders as the mockup does (dark + light); one-open accordion + independent toggle work; keyboard nav via capture-phase listener.
   - Depends on: slice 1 (icons).
   - Parallel: with slice 6.

3. **Feature model + state + persistence (decompose `+page.svelte`)**
   - Goal: define the ordered feature list (id, icon, required, eyebrow/title/sub, body ref), wire toggle/open state (M1), load/save against `recording_settings`, and replace the linear-step state machine.
   - Areas: `routes/onboarding/+page.svelte` (slimmed), new state/model module.
   - Acceptance: settings load into a draft and save on finish; `untrack` guards init; mapping covers all fields the old flow saved; files <800 lines.
   - Depends on: slice 2 (shell contract).
   - Parallel: no (defines the contract slice 4 consumes).

4. **Per-feature config bodies** (split into parallel sub-slices once 2+3 land)
   - 4a. Capture core: Permissions (pills + grant/open-settings), Screen capture (Segmented resolution/bitrate + fps/segment sliders + idle pause), Storage & retention.
   - 4b. Intelligence: OCR (provider/mode via Segmented + model download), Transcription (provider/model-size via Segmented + model download), Microphone, System audio.
   - 4c. Privacy (typeahead combobox + chips + apply-recommended) and Ask AI (disclosure + provider/model + keychain affordance, off by default).
   - Areas: new body components under `routes/onboarding/`; reuse shared controls + existing permission/model logic.
   - Acceptance: each body reads/writes its draft fields; dim-when-off; model/permission states drive "needs attention"; `bun run check` green.
   - Depends on: slices 2 + 3.
   - Parallel: 4a/4b/4c with each other after the model contract exists.

5. **Minimal/modern styling pass + cleanup**
   - Goal: apply the spacing/type/token system from the mockup across the new components; remove dead onboarding code (`ProgressArc`, `SceneShell` if unused) and old stage-frame/arc styles; verify dark + light.
   - Areas: `routes/onboarding/` styles, dead-file removal.
   - Acceptance: visual parity with `stack-a-polished.html`; no unused components; files <800 lines.
   - Depends on: slices 2–4.
   - Parallel: no (overlay/cleanup pass).

6. **Segmented adoption in Settings**
   - Goal: migrate the few-option single-selects in `capture/Video.svelte`, `capture/Audio.svelte`, `intelligence/Ocr.svelte`, `intelligence/Transcription.svelte` from `RadioGroup`/chips to `Segmented`; leave many-option/described selects alone.
   - Areas: the four panel files above.
   - Acceptance: value binding + persistence unchanged; default selection matches prior; dark/light parity; `bun run check` green.
   - Depends on: none.
   - Parallel: yes (independent of onboarding).

7. **Verification + docs/memory**
   - Goal: full `bun run check`; manual onboarding walkthrough in the running app; update `SUPPORTS.md` if behavior/platform notes change; refresh the onboarding-layout memory (fixed-frame note is now obsolete).
   - Areas: repo-wide check, docs.
   - Acceptance: check green; manual walkthrough completes and persists settings; docs updated.
   - Depends on: slices 2–6.
   - Parallel: no (final).

Parallel groups: [1, 6] → [2, 6] → [3] → [4a, 4b, 4c] → [5] → [7].

## Out of Scope

- No changes to the capture pipeline, recording lifecycle, or `RecordingSettings` schema.
- No deep Reasoning Engine configuration in onboarding beyond the lightweight enable + provider/model + keychain affordance (full config stays in Settings → Intelligence).
- No new capabilities/features; this reorganizes setup of existing ones.
- No Windows/Linux-specific onboarding work; permission flow stays macOS-first per current support.
- No localization/i18n of onboarding copy.
- No migration of described/long-list settings controls to `Segmented` (only the few-option single-selects).
- Not productionizing the HTML mockup itself — it's the reference, not shipped code.

## Further Notes

- **Risk — DRY vs coupling:** the cleanest reuse would be sharing the exact config UIs between onboarding bodies and the settings panels (`Video.svelte`, `Ocr.svelte`, …), but those panels are coupled to the carded `SettingGroup`/`SettingRow` layout and settings state. Decision/assumption: onboarding gets its own lighter bodies that reuse the shared *controls* and shared *logic helpers* (permission, model download), not the whole panels. Revisit extracting shared control clusters if duplication gets heavy.
- **Resolved — Retention control:** reuse `RetentionPicker` (parity with Settings, supports custom days); the mockup's segmented presets are not carried into the real build.
- **Resolved — save cadence:** persist only on finish (one atomic write). Abandoning onboarding mid-flow leaves prior settings untouched.
- **Decommission:** remove `ProgressArc.svelte` and the phase/step machine; confirm nothing else imports them before deleting.
- **Observability:** keep the existing onboarding logging for permission grants and model downloads so setup failures stay diagnosable.
