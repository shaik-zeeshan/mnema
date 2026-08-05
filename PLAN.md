# Plan: Mnema full-app redesign — implementation

Source of truth: `docs/redesign/` (README.md rules · DECISIONS.md binding record ·
system.css implementable system · mockups/design.html the design). This plan is the
grill-final 10-step sequence turned into slices. Where this file and DECISIONS.md
disagree, DECISIONS.md wins.

## Problem

The app's UI grew feature-by-feature: type is 100% mono at six unrelated sizes with no
usage rules, `.btn` is re-declared in ~10 places, five uncoordinated error mechanisms
reflow content, recording chrome is a button cluster the founder doesn't trust, search is
a destination instead of a tool, and audio surfaces as minutes instead of conversations.
The redesign (three mockup rounds, grilled twice, founder-signed) fixes all of it with one
native-macOS visual system — it now needs to be built.

## Solution

Land `system.css` (type roles, spacing, primitives) into the app, then build outward in
the settled order: shared Button → state pill recording chrome → per-source mask →
Timeline/Overview switcher → Quick Look search grid → toast system → conversations/moments
backend queries → the Overview bento last, against real data. The Timeline is FROZEN
(re-typed only). Same window set as ships.

## User Stories

1. As a user, I want one calm recording pill with a transport popover, so that recording
   state is always visible without a button row dominating the title bar.
2. As a user, I want to toggle individual sources mid-session, so that I can mute the mic
   without stopping the whole recording.
3. As a user, I want to pick which surface (Timeline or Overview) the app opens on, so
   that my main workflow is one keystroke away.
4. As a user, I want Search and Ask in the Quick Look window with a large frame grid, so
   that recall is a summonable tool, not a place I navigate to.
5. As a user, I want errors as toasts that never move what I'm reading, so that a failed
   background job doesn't yank the frame I'm inspecting.
6. As a user, I want audio presented as conversations ("Launch sync · 38 min · 5
   speakers"), so that I recall moments, not minutes.
7. As a user, I want an Overview bento of today — digest with citations, moments strip,
   conversations, capture/storage state — so the AI surface is a real dashboard.

## Implementation Decisions

All from DECISIONS.md; the load-bearing ones:

- **Timeline FROZEN**: `routes/+page.svelte` from the thin bar down renders as shipped;
  only shared type roles + spacing constants may touch it. New title bar sits above it.
- **Two surfaces, ⌘1/⌘2 switcher**; out-of-box default Timeline; `app_settings` key
  `ui.main_surface`. Search+Ask live only in Quick Look (1120×720 fixed, 3-up 349×196
  grid). The Overview Ask field is a *launcher* into Quick Look — never an answer surface.
- **Materials are CSS** (`backdrop-filter`); NSVisualEffectView deferred indefinitely.
  Scroll-under-chrome is Overview-only (frozen Timeline is opaque).
- **State pill**: dot + elapsed + cost capsule, degradation ladder, DOM popover with
  Stop/Pause first; tray keeps full native transport in `status_bar.rs`. "270 MB" needs a
  new bytes-captured-today stat command.
- **Per-source mask**: a user-scoped per-source mask routed through the existing
  paused-flag seam in `native_capture/lifecycle.rs`. Until it lands, popover source
  toggles render disabled-while-recording (today's tray semantics). Tray + shortcut
  labels must always match the popover.
- **Conversations**: no new entity — read-time JOIN of `user_context_activities` ×
  overlapping `speaker_turns`. Bar: total overlapping turn time ≥ 2 min (one knob, no
  speaker-count minimum; media passing the bar is accepted). Speakers = distinct clusters;
  spilling turns extend *displayed* duration only. No migration, retroactive. Freshness
  lag ~5–40 min accepted; tile shares the digest's "updated HH:MM" semantics.
- **Moments strip v1**: the day's activities' headline frames (`is_headline`, migration
  0046) ordered by a focus+duration heuristic. No ranking infrastructure.
- **Dialogs**: confirms/alerts always `@tauri-apps/plugin-dialog` (mockup chrome not
  followed, content only). In-DOM `Dialog.svelte` solely for rich sheets (Settings sheet,
  CLI consent card).
- **Toasts**: bottom-right, stack to three, overlay, errors never auto-dismiss, bell
  archives; `.timeline__stage-status` folds in (acks → success, errors → danger).
- Border ceiling counts containers, not control rings. Errors never reflow. One
  `--t-display` per screen.

## Testing Decisions

- **Verify UI by rendering, not grepping** (memory `mnema-verify-ui-by-rendering`):
  screenshot the running app / rendered pages and look, both themes, at 800×600 and
  1100×720 for affected surfaces.
- `bun run check` per UI slice; `cargo test -p app-infra` for the conversations/moments
  queries; `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib` for pill
  state + mask + status_bar changes (respect existing status_bar test conventions).
- Conversations JOIN gets fixture tests: below/at/above the 2-min bar, distinct-cluster
  counting, spill-extends-displayed-duration, zero-turn activity excluded.
- Per-source mask: lifecycle tests that the mask composes with inactivity-pause flags and
  that tray/popover/shortcut state stay in sync.
- If any wire shape lands in `capture-types` ↔ `lib/insights`, keep the serde round-trip
  test green (hand-mirrored, no codegen).
- Do not test the frozen Timeline's internals; only that re-typing didn't change layout.

## Slices

1. **Dark surface steps fix (`system.css`)**
   - Goal: retune the three dark surface tokens (currently ~7 sRGB apart) for
     region-scale separation; every later dark-mode surface depends on them.
   - Areas: `docs/redesign/system.css` (§1 tokens + KNOWN GAPS note), re-render
     `design.html` both themes to confirm.
   - Acceptance: regions separable in dark render; gap 1 removed from KNOWN GAPS;
     README's "one deliberate token change" sentence stays true.
   - Depends on: none. Parallel: with slice 9 (backend).

2. **Land `system.css` into `+layout.svelte`**
   - Goal: type ramp, spacing constants, control metrics, motion rules, primitives in the
     app; replace the `--text-*` family per §2's migration map; body 12→13.
   - Areas: `+layout.svelte` `:root` + global styles; every `--text-*` consumer.
   - Acceptance: no `--text-*` left; `bun run check`; render both themes — frozen
     Timeline re-typed but structurally unchanged.
   - Depends on: 1.

3. **Shared Button**
   - Goal: one `.btn` (`lib/components/Button.svelte`), variants push/default/ghost/
     danger/sm/lg/icon.
   - Areas: the ~10 declarations in frame 17's Button row (+page.svelte scoped `.btn`,
     insights shells, `.titlebar__record`, `.license-banner__btn`,
     `.quick-recall__state-btn`, `.gate-cta`, settings buttons).
   - Acceptance: one definition, all call sites migrated, render check.
   - Depends on: 2.

4. **State pill**
   - Goal: `RecordPill.svelte` replaces the title-bar cluster; all frame-11 states +
     ladder 0–3; DOM popover, Stop/Pause first; source toggles present but
     disabled-while-recording.
   - Areas: `+layout.svelte` title bar, new component, new Tauri stat command
     (bytes captured today) in `lib.rs`.
   - Acceptance: nine states renderable, ladder collapses at narrow widths, popover
     matches tray semantics; the old `.rec` cluster CSS does not ship.
   - Depends on: 2, 3. Parallel: with 6, 7, 8.

5. **Per-source mid-session mask**
   - Goal: user-scoped per-source stop/start routed through the paused-flag seam; enable
     the popover toggles; tray + global-shortcut labels match.
   - Areas: `native_capture/lifecycle.rs`, Tauri commands, `status_bar.rs`, RecordPill.
   - Acceptance: mask composes with inactivity flags; mic release/restore verified;
     tray/popover/shortcuts one behavior; lifecycle tests.
   - Depends on: 4.

6. **Surface switcher + default-surface setting**
   - Goal: `SurfaceSwitcher.svelte` (⌘1/⌘2), `app_settings` key `ui.main_surface`
     (default `timeline`), read at window open; "Open Mnema here" affordance.
   - Areas: `+layout.svelte`, settings, one trivial backend key.
   - Acceptance: switch + persistence + correct open surface; render check.
   - Depends on: 2, 3. Parallel: with 4, 7, 8.

7. **Quick Look search + grid + Ask handoff**
   - Goal: search moves into Quick Look; 3-up 349×196 fixed-16:9 `ResultCell` grid
     (≤50 cells in DOM), scope chips, oversized field (`--t-title` @ 400), Ask mode ⌃⏎;
     result click hands off to Main focused on Timeline at that instant.
   - Areas: `routes/quick-recall/`, new `ResultCell.svelte`, `Chip.svelte`.
   - Acceptance: grid at fixed 1120×720, empty/no-match/error states per frame 10,
     handoff works; render check.
   - Depends on: 2, 3. Parallel: with 4, 6, 8.

8. **Toast system**
   - Goal: `Toast.svelte` + `toastStore`; bottom-right stack of three + "+N more";
     errors sticky, info/success auto-6s; bell archive; fold `.timeline__stage-status`.
   - Areas: `+layout.svelte`, timeline status call sites.
   - Acceptance: toasts never reflow content; stage-status gone; render check.
   - Depends on: 2, 3. Parallel: with 4, 6, 7.

9. **Conversations JOIN + moments queries (backend)**
   - Goal: app-infra read-time queries — conversations-for-day (activities × overlapping
     `speaker_turns`, ≥2 min bar, distinct-cluster speaker count, spill-extended display
     duration) and moments-for-day (headline frames ordered by focus+duration); Tauri
     commands exposing both.
   - Areas: `crates/app-infra` (query code, no migration), `lib.rs` commands,
     `capture-types` if a wire shape is needed.
   - Acceptance: fixture tests per Testing Decisions; `cargo test -p app-infra`.
   - Depends on: none. Parallel: can start immediately alongside 1.

10. **Overview bento — last, against real data**
    - Goal: the converged frame-04 surface: `Tile.svelte`, `DayStrip` moments strip,
      digest with frame/conversation citations, Conversations tile → AudioDrawer route,
      Context tile, Capture/Storage tiles, Ask-field launcher into Quick Look, engine-off
      state (frame 06), scroll-under material toolbar, 800×600 drop ladder (hours hero
      stays; storage line drops).
    - Areas: `lib/insights/`, Overview route.
    - Acceptance: renders against real data from slice 9 (never ships hollow); three
      widths, both themes; launcher never renders an answer in Overview.
    - Depends on: 4, 6, 7, 8, 9.

Parallel groups: [1, 9] → [2] → [3] → [4, 6, 7, 8] → [5] → [10].

## Out of Scope

- Any Timeline change beyond shared type/spacing tokens (FROZEN — two corrections paid).
- NSVisualEffectView / behind-window vibrancy (deferred indefinitely).
- Media-vs-conversation filtering, conversation sessionization entity, ranking
  infrastructure, live in-progress conversation rows.
- New windows; Quick Look resizing; masonry grids.
- Styled confirm/alert dialogs (native plugin-dialog only).
- Fix-when-hit system.css gaps: text-over-image, oversized-input role, object-size ramp.

## Further Notes

- `design.html` is the spec; its frame 17 maps every component to code. Verify by
  rendering (Playwright recipe in memory), never by grepping class names.
- Slice 5 is the one deep native-capture change — keep it a thin adapter over
  `lifecycle.rs` per CLAUDE.md boundaries.
- Slice 9's freshness depends on the existing derivation worker cadence (2–10 min beat);
  no new scheduling.
- After each slice lands, update `docs/redesign/DECISIONS.md` only if a decision changes;
  the design mockup is not maintained against the implementation.
