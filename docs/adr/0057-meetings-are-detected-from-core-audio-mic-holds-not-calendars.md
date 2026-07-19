# Meetings are detected from Core Audio mic-holds, not calendars, app focus, or audio analysis

Status: accepted

The Triggers feature (docs/triggers/CONTEXT.md) needs a live "meeting ended" signal. We detect a Meeting as: a process on the app-shipped Conferencing Allowlist — or a browser in which a known meeting URL was observed — holding the microphone for at least the minimum duration (default 5 min, per-trigger advanced option); it ends when the mic stays released through a grace period (2 min, global setting). The mic-in-use-per-process state comes from the same Core Audio process-object enumeration `crates/capture-system-audio` already uses for its tap exclude list.

## Considered options

- **Calendar integration** — rejected for v1: new permission, misses every ad-hoc call, and invites the assumption that unscheduled calls aren't meetings. May return later as *enrichment* (matching a detected meeting to an invite), never as the detector.
- **Frontmost-app watching** — rejected: people background Zoom mid-call, Meet's bundle id is just the browser, and conferencing apps keep running long after calls end.
- **Audio-content analysis (speaker turns / system-audio activity)** — rejected as the trigger: it only exists after the processing pipeline runs, so it is minutes late and heuristic. Fine as evidence *inside* a recap.

## Consequences

- Browser detection is evidence-sticky: one meeting-URL sighting during a mic hold marks the whole hold, because the URL probe only sees the active tab of the front window and meeting tabs get backgrounded.
- The Conferencing Allowlist (bundle ids + URL patterns) is app-curated, not user-editable in v1; a missed niche app is an allowlist update, not a setting.
- Sub-threshold mic holds (dictation, voice memos) never fire — short real calls below the floor are deliberately sacrificed to avoid false recaps.
