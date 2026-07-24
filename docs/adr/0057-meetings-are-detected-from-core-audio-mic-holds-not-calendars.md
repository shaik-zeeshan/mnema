# Meetings are detected from Core Audio mic-holds, not calendars, app focus, or audio analysis

Status: accepted — amended 2026-07-21 (browser-URL probe obeys capture privacy gates)

The Triggers feature (docs/triggers/CONTEXT.md) needs a live "meeting ended" signal. We detect a Meeting as: a process on the app-shipped Conferencing Allowlist — or a browser in which a known meeting URL was observed — holding the microphone for at least the minimum duration (default 5 min, per-trigger advanced option); it ends when the mic stays released through a grace period (2 min, global setting). The mic-in-use-per-process state comes from the same Core Audio process-object enumeration `crates/capture-system-audio` already uses for its tap exclude list.

## Considered options

- **Calendar integration** — rejected for v1: new permission, misses every ad-hoc call, and invites the assumption that unscheduled calls aren't meetings. May return later as *enrichment* (matching a detected meeting to an invite), never as the detector.
- **Frontmost-app watching** — rejected: people background Zoom mid-call, Meet's bundle id is just the browser, and conferencing apps keep running long after calls end.
- **Audio-content analysis (speaker turns / system-audio activity)** — rejected as the trigger: it only exists after the processing pipeline runs, so it is minutes late and heuristic. Fine as evidence *inside* a recap.

## Consequences

- Browser detection is evidence-sticky: one meeting-URL sighting during a mic hold marks the whole hold, because the URL probe only sees the active tab of the front window and meeting tabs get backgrounded.
- The Conferencing Allowlist (bundle ids + URL patterns) is app-curated, not user-editable in v1; a missed niche app is an allowlist update, not a setting.
- Sub-threshold mic holds (dictation, voice memos) never fire — short real calls below the floor are deliberately sacrificed to avoid false recaps.

## Amendment 2026-07-21 — the browser-URL probe obeys the capture privacy gates

The original text was silent on how the meeting-evidence URL probe interacts
with the existing browser-URL privacy controls; the first implementation
bypassed all of them. Decision: creating a Meeting Ends trigger is consent to
*recaps*, not to overriding privacy settings the user explicitly set. The
trigger probe follows the identical rules as the capture-metadata prober:

- **Browser-URL mode `Off` disables the probe.** Browser meetings go
  undetected for those users; app meetings (Zoom, Teams, FaceTime apps) still
  detect. The creation wizard discloses this when it applies.
- **Privacy-excluded browsers are never probed.** Exclusion means "Mnema
  ignores this app" — parity with screen capture and the system-audio tap,
  not a feature.
- **Stored evidence is sanitized per the user's mode.** The probe matches the
  allowlist on the raw URL (host + path only), but the URL persisted into the
  firing context passes through `sanitize_url` — in the default Sanitized
  mode this strips query strings, so e.g. Zoom's `?pwd=` meeting password
  never lands in the run's conversation row.
- **The probe never raises a permission dialog.** The Gecko Accessibility
  prompt was already suppressed; the AppleScript Automation prompt is too
  (pre-checked without asking). Permission is earned on the capture prober's
  foreground path (browser frontmost while recording); until then the probe
  quietly yields no evidence.
