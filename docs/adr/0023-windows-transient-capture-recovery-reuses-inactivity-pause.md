# Windows transient capture recovery reuses the inactivity pause mechanism

## Status

Accepted.

## Context

[ADR 0021](0021-recover-from-display-unavailable-as-transient-liveness.md) established that a display going away mid-recording (sleep, lock, lid close, monitor disconnect) is a transient liveness condition Mnema should ride out — suspend screen/system-audio, keep the session alive and microphone recording, and auto-resume when a display returns — because Mnema is always-on passive capture and a routine screen lock must not kill the recording. On macOS that recovery rides the `PrivacyCaptureSuspension` path, because the live privacy filter already re-applies on a 1s poll and naturally surfaces "display unavailable."

Windows has no live privacy filter and therefore no `privacy_capture_suspension` field, so there is no existing suspension/recovery vehicle. The Windows MVP did the opposite of ADR 0021: when the WGC screen session reports `GraphicsCaptureItem.Closed` or stops being live, `fail_if_windows_screen_capture_stopped` calls `mark_runtime_session_failed` and stops the loop. A monitor disconnect, a `Win+L`, or a laptop sleep ends the whole recording.

Separately, the Windows inactivity slice introduces a per-family pause/resume mechanism (`screen_paused` / `microphone_paused` / `system_audio_paused` plus the stop/start-segment actions). An involuntary "the display is gone" suspension is structurally the same operation as an inactivity pause: stop the affected source's segment, mark it paused, and restart it when a resume condition is met. The only real difference is the resume *trigger*.

## Decision

Windows models sleep/wake, session lock/unlock, and monitor/display-change as transient, auto-recovering liveness conditions (per ADR 0021's philosophy), and reuses the inactivity pause/resume mechanism as the recovery vehicle rather than cloning the macOS `PrivacyCaptureSuspension` state machine.

- **Pause-reason discriminator.** The screen-pause state carries why it is paused: `Inactivity` vs `TransientLiveness { trigger: SystemSuspend | SessionLock | DisplayUnavailable }`. The two share the same underlying stop/start-segment actions but never cross-trigger: an `Inactivity` pause resumes on user activity (the existing activity snapshot), while a `TransientLiveness` pause resumes on a cheap, throttled "is a display/session present again" probe — regardless of user activity — mirroring macOS's `DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL` throttle. Without the discriminator the two resume conditions cross-wire (a keystroke trying to resume a screen whose display is still asleep, or a display-present probe resuming a screen the user is still idle away from).
- **Triggers.** `WM_POWERBROADCAST` drives system suspend/resume, `WTSRegisterSessionNotification` drives session lock/unlock, and display-change notifications plus the existing `GraphicsCaptureItem.Closed`/not-live signal drive display-unavailable. The old "fail the session on screen death" path is replaced by entering a `TransientLiveness` suspension.
- **Per-trigger family scope** (consistent with the independent-source model of [ADR 0022](0022-system-audio-is-an-independent-source-on-windows.md)):
  - **System suspend** suspends all active families and re-initializes them on resume (the machine sleeps regardless).
  - **Session lock** suspends screen only — WGC blanks on the secure desktop — while microphone and system-audio keep recording through the lock.
  - **Monitor/display change** suspends screen only and resumes when a display returns.
- **Session stays alive.** As with ADR 0021, a transient-liveness suspension keeps the session alive even with no other live source, so screen capture resumes on its own; only genuine, non-transient failures still end the session.

## Alternatives Rejected

- **Keep failing the session on screen loss (the MVP behavior).** Simplest, but turns a routine lock/sleep/disconnect into a dead recording with a manual restart — the wrong default for passive always-on capture, and a regression from macOS.
- **A dedicated Windows capture-suspension state mirroring `PrivacyCaptureSuspension`.** Closer to the macOS shape, but duplicates pause/resume, segment-stop, and source-bookkeeping logic the inactivity mechanism already owns, for no behavioral gain. A pause-reason discriminator on the existing state is sufficient — the same conclusion ADR 0021 reached when it added a `kind` to the shared suspension type instead of a parallel path.
- **Pause audio too on lock/display outages (uniform suspension).** Simpler and symmetric, but discards microphone and system-audio that Windows can still capture while only the screen is unavailable. Independent sources (ADR 0022) make screen-only suspension both possible and preferable.

## Consequences

On Windows, system sleep, `Win+L`, and monitor disconnect now suspend the affected sources quietly and auto-resume instead of killing the recording, matching macOS behavior. The screen-pause state gains a reason discriminator, and the inactivity resume logic must branch on it so liveness recovery and inactivity recovery never cross-trigger. Recovery and inactivity now share one mechanism, so changes to pause/resume or segment-stop actions affect both and must be tested for both reasons. Audio continuing through a locked session is intentional and must be reflected in source-session bookkeeping and segment continuity.
