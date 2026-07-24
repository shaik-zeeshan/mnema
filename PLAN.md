# Plan: Warm Paper redesign & story-first shell

Design spec: [`docs/mockups/unified-shell/DESIGN.md`](docs/mockups/unified-shell/DESIGN.md).
Pinned mockup: [`docs/mockups/unified-shell/main-surface/story-first-v5.html`](docs/mockups/unified-shell/main-surface/story-first-v5.html);
per-surface anatomy in [`docs/mockups/unified-shell/app-match/`](docs/mockups/unified-shell/app-match/).

## Problem

Mnema's window opens onto the raw frame timeline — the machine's-eye view — while the story of the
user's day (journal, meetings, recaps) hides behind an Insights toggle, and automations live on a
third surface. Meetings, the flagship use case, have no home of their own: a recap is findable only
as a chat-rail row. Creating a trigger starts with a mechanism question ("pick a Condition") instead
of intent. The visual identity (dark terminal) undersells a memory product to non-technical users.
Competitors (Littlebird) feel friendlier while doing structurally less.

## Solution

Make the story layer the app: a Warm Paper (light-first, warm-dark sibling) shell whose home is the
day's journal — greeting, real digest paragraph, composer with activity-drawn suggestions, and a
ledger-prose river — with evidence one click away in a span-scoped receipt drawer and the raw
timeline demoted to a titlebar door. Add a Meetings surface keyed off detected meetings, a
template-gallery first step for trigger creation, and recast capture controls as "the record"
(status pill, timed off-the-record with auto-resume, live edge showing the state in the story).

## User Stories

1. As a user, I want the app to open onto a readable story of my day, so that I get value without learning a scrubber.
2. As a user, I want every meeting Mnema detected listed in one place with its summary and transcript, so that meetings are findable without remembering which chat a recap landed in.
3. As a user, I want to create a trigger by picking a ready-made template, so that I express intent, not mechanism.
4. As a user, I want to go off the record for a bounded time that resumes by itself, so that a private call never silently costs me the afternoon.
5. As a user, I want to click any activity and see the frames/audio behind it, so that I can verify what the story claims.
6. As a user, I want a light, warm interface (with a dark sibling), so that the app feels like a journal, not a terminal.

## Implementation Decisions

- **Two tracks:** (A) surface/architecture work, (B) Warm Paper retheme. Track B is CSS-token work spanning every stylesheet and must not block Track A; new surfaces are built token-clean so the retheme is a token swap, not a rewrite.
- Rail nav: Today / Meetings / Subjects / Triggers. `/triggers` route folds into the shell rather than a separate page shell; Timeline remains its own surface behind the titlebar icon. Overview.svelte and Context.svelte retire; Context's content merges into Subjects.
- The journal river keeps `JournalRiver`'s data model; ledger-prose is a render change. The receipt drawer reuses `ReceiptViewer`/receipt-lane/receipt-playback scoped by activity span.
- Meetings surface reads the meeting detector's ledger (the same detection that fires Meeting Ends — meetings are recorded whether or not a trigger fires; verify the ledger persists non-firing holds, else extend it). Transcript tab reads speaker turns for the window; Summary links the trigger-run conversation. Notes = one text column on the meeting row.
- Suggestion chips v1 are mechanical templates filled from real data (last meeting, top app before now) — no LLM call to render the empty state.
- Digest paragraph reuses the existing digest pipeline; the front page renders its latest output with re-read.
- Timed off-the-record: implemented in the Rust capture lifecycle (`native_capture/lifecycle.rs` seam) as a pause-with-deadline; auto-resume must survive app restart (persist the deadline) and sleep. Status pill/tray reflect the countdown; tray items get the same "record" language.
- Trigger templates ship in-app (static list), each = real `TriggerCondition` + prompt prefill; gallery lands on Review with the Import mechanic. No online gallery.
- Wire shapes stay hand-mirrored `capture-types` ↔ TS per existing convention; new commands registered in `lib.rs`.
- Assumption to verify early: meeting-hold evidence rows exist independently of trigger firings; if not, that write is slice 1's first task.

## Testing Decisions

- Rust: unit tests for the meetings-ledger query surface (day grouping, states incl. processing/skipped), timed-pause deadline persistence + auto-resume across restart simulation, serde round-trips for new wire types.
- Frontend: bun tests for suggestion-chip fill logic and digest-placement state; keep the conversation serde round-trip test green.
- **UI verified by rendering, not grepping** (per repo memory): every surface slice ends with a Playwright screenshot compared against its mockup frame.
- Manual drills: off-the-record 15-min auto-resume with app restarted mid-window; meeting with no recap trigger appears transcript-only; skipped-run meeting shows quietly.
- Not tested: pixel-parity of the retheme, tray icon rendering.

## Slices

1. **Meetings backend contract**
   - Goal: `list_meetings` / `get_meeting` commands — day-grouped detected meetings with state, speakers, provenance; notes column.
   - Areas: app-infra (ledger/query + migration if needed), capture-types, lib.rs.
   - Acceptance: unit tests for grouping/states; transcript-only + processing + skipped states representable.
   - Depends on: none. Parallel: yes.
2. **Rail + shell restructure**
   - Goal: four-item rail, tightened-B history, retire Overview/Context (Context → Subjects), titlebar timeline icon, drop the segmented surface switcher.
   - Areas: InsightsRail/RailHistory, routes/+layout.svelte, routes/insights.
   - Acceptance: screenshot vs v5 frame 1 rail; all old routes still reachable.
   - Depends on: none. Parallel: yes.
3. **Today front page + ledger-prose river**
   - Goal: greeting → digest paragraph (↻, lede stats) → suggestion chips + composer → river re-rendered as ledger prose.
   - Areas: insights Overview/Journal components, new Today assembly, chip-fill module (bun-tested).
   - Acceptance: screenshot vs v5 frame 1; chips fill from fixture data.
   - Depends on: 2. Parallel: with 4–6.
4. **Receipt drawer**
   - Goal: span-scoped evidence drawer from any receipt link; "Browse full timeline" door.
   - Areas: ReceiptViewer + receipt-* modules, drawer host in the shell.
   - Acceptance: screenshot vs story-first frame 3; opens scoped, dismisses clean.
   - Depends on: 2. Parallel: yes.
5. **Meetings surface UI**
   - Goal: list + detail (Summary/Transcript/Notes) per app-match/meetings.html.
   - Areas: new lib/meetings + route, conversation link into chat surfaces.
   - Acceptance: screenshots vs both meetings frames; all four states rendered from fixtures.
   - Depends on: 1, 2. Parallel: with 3/4.
6. **Trigger template gallery**
   - Goal: Template step 0 → prefill → land on Review with removable template chip.
   - Areas: TriggerWizard, static template list, wizard.css.
   - Acceptance: screenshot vs triggers frames 2–3; scratch path lands on step 01.
   - Depends on: 2 (shell placement only). Parallel: yes.
7. **"On the record" capture model**
   - Goal: pill + popover + timed off-the-record with persisted auto-resume; live-edge state in the river; tray language updated.
   - Areas: native_capture/lifecycle.rs, status_bar.rs, titlebar components, Today live edge.
   - Acceptance: Rust tests for deadline persistence/resume; screenshots vs v5 frames 1–2; manual drill passes.
   - Depends on: 3 (live edge). Backend half parallel: yes.
8. **Warm Paper retheme (track B)**
   - Goal: light + dark token sets app-wide; light becomes default; theme toggle intact.
   - Areas: +layout.svelte tokens, settings/onboarding/quick-recall stylesheets.
   - Acceptance: screenshot sweep of every surface in both themes; no hard-coded dark values left on touched surfaces.
   - Depends on: none to start; final pass after 2–7 land. Parallel: continuous.

Parallel groups: [1, 2, 8-start] → [3, 4, 5, 6, 7-backend] → [7-UI, 8-final].

## Out of Scope

- Vertical-spine main surface (north star; receipt drawer is its stepping stone).
- Live/streaming transcription; meeting-upcoming (calendar) condition; pre-meeting prep.
- Outward delivery (Slack/mobile), online template gallery, LLM-generated suggestion chips.
- Projects/folders for chat history.

## Further Notes

- SUPPORTS.md and docs/agents updates where behavior is macOS-specific (timed resume across sleep).
- The old dark terminal theme stays selectable until Warm Paper Dark reaches parity; removal is a later decision.
- Trigger-run document view already matches the "calm document" direction; restyle only.
- After slice 7, revisit ADR 0021/0052 docs' wording if "off the record" surfaces in logs/UI copy.
