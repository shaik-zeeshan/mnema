# Plan: Subjects "Conviction view" redesign

## Problem

The Insights → **Subjects** tab is a flat grid of identical cards, each reducing a subject to a single confidence number. That buries the one thing that is actually unique and "headline" about this data: a belief's **confidence moves over time** — it warms with fresh evidence, cools on its own during silence, and drops when contradicted — and a subject is never one score but a *stack* of conclusions, each with its own arc. Users can't see, at a glance, what Mnema is sure of, what is still forming, or what is fading — nor watch it change as the engine works. We need the surface to be organized around **conviction and movement**, with each subject's movement arc as the hero, and to update **in real time** without yanking the page out from under the reader.

Approved design mockup: `docs/user-context/mockups/subjects-index.html` (the "Conviction view").

## Solution

Replace the card grid in `apps/desktop/src/lib/insights/Subjects.svelte` with a tiered layout that groups subjects by **conviction** (how firmly held) or **movement** (which way trending), with each row's hero element a **multi-line sparkline** (one faint line per conclusion) drawn from real, lazily-fetched confidence history. Clicking a row still opens the existing full `SubjectDetail` surface; an inline expand offers a quick look with per-conclusion confidence bars, evidence, and per-conclusion Pin/Dismiss. The view stays live via the existing `user_context_changed` event — debounced and buffered into a "refresh" pill so the page never reflows or re-tiers while the user is reading.

## User Stories

1. As a user, I want subjects grouped by how firmly Mnema holds them, so that I can see at a glance what it is sure of versus still figuring out.
2. As a user, I want each subject to show its movement arc (one line per conclusion), so that I can see how a belief has warmed, cooled, or faded — not just a single number.
3. As a user, I want to flip between a "what I know" (conviction) and "what changed" (movement) view, so that I can read the surface either way.
4. As a user, I want to expand a subject in place to see its individual conclusions, the evidence behind them, and to pin or dismiss each one, so that the engine's beliefs are explainable and correctable.
5. As a user, I want the surface to update as the engine forms new beliefs without the page jumping, so that it feels live but stays readable.
6. As a new user with only a couple of subjects, I want a simple list rather than mostly-empty tier headers, so that the surface doesn't look broken when sparse.

## Implementation Decisions

- **Scope / placement.** This replaces the card-grid body of `Subjects.svelte` (the index). It does **not** replace `SubjectDetail.svelte`; clicking a row still navigates to the full detail (the big multi-line trajectory chart + evidence). Inline-expand is a quick look, not the deep dive. `onOpenSubject(subject)` prop stays.
- **Data sources (reuse existing).**
  - `list_user_context_conclusions` ({ includeFaded: true }) — primary list, already loaded.
  - `get_user_context_subject` ({ subject }) — per-subject `trajectories` (real `ConfidenceSnapshot` history) for the hero arcs, fetched **lazily with bounded concurrency** (the existing `loadTrajectories` worker pattern, CONCURRENCY 4). Missing/empty history falls back to a flat baseline from current confidence (existing `buildSpark` fallback).
- **Subjects are still derived client-side** by grouping conclusions on `subject` (existing `groupSubjects`). Tier membership keys off the subject's top (highest-confidence) conclusion.
- **Conviction tiers map to the engine's real thresholds**, not round numbers (constants live in `crates/app-infra/src/user_context/.../confidence.rs`):
  - **Fading · kept for history** — `status === "faded"` (below `DISPLAY_FLOOR` 0.15). Rendered dimmed, sunk to the bottom, never deleted.
  - **Just taking shape** — top confidence below ~`INITIAL_BASE` (0.30), not faded.
  - **Forming** — ~0.30 to the Strongly-held cutoff.
  - **Strongly held** — top confidence ≥ **0.68** (chosen: no engine constant exists for this boundary; single source-of-truth constant in the component, easy to tune).
- **Movement tiers** (toggle): **Warming / Steady / Cooling** (from `deriveTrend`, the existing first-vs-last trajectory delta) + **Fading · kept for history** (faded). Within movement tiers sort by recency (`lastMovedAtMs` desc); within conviction tiers sort by confidence desc. Empty tiers render no header.
- **Grouping toggle** uses the canonical `.sort-seg` segmented control: "By conviction" (default) / "By movement". The existing Most active / Recently moved / A–Z sort is folded into per-tier ordering; if still wanted, keep it as a secondary control — otherwise drop it (decide in slice 1).
- **Sparse state.** Below **5** subjects, skip tiers entirely and render one ungrouped list (sorted by confidence desc), so early users don't see mostly-empty headers.
- **Row anatomy.** Category dot + name + pin marker, a trend pill (warming=accent / steady=muted / cooling=danger), conclusion count, 1-line headline, and the hero **`Sparkline.svelte`** (reused as-is: `series` = one entry per conclusion, `floor` = 0.15) plus top confidence + last-moved.
- **Inline expand.** Ranked conclusions, each with a `confidence-bar`, status chip, and its **own Pin / Dismiss** controls — Pin/Dismiss are **per-conclusion** (`user_context_set_pinned` { id, pinned }, `user_context_dismiss_conclusion` { id }), matching the engine model where Pin protects a single conclusion from decay and Dismiss rejects it. A subject-level "pin" (if added) is sugar that pins all conclusions. Evidence refs link out via `open_capture_result_in_main_window`. "Also informs" related-subject chips are optional (no edges source exists in the real backend yet — defer unless cheap).
- **Realtime.** Keep the existing `listen("user_context_changed", …)` subscription. Change the handler from "reload immediately" to:
  - **Debounce** reloads ~500ms (derivation passes fire bursts).
  - Reload into a **staging buffer**; if the resulting set/order differs from what's displayed, show a small accent **"{n} views updated · refresh"** pill instead of reflowing. Apply the staged data on pill click, or automatically when the list is idle and scrolled to top.
  - **Never re-tier or reorder a row that is currently expanded**; hold it in place until collapsed/refreshed.
  - For the currently-focused/expanded subject, allow its **arc to extend** with fresh trajectory points without moving the row.
- **Motion discipline.** Static at rest; only snappy ~0.12s transitions on hover/expand/refresh; honor `prefers-reduced-motion`. No canvas / rAF / ambient animation (consistent with the mockup and `Sparkline.svelte`).
- **Open question:** exact **Strongly-held** cutoff (currently 0.68) — confirm with product; everything else is anchored to engine constants.

## Testing Decisions

- **Tiering logic** is the highest-value unit to test: extract pure helpers (`tierFor(subject, axis)`, threshold mapping, sparse-state collapse at <5) and cover boundary cases — confidence exactly at 0.15 / 0.30 / 0.68, all-faded subject, single-subject and 4-vs-5-subject sparse boundary.
- **Trend derivation** already has behavior via `deriveTrend`; add cases for flat/short history (≤1 point → falls back to status) and faded→cooling.
- **Realtime buffering**: verify a `user_context_changed` burst within the debounce window triggers one reload; verify an expanded row is not reordered while open; verify the refresh pill count reflects added/removed subjects. Prefer testing observable state (pill shown, order held) over internal timers — inject the debounce delay so tests don't wait real time.
- **Sparkline reuse**: no new chart tests; rely on existing `Sparkline.svelte` coverage. Verify the index passes one series entry per conclusion and the floor prop.
- **Manual checks**: `bun --cwd=apps/desktop run check`; open the app with the engine on, confirm tiers populate, expand a row, pin/dismiss a conclusion and see it move, watch a live update surface the pill rather than jumping.
- **Do not test**: visual exact pixel layout, the synthesized trajectories from the mockup (real build uses real history), or `SubjectDetail` (unchanged).

## Slices

1. **Tiering + grouping core (pure logic).**
   - Goal: pure helpers for conviction/movement tiers, threshold constants mapped to engine values, sparse-state collapse, per-tier sort.
   - Areas: `Subjects.svelte` script (or a colocated `subjectsTiers.ts`).
   - Acceptance: unit tests cover all tier boundaries + sparse <5 collapse.
   - Depends on: none. Parallel: yes.
2. **Conviction layout + hero sparkline rows.**
   - Goal: render tiered sections with `.section-title` headers and rows; hero `Sparkline.svelte`; trend pill, headline, counts; reuse lazy `loadTrajectories` with flat fallback.
   - Areas: `Subjects.svelte` template + styles (port from `docs/user-context/mockups/subjects-index.html`).
   - Acceptance: matches mockup; arcs draw from real trajectories, fall back flat when absent; `bun run check` clean.
   - Depends on: slice 1. Parallel: with slice 3 once row markup contract is set.
3. **Inline expand: conclusions, evidence, per-conclusion Pin/Dismiss.**
   - Goal: expand-in-place with ranked conclusions (`confidence-bar` + status), per-conclusion `user_context_set_pinned` / `user_context_dismiss_conclusion`, evidence → `open_capture_result_in_main_window`. Row click still calls `onOpenSubject`.
   - Areas: `Subjects.svelte`.
   - Acceptance: pin/dismiss update a single conclusion and reflect in the arc; detail navigation unchanged.
   - Depends on: slice 2. Parallel: no.
4. **Realtime refresh-pill behavior.**
   - Goal: debounce `user_context_changed` ~500ms, stage into buffer, show "{n} views updated · refresh" pill, never re-tier an expanded row, extend focused arc live.
   - Areas: `Subjects.svelte` event handling.
   - Acceptance: burst → one reload; pill shown instead of reflow; expanded row held; tests with injected debounce.
   - Depends on: slice 2 (display state to diff against). Parallel: with slice 3.
5. **Grouping toggle + polish.**
   - Goal: `.sort-seg` "By conviction / By movement" toggle, honest summary counts line, footer note, reduced-motion, empty/engine-off states.
   - Areas: `Subjects.svelte`.
   - Acceptance: toggle re-groups; counts correct; reduced-motion disables transitions.
   - Depends on: slices 1–2. Parallel: after layout exists.

Parallel groups: [1], [2 after 1], [3 and 4 after 2], [5 after 2].

## Out of Scope

- Changes to `SubjectDetail.svelte` (the deep trajectory chart + evidence inspector) beyond it remaining the click-through target.
- New backend commands, schema, or events — this reuses the existing invokes/event entirely.
- A real "related subjects / also informs" edges source (the mockup's `edges` are synthetic; defer until a backend signal exists).
- Backfilling confidence history; arcs degrade to flat baselines where history is missing.
- Bulk actions, fade-rate controls, or any new user-tunable confidence dials (engine policy intentionally exposes only Pin/Dismiss).

## Further Notes

- Risk: per-subject `get_user_context_subject` is N fetches on the index. Mitigated by the existing bounded-concurrency lazy loader + flat fallback; arcs fill in progressively. If it ever feels heavy, consider a batched trajectories endpoint later (out of scope now).
- The threshold constants (0.15 floor, 0.30 base) should be referenced as named constants in the component with a comment linking to `confidence.rs`, so they stay in sync if the engine policy changes.
- Keep all color token-driven and reuse app primitives (`.card`, `.section-title`, `.confidence-bar`, `.pill`, `.chip`, `.btn`, `.sort-seg`) — the approved mockup already does, and it matches the rest of Insights.
- After landing, update `docs/user-context/mockups/README.md` / any ADR if the Subjects surface contract is documented there.
