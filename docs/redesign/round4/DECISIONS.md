# Round 4 — resolved decisions (grilled 2026-08-05)

Founder-grilled resolutions for every feature the five round-4 mockups introduce (G1–G11),
plus the implementation-workflow decisions (W1–W4). This file is the orchestrator's input for
implementation. Grill transcript context: `/private/tmp/session-round4-feature-grill.html`
(Part II carries the same resolutions inline).

**Direction pick: deliberately deferred.** All five directions (`01-bento-native` …
`05-tactile-instruments`) stay live for testing. Everything below is **direction-independent
and binding**; only two things remain per-direction: the current-frame bar's geometry and the
settings navigation shape.

Two decisions below (G4, G5) **amend settled records on purpose** — do not flag them as
regressions against the round-2/round-3 decision files.

---

## G1 — Current-frame source: live capture with a self-excluding content filter

- **Live screenshot at invoke time** — never the pipeline's newest frame (stale, and absent
  exactly when the feature is most wanted: while not recording).
- The capture uses a **ScreenCaptureKit content filter that excludes (a) Mnema's own windows
  and (b) privacy-listed apps**. Self-exclusion in the filter — not capture-timing tricks — is
  the mechanism. The mockups' "capture before the bar draws" race is rejected: with the filter,
  capture is clean on invoke, on re-grab, and on follow-ups with the answer panel open, and the
  on-screen outline (G3) never appears in its own shot.
- Excluded apps — including an excluded *frontmost* app — are **blanked and named** in the
  context chip ("1Password excluded"). Never silently dropped; never a refusal (the user may be
  asking about something else on the same screen).
- Same exclude-list source as the system-audio tap (the existing parity rule).

## G2 — Non-vision models: upfront disclosure, OCR-text fallback

- The feature stays available on non-vision models; it is never hidden.
- Disclosure is **upfront, before the user types**: when the overlay collapses, the context
  chip already states "⟨model⟩ can't see images — sending text from this screen".
- On send, the frame's **OCR text + window metadata** (app, window title, timestamp) go
  instead of pixels. Never silently degrade.
- `resolve_engine_config` gains a vision-capability dimension.

## G3 — Current-frame anatomy: four points bind; outline overlay in v1

Binding across all five directions:

1. **Same window collapses** — the Quick Access window itself shrinks to the bar; never a
   second window. ⌘O grows it back.
2. **Context is a chip-in-sentence** — small inline chip, deleted like a word. Never a large
   thumbnail.
3. **Answer is a detached second piece** — dismissing the answer never hides the
   controls/Stop.
4. **Capture is implicit** (collapsing *is* the gesture — no "attach screen?" modal);
   **indication is explicit**.

The **on-screen outline overlay** ("Mnema is reading this screen" — transparent, click-through
overlay window) **ships in v1**, as its own slice behind the core feature. Implicit capture is
only defensible when indication is maximally explicit; the pair was designed together. Bar
geometry (560×40 / 640px / 720×48…) stays per-direction.

## G4 — Ask as ranked row: ADOPTED, round-3 anatomy amended  *(conflict, resolved)*

- **One field, no Search/Ask segmented control.** "Ask AI about '⟨query⟩'" is a ranked row in
  the search results (promoted to selection when search returns nothing); selecting it
  transforms the surface into the visually distinct Ask mode.
- **Round-3 `design.html` frame 08/09 anatomy (segmented control) is formally superseded.**
- Rationale: search is the free default posture; ask is a visible escalation that costs a
  model call. All five directions converged on this independently.

## G5 — Hour/Day/Week zoom: IN, span-only  *(conflict, resolved)*

- The zoom segmented control returns: **Hour / Day / Week — no Month level** (the jump menu's
  month grid covers month-scale navigation).
- Zoom owns **span**; the position pill (G6) owns **position**; the from/to date-range pair
  stays dead.
- The round-2 rejection is **amended for zoom alone**. It was rejected as part of the whole
  NLE idiom (hour rulers, density bars, filmstrips, NLE lanes) — the rest of that rejection
  stands.

## G6 — Jump menu: one cached query, fixed contents

- Per-day coverage = **one GROUP BY over the existing segments table**, cached, invalidated on
  capture/retention events. **No new tables.** Ships in the same slice as the position pill.
- Menu contents: quick targets **Now / This morning / Yesterday**, then **seven day rows**
  (captured hours + coverage bar each), then the **month grid**. Days with no recording are
  **disabled** — you can never land on an empty day.
- 04's type-a-date field is **dropped from v1**; revisit only if the friction is felt.

## G7 — Settings: autosave pattern binds, ⌘F in, Undo out

- **No bottom save bar, ever.** Autosave = a persistent chip in a top-anchored, unclippable
  strip + a row-level "Saved ✓" echo (~1.5 s) on the changed row. Failure is the only
  persistent state and also raises a toast (which never auto-dismisses).
- **⌘F row-filtering is IN**: every settings row indexed (label + synonyms + section); hits
  render with breadcrumb and their *live control*. Index completeness is enforced by a test —
  every registered setting must have an index entry.
- **Settings-Undo is OUT for v1** (02/04 drew it). The chip + row echo answer the anxiety the
  founder actually named; an undo stack is speculative machinery.
- Navigation shape (toolbar tabs / filtered scroll / floating rail / tabs+⌘F) stays
  per-direction.

## G8 — Consequence denominators: honest numbers only

- **A denominator ships only where the value is real on this machine**: fps→GB/day, RAM total
  vs model sizes, retention→GB kept, OCR backlog count, disk free, semantic index size.
- Model sizes must come from the **corrected registry** (speakrs is 419 MB, not 31 MB; real
  model set ≈ 1.1 GB).
- **Dropped: all °C claims** (the duty-cycle bar shows both halves + frames/hour and says
  nothing about temperature) **and minute-precise ETAs** — round coarsely ("~20 min",
  "under an hour").
- **05's instrument rule is the app-wide review standard** for any future custom input: name
  the physical quantity and write the consequence as a fraction of something real, or it stays
  a stock control.

## G9 — Shortcut conflicts: honest copy, no scanning

- **In-app** conflicts are detected and named with their owner ("already used by Quick Access
  — ⌘⌥Space").
- **External** register-failures say "this shortcut is taken by another app — try a different
  combination" — no name. macOS has no API to enumerate other apps' hotkeys; no event-tap
  scanning hacks (fragile, input-monitoring TCC, still guessable-wrong).
- Mockup copy ("taken by CleanShot X") is amended accordingly.

## G10 — Semantic coverage meter: conditional, price-before-enable

- **Shipping default stays OFF** (zero embeddings in prod, per the settled benchmark
  decision).
- The coverage meter renders **only when semantic search is enabled**.
- The off state adopts **05's price-before-enable pattern as binding copy**: the row states
  what enabling costs (index size + processing time), computed honestly per G8 from the user's
  actual frame count — not the mockups' fiction.

## G11 — Overview tiles: This Week + Ask history in; Open Threads is prose

- **This Week** (7-day capture bars — same query family as G6) and **Ask history**
  (conversation store read) are **IN**.
- **Open Threads v1 = digest prose only**: surface the digest's existing "one open thread…"
  sentence where 03 drew the tile. **No entity, no table, no extraction pipeline.** Wanting to
  mark one resolved is the signal to design the entity — not before.
- The standing decision holds: the bento ships last, against real data.

---

## Workflow decisions (W1–W4, grilled 2026-08-05, second pass)

### W1 — Work unit: strict 1:1:1

One slice = **one worktree = one branch = one PR**, no exceptions. An oversized slice is
split into *more slices* (more worktrees, more PRs) — never into two PRs from one worktree.
Every live worktree = one in-flight PR, so `orca worktree ps` stays a truthful dashboard.
No stacked-PR tooling; a slice depending on an unmerged one branches from the open PR and
retargets to main after it merges.

### W2 — Two phases: shared machinery on main, then five direction worktrees

- **Phase 1 — shared machinery, built once, on main.** Everything identical across all five
  directions lands first via feature worktrees, mostly sequential: the capture filter (G1),
  vision fallback (G2), ask-as-ranked-row logic (G4), zoom (G5), jump-menu query (G6),
  settings ⌘F index (G7), honest denominators (G8), and the substrate (system.css dark
  faint-text fix — `--app-text-faint` ≈ 1.5:1 in dark, fix at the root — token block, shared
  `.btn`, state pill, toasts, per the round-3 sequence).
- **Phase 2 — five direction worktrees, one per mockup.** Each direction gets one worktree;
  its orchestrator fans out sub-agents to skin the app in that direction's style on top of the
  phase-1 machinery. Parallel vs sequential *inside* a worktree is decided by file overlap.
  Direction worktrees **never merge to main** — they exist to test the five candidates as
  real apps before the founder picks.
- **Models: orchestrator = Fable or Opus; sub-agents = Opus 5.**

### W3 — No standing founder gates; decision precedence

- **No standing gates.** Phase 1 has no open founder decisions; in phase 2, each direction's
  own mockup + its `07-components.html` deviations list *is* the spec for bar geometry and
  settings-nav shape.
- **Precedence binds: G-decisions > direction mockup > agent judgment.** Agents apply the
  G-amendments over the mockup pixels — G8 deleted the °C claims, G9 weakened the conflict
  copy, G6 dropped type-a-date; copying the mockup verbatim there is a bug, not fidelity.
- `gate-create` (orchestration skill) is reserved for genuine contradictions only — a mockup
  vs a G-decision, or a backend fact that breaks a mockup's promise. The worker blocks and
  asks instead of improvising.

### W4 — Per-worktree setup baked into creation

- Every Rust-touching worktree pays: `bun install` → `bash
  scripts/prepare-mnema-cli-sidecar.sh debug` (required for any `src-tauri` cargo
  invocation) → source `scripts/openblas-build-env.sh` for direct cargo calls. Bake these
  into the worktree-create step / orca per-workspace env recipe so agents don't rediscover
  the sidecar failure one by one.
- **Pure-UI slices** (`bun run check` only) skip the cargo-side steps and stay cheap; the
  recipe states which steps apply per slice kind.
- Run **`orca skills install`** once before phase 2 (as of 2026-08-05 only `orchestration`
  was in the session skill list; `orca-cli` was missing).
