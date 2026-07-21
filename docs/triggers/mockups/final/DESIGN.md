# Triggers UI — final design

Finalized 2026-07-20 from the flow-mockup exploration (issue #174). The chosen direction is the
**guided wizard** flow. The living reference is [`triggers-ui.html`](triggers-ui.html) — a fully
interactive, self-contained mockup of every screen and state below; open it in a browser and click
through. The superseded per-surface variants and the other four flow directions remain in
[`../`](../) for the record.

Domain language: [`../../CONTEXT.md`](../../CONTEXT.md). Contract for runs-as-conversations:
[ADR 0058](../../../adr/0058-trigger-runs-are-conversations-with-a-document-view-and-a-sealed-toolbox.md).

## Visual conventions

- Tokens from `docs/user-context/mockups/tokens.css` (terminal green `--app-accent`, dark, flat,
  1px borders, radii 4–9 px, Berkeley Mono stack at 13 px).
- **Condition glyphs** identify condition types everywhere — section headers, runs header, run
  badges, report eyebrow. No emoji (the earlier ⚡ marker is rejected).
  - `◉` Meeting Ends `▣` App Opened `◷` Schedule
- Status colors: completed = accent, skipped = neutral/muted, failed = danger, running = accent
  with a pulsing dot, needs-provider = warn, no-runs-yet = subtle.
- Controls carry hover, pressed (`:active`), `:focus-visible`, and disabled states. Destructive
  actions use danger colors.

## Screen 1 — `/triggers` (list)

- Header: "Triggers" + one-line explainer subtitle + **Import** button.
- Triggers are **grouped by condition** into three sections, each with glyph + title
  ("When a meeting ends") and a muted explainer sentence.
- Row = enable switch · name · condition detail ("— Figma, after 30 min away") · last-run status.
  - Name click → the trigger's **Runs** screen. Completed status click → the run's document.
  - Hover reveals quiet row actions: **edit · share · delete** (share copies Trigger JSON with a
    "copied ✓" flash; delete confirms; delete is danger-styled).
  - The **six lifecycle states** are all shown: completed / skipped — reason / failed — reason
    (+ a "run again" retry link when the failed run's conversation exists — see Screen 2) /
    running — waiting for the transcript (Readiness Wait, pulsing dot) / no runs yet /
    needs an AI provider (Provider Gate: dimmed row, warn chip, "Set up provider" link,
    switch disabled). Every status has an explanatory tooltip.
- Each section ends with a dashed ghost row "＋ add a …-trigger" that opens the wizard with that
  condition preselected. A section emptied by deletes shows "Nothing here yet."
- Creating/saving flashes the affected row accent-green (success feedback).

## Screen 2 — `/triggers/runs` (per-trigger firing ledger)

- Breadcrumb `triggers / <name>`; header `<name>` + `<glyph> <condition> <params>`.
- Sub-line sets expectations: skips and failures never notify — they only show up here.
- One row per firing: status word + (completed → document title, clickable, "open ↗" hover hint;
  skipped/failed → "— reason") + relative time, newest first.
- Failed rows carry a **run again** action (added 2026-07-21 — the original mockup's omission was
  an oversight; issue #182 asked for it). It retries *that* firing: a fresh sealed turn re-running
  the persisted question in the same conversation. Cooldown does not gate the click (it guards
  flapping detectors, not deliberate retries); the Provider Gate does. The retry appends a new
  ledger row (never amends the failed one) and notifies on completion like any run. No button on
  the one failed variant with no conversation to retry into (conversation creation itself failed).
- Empty states: "No runs yet — it hasn't fired…" and a provider-gate variant.
- Footer note: runs are ordinary conversations, also in the chat rail under the Triggers filter.

## Screen 3 — `/triggers/new` · `/triggers/edit` (3-step wizard)

- Step header `01 Condition → 02 Prompt → 03 Review` with accent progress; visited steps are
  clickable, forward-jumping past unvisited steps is not.
- **Step 1 — Condition**: three descriptive radio cards; the selected card grows an attached
  params panel (Meeting Ends: read-only detection description; App Opened: app picker + fresh-
  session note; Schedule: time + Mon–Sun day chips + live "Runs weekdays at 6:00 PM." preview).
- **Step 2 — Prompt**: starter template per condition in a monospace editor; switching condition
  swaps the template only while the prompt is unedited (dirty tracking, "edited" chip, Reset);
  live char count; footer: "plain prose, no variables — Mnema adds context automatically".
- **Step 3 — Review**: name field (required; inline error), condition echo (params + tuned
  Advanced values), 3-line-clamped prompt preview with show-all, the **Context Assembly** list
  ("What Mnema adds automatically"), and the collapsed **Advanced** disclosure:
  min meeting length 1–30 (Meeting Ends only) · away gap 5–120 (App Opened only) ·
  cooldown 0–120, steppers, "all defaults / modified" note.
- **Import**: prefills the wizard and lands on **Step 2** (the prompt is what needs review) with
  the warn banner "Imported — review this prompt before saving…". Never saves directly.
- **Edit**: opened from row/report; lands on **Review** with all steps unlocked, breadcrumb
  `edit · <name>`, primary button "Save Changes"; preserves enabled state and run history.
- **Share as JSON** (Review only) copies the canonical shape — never provider/model config:
  `{ version: 1, name, condition: { type, params (+cooldown_minutes) }, prompt }`.

## Screen 4 — `/triggers/run` (document view)

Per ADR 0058: no question bubble, no chat chrome — a report.

- Context-aware back link ("◀ <trigger> · runs" when arrived from a runs list, else "◀ Triggers").
- Eyebrow chip `<glyph> TRIGGER RUN · <trigger name>` (name opens the trigger's edit wizard);
  document title; metadata line (condition · app · window · ran-at); thin accent rule.
- Markdown body with the three typed AnswerBlocks inline as flat panels:
  `mnema-bars` (speaking time), `mnema-timeline` (meeting flow), `mnema-dossier` (facts).
  Action items render as a toggleable checklist.
- "FOLLOW-UP" divider, then normal chat turns + composer — follow-ups continue the same
  conversation. Hint line restates the sealed toolbox (read-only tools over capture history).

## Chat rail treatment (Insights shell)

Trigger-run conversations carry a small accent-bordered chip `<glyph> <trigger name>` under the
title; plain chats have no chip. The rail's origin filter is All / Chats / Triggers. (Shown in
`../document-view-a.html`, which remains the rail-treatment reference; the report body there
predates the final report styling in `triggers-ui.html`.)

## Mockup-only shortcuts (not design decisions)

- Every completed run opens the same canned "Product Sync" document.
- "Set up provider" alerts instead of opening Settings.
- State is in-memory; nothing persists across reloads.
