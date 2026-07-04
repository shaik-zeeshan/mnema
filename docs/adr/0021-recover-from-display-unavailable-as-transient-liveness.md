# Recover from display-unavailable as transient liveness, not privacy failure

## Status

Accepted. Amended 2026-07-04: the delegate-stop door now enters the same suspension (see Amendment).

## Context

The 1s live privacy-filter refresh re-queries ScreenCaptureKit to re-apply app-exclusion filters while recording. When the only display goes away mid-operation — display sleep on idle, screen lock, lid close in clamshell, or a monitor disconnect — that re-apply fails because ScreenCaptureKit reports no displays (`-3815`), surfaced as `PrivacyFilterApplyErrorKind::DisplayUnavailable`.

The old path flattened every apply failure into one generic `privacy_filter_apply_failed` error and routed it through `suspend_screen_system_audio_for_privacy_failure`. That produced a misleading, repeated cascade every time a display slept:

1. `privacy filter update failed; suspending screen/system-audio capture`
2. a doomed finalize of the in-flight segment whose `.mov` was already dead (no `moov`) → `failed to finalize suspended … screen output missing`
3. `-3808` from trying to stop an already-stopped stream

Worse, for a screen-only session (no microphone to keep alive) the loop then called `mark_runtime_session_failed` and exited — so a routine screen lock **killed the recording**, and because the segment loop owns no broadcast on an internal failure, the dashboard and native status-bar tray kept showing a running session that no longer existed.

A missing display is not a privacy-policy violation; it is a transient liveness condition that resolves itself when the display returns — exactly the shape macOS sleep/wake recovery already handles. The existing privacy-suspension recovery path (`attempt_privacy_suspension_recovery`) already restarts a fresh screen segment when recovery is possible; it was just being given up on (capped at `MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS`) and was hammering ScreenCaptureKit once per 1s poll, re-spamming `-3815`.

## Decision

A display-unavailable apply failure is modeled as a transient liveness suspension that recovers automatically:

- **Distinct error code.** `apply_privacy_filter_update` maps `DisplayUnavailable` to `privacy_filter_display_unavailable` (`privacy::PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE`) instead of flattening it into `privacy_filter_apply_failed`.
- **Two suspension kinds, one mechanism.** `PrivacyCaptureSuspension` carries a `CaptureSuspensionKind` (`PrivacyFilter` vs `DisplayUnavailable`). Both share `privacy_capture_suspension` and the same recovery path, but retry differently: `PrivacyFilter` stays capped and then escalates to the manual-restart notification, while `DisplayUnavailable` **never escalates** — a display returning is expected, not a failure to give up on.
- **The session stays alive.** A display-unavailable suspension keeps the session running **even with no microphone**, so screen capture resumes on its own when a display returns. Only a genuine privacy-filter failure with no other live source still ends the session.
- **No doomed work, no spam.** The suspend path skips finalizing the already-dead in-flight segment. Recovery probes for a returning display with a cheap `capture_screen::screen_display_available()` (`CGGetActiveDisplayList`) check, gated by a `DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL` throttle, so it waits quietly instead of attempting a `-3815`-logging restart every poll.
- **Internal session-end is broadcast.** When the segment loop ends a session itself (any internal `mark_runtime_session_failed`, not a user/command stop), it now emits `native_capture_session_changed` + `status_bar::refresh` after the loop exits, so both surfaces resync to the real state.

This stays inside the app-only privacy model of [ADR 0006](0006-use-app-level-live-privacy-exclusion-only.md) / [ADR 0008](0008-sensitive-capture-protection-v1-stays-app-exclusion-only.md); it changes liveness handling, not the privacy filtering surface.

## Alternatives Rejected

- **Stop the recording cleanly and require a manual restart.** Simpler, and the tray fix alone would then suffice. Rejected because Mnema is passive always-on capture: a screen lock or idle display sleep ending the recording (and needing a manual restart every time) is the wrong default. Auto-recovery matches sleep/wake behavior.
- **Swallow `DisplayUnavailable` (return `request_satisfied: false`, no suspension).** Avoids the cascade but leaves a dead screen stream with no recovery driver — recovery is gated on `privacy_capture_suspension.is_some()` — so capture would silently stop until the next system wake or app restart. Worse than the loud failure it replaced.
- **A separate display-liveness state machine and recovery path.** The privacy suspension already means "screen/system-audio capture is suspended and will be retried," which is exactly the needed shape. A parallel mechanism would duplicate recovery, segment-rotation, and source-bookkeeping logic for no behavioral gain; a `kind` discriminator on the existing type is sufficient. The type keeps its `Privacy*` name (documented) to avoid a wide rename.
- **Uncapped retry without the display pre-check.** With a 1s poll, retrying a full ScreenCaptureKit restart every second while the display is asleep re-introduces the `-3815` spam (now once per second) and churns capture starts. The `CGGetActiveDisplayList` pre-check plus throttle keeps the wait quiet and bounds the rare display-present-but-start-fails case.
- **A live-session liveness watchdog.** Not needed: the 1s privacy poll already drives both detection (apply failure) and recovery (the suspension retry loop), so no additional timer thread is introduced.

## Consequences

A display sleeping, locking, closing, or disconnecting during a recording now suspends screen/system-audio quietly, preserves microphone continuation, keeps the session alive, and resumes screen capture automatically when a display returns — with one informative log line instead of a session-killing cascade. The status-bar tray and dashboard stay truthful when a session ends internally. Distinguishing the two suspension reasons requires a `kind` field and kind-aware retry policy on `PrivacyCaptureSuspension`, and adds a small CoreGraphics display-availability seam in `crates/capture-screen`. Genuine privacy-filter apply failures are unchanged: still capped, still escalate to the manual-restart notification.

## Amendment (2026-07-04): the delegate-stop door must suspend too

This ADR's suspension was only entered from the privacy-refresh apply failure. In production (macOS 26), ScreenCaptureKit kills the stream **immediately** when the display sleeps and reports it through the stream delegate (`capture_stream_system_stopped`, `-3815`) — and once the delegate flags the stream dead, `apply_privacy_filter_update` no-ops on the not-live session, so the privacy door could never fire. The delegate-stop reconcile in `tick_inactivity` merely cleared screen state without a suspension owner, leaving "screen requested, no session, no suspension"; the next segment rotation (60s for short segment settings) rotated into the missing session, hit `invalid_runtime_state`, and killed the entire session — the exact failure mode this ADR was written to remove, through a door it didn't cover. Observed 2026-07-03 18:18 IST: display slept mid-segment, recovery lost the ~20s race to the boundary, and the session silently ended for the rest of the day.

Changes:

- **Delegate-stop reconcile suspends.** An unexpected stream-stop error taken in `tick_inactivity` now enters the `DisplayUnavailable` suspension (same owner, recovery, and retry policy as the privacy door) instead of a bare state clear.
- **Rotation-boundary backstop.** `tick_rotation` suspends (rather than fatally rotating) if screen/system-audio is active but the screen session is missing with no suspension owner — covering any remaining path into that state, e.g. a wake racing the will-sleep teardown.
- **The tail segment is preserved, superseding "no doomed work".** A delegate-reported stop is terminal (`stream_terminated`), so the stop path skips the doomed second `stop_stream` call but still finalizes the writers — the samples appended before the stream died make an openable `.mov`. With the tail no longer truncated, the suspend path commits the in-flight segment for all suspension kinds instead of skipping the commit for `DisplayUnavailable`; the "spurious finalize error" this ADR avoided no longer occurs because the file is valid.
