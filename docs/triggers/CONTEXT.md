# Triggers

User-defined automations: the user picks a Condition from a fixed menu, attaches their own Prompt, and Mnema runs the AI and delivers the result. First flagship Trigger: Meeting Recap.

Decisions: [ADR 0057](../adr/0057-meetings-are-detected-from-core-audio-mic-holds-not-calendars.md) (meeting detection), [ADR 0058](../adr/0058-trigger-runs-are-conversations-with-a-document-view-and-a-sealed-toolbox.md) (runs are conversations; document view; sealed toolbox).

Surfaces: dedicated `/triggers` page (list, last-run status, create/edit, share/import); runs appear in the chat rail with an origin badge + filter; definitions in `triggers.json` (app config dir); firing ledger + runs in the encrypted DB; release grace in Settings.

UI direction (finalized 2026-07-20 from the flow mockups in `docs/triggers/mockups/`): the **guided wizard** flow. Final design = [`docs/triggers/mockups/final/DESIGN.md`](mockups/final/DESIGN.md) (spec) + [`triggers-ui.html`](mockups/final/triggers-ui.html) (interactive reference): `/triggers` list grouped by condition with per-section ghost add-rows and hover edit/share/delete; a per-trigger runs ledger; creation/edit as a 3-step wizard (Condition → Prompt → Review; Import prefills and lands on Prompt; Edit lands on Review with "Save Changes"); runs render as a chrome-free report-style document. Trigger runs are marked with condition glyphs (◉ meeting-ends, ▣ app-opened, ◷ schedule), not an emoji.

## Language

**Trigger**:
A user-authored rule of the shape Condition + Prompt + Delivery.
_Avoid_: automation, rule, workflow

**Condition**:
One of a closed, app-shipped menu of detectable situations that fires a Trigger; users pick conditions, they cannot define new condition types.
v1 menu: **Meeting Ends**, **App Opened**, **Schedule**.

**Meeting**:
A period during which a known conferencing app — or a browser in which a known meeting URL was observed — held the microphone for at least the minimum duration (~5 min). Detected via Core Audio per-process mic-in-use state (the "orange dot" signal), not via frontmost app or audio content.

**Meeting Ends**:
The Condition that fires when the mic-holding conferencing process releases the microphone and keeps it released through a grace period (~2 min), absorbing drop/rejoin and back-to-back gaps.

**Conferencing Allowlist**:
The app-shipped list of known meeting apps (bundle ids: Zoom, Teams, Slack, FaceTime, …) and known meeting URL patterns (meet.google.com, zoom.us/j/, teams.microsoft.com, …) that scope Meeting detection. Browser processes count only when a meeting URL was seen during the mic hold.

**App Opened**:
The Condition that fires when a chosen app becomes frontmost after ≥30 min of not being frontmost (a fresh session). Any frontmost moment resets the clock, so rapid window switching never fires it. Gap is a fixed constant in v1, not a knob. Detected via the existing NSWorkspace `did_activate_app` observer.

**Cooldown**:
A per-trigger guard: a Trigger never fires again within 10 min (default) of its last firing, regardless of Condition. Protects against flapping (mic churn, crash-looping apps) without per-condition special cases.

**Advanced Options**:
Per-trigger tunables under a collapsed "Advanced" disclosure, all defaulted: minimum meeting length (5 min), App Opened away-gap (30 min), Cooldown (10 min). The Meeting release grace (2 min) is NOT per-trigger — it belongs to the one global detector and lives in Settings.

**Trigger JSON**:
The shareable form of a Trigger: canonical JSON (name, condition + params, prompt, `version: 1`) copied to the clipboard via "Share". "Import" pastes it but only *prefills the creation form* — the user reviews (especially the prompt, which will run automatically with outward-reaching tools) and saves. Never creates directly; never carries provider/model config.

**Meeting Recap**:
The flagship Trigger: Meeting Ends + a summarization Prompt (speaker-labeled summary + feedback).

**Prompt**:
The user's own free-text instruction, written (or edited from a starter template) at trigger creation. Plain prose — no template variables. Mnema assembles everything else around it.

**Context Assembly**:
What Mnema wraps around the Prompt on every Trigger Run, invisibly and by default: the firing context (condition, time window, app), User-Context conclusions (via the existing `recall_context` path), speaker identity (which voice is the user), and previous runs of the same Trigger — so results are personalized and compound over time. No per-trigger knobs in v1; an impersonal run is a prompt-level request ("keep it generic").

**Trigger Run**:
One firing of a Trigger, persisted as a normal conversation in the existing conversation store with `origin = trigger` (+ trigger id and display name); rendered by the same Chat surfaces, filterable in the rail.
_Avoid_: report, digest, notification (the notification is only the doorbell)

**Document View**:
The render mode for `origin = trigger` conversations: no question bubble or chat chrome — the answer is a titled, full-width markdown page with the existing typed AnswerBlocks (`mnema-bars`, `mnema-timeline`, `mnema-dossier`) inline. The trigger preamble instructs the model to write a structured document, not chat. Follow-up turns render as normal chat beneath the document.

**Sealed Toolbox**:
Trigger runs get read-only, inward-facing tools only (capture search, timeline, `recall_context`, past runs of the same trigger) — no web fetch, no MCP connectors, no app-control, no per-trigger model pin, unconditionally in v1. Makes pasted-prompt exfiltration structurally impossible for unattended runs; outward delivery (e.g. Slack) is a future Delivery option, not an open outbound tool.

**Delivery**:
A macOS notification (new: Tauri notification plugin) announcing a finished Trigger Run; clicking it opens that conversation. In-app banners and the tray are not delivery surfaces for triggers.

**Skipped Run**:
A firing that produced no Trigger Run because there was nothing to work with (e.g. Meeting detected but Mnema wasn't recording). Never notifies; shown quietly as the trigger's last-run status in the management UI. Notifications are only ever good news.

**Run Again**:
The retry affordance on a failed run's ledger row: re-runs *that* firing (the persisted question, as a fresh sealed turn in the same conversation) — never a synthetic new firing. Bypasses Cooldown (a deliberate click isn't flapping), respects the Provider Gate, appends a new ledger row, and notifies on completion like any run.

**Readiness Wait**:
Between a Condition firing and the AI run: the trigger waits (bounded, ~15 min cap) for the processing pipeline to finish transcription/diarization over the firing window. Delivery is simply a little later — never a partial-data run before the cap.

**Provider Gate**:
Triggers cannot be created without a configured AI provider, and flip to a visibly disabled "needs an AI provider" state if the provider is removed. A run never starts unconfigured — unconfigured is a trigger state, not a run failure.

## Relationships

- A **Trigger** references exactly one **Condition** type (plus per-type parameters).
- **Meeting** detection is evidence-sticky: one meeting-URL sighting during a mic hold marks the whole hold as a Meeting, because the meeting tab may be backgrounded while the browser still holds the mic.
- **Meeting Ends** depends on capture-system-audio's existing Core Audio process-object enumeration (`crates/capture-system-audio/src/exclude.rs`) and the existing browser-URL probe.
- The meeting-evidence URL probe obeys the same privacy gates as capture metadata: browser-URL mode `Off` disables it, privacy-excluded browsers are never probed, stored evidence is sanitized per mode, and it never raises a permission dialog (ADR 0057, amendment 2026-07-21).

## Flagged ambiguities

- "Any kind of trigger" (initial pitch) — resolved: users get freedom in Prompt and Condition × Prompt combinations, not user-defined condition *types*. Every Condition is a detector Mnema ships.
- "Meeting" ≠ `ActivityCategory::Meetings` (User Context's LLM-assigned tag on derived Activities). The Trigger-context Meeting is a live, mechanically-detected mic-hold window.
