# Windows low-disk safety rides Capture Suspension, not the inactivity path

## Status

Accepted.

## Context

[ADR 0040](0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md)
modeled low-disk safety as a transient-liveness `CaptureSuspension` kind
(`CaptureSuspensionKind::LowDisk`), implemented macOS-first. The Windows branch
(issue #130) now picks it up.

But Windows does not use `CaptureSuspension` at all today.
[ADR 0023](0023-windows-transient-capture-recovery-reuses-inactivity-pause.md)
deliberately routed every Windows transient-liveness condition — system suspend,
session lock, monitor/display change, DPMS display-asleep — through the
*inactivity* pause/resume mechanism with a `ScreenPauseReason::TransientLiveness {
trigger }` discriminator, and explicitly **rejected** a parallel Windows
suspension store mirroring macOS's `PrivacyCaptureSuspension` — "for no behavioral
gain." So the #130 pickup faces a real fork: un-gate the macOS `CaptureSuspension`
store onto Windows for `LowDisk`, or extend the existing inactivity path with a
low-disk reason.

The fork is load-bearing because `LowDisk` differs from every existing Windows
inactivity/transient-liveness trigger on four behavioral axes:

- **Scope** — it suspends *all* sources (screen + microphone + system audio),
  because every source writes to the same recordings volume. Only `SystemSuspend`
  matches that breadth, and it does so through a dedicated side-channel snapshot
  (`InactivityState::system_suspend_paused_sources`), not the per-family reason.
- **Resume trigger** — it resumes on a free-space re-probe, not on the
  activity/display-present signals the inactivity families use. Routing it through
  those would cross-wire (a loud microphone must not resume a low-disk-suspended
  microphone).
- **Escalation** — below the reserve floor it *stops the session* gracefully; no
  inactivity pause ever ends a session.
- **Surfacing** — it raises a user-facing warning notification; inactivity pauses
  are silent.

## Decision

**1. Windows un-gates `CaptureSuspension` for `LowDisk`; DPMS/lock/sleep stay on
the inactivity path.** Low disk is modeled identically across platforms (ADR 0040
verbatim), while the existing Windows transient-liveness triggers keep riding the
inactivity mechanism (ADR 0023). This does **not** contradict ADR 0023: that ADR
rejected a parallel store *"for no behavioral gain,"* and that precondition fails
for `LowDisk` on all four axes above. The same logic ADR 0023 applied — share the
mechanism when there is no behavioral gain, split it when there is — now points
the other way. Routing `LowDisk` through the inactivity path would instead
re-create the `system_suspend_paused_sources` side-channel a second time (a
low-disk snapshot + tick short-circuit + free-space probe) *and* model one domain
concept two different ways across platforms — more divergence for no less wiring.

**2. The two stores are independent holds on the screen, with this precedence.**
On Windows `inactivity` owns DPMS/lock/sleep and `capture_suspension` owns
`LowDisk`; they never share a flag (the macOS suspend path keeps low-disk state
entirely in `capture_suspension` and never touches `inactivity.screen_paused`).
When both claim the screen — e.g. the display sleeps (screen inactivity-paused as
`TransientLiveness { DisplayAsleep }`, audio still recording) and then the disk
fills and the audio rotation boundary trips `LowDisk`:

- **Critical-floor graceful stop is absolute.** Free space `< RESERVE_FLOOR` stops
  the session regardless of DPMS/lock state (ADR 0040's "app-storage safety wins
  over a display that may still be asleep" — now concretely reachable on Windows
  because audio keeps recording through DPMS).
- **Above the floor, the screen restarts only when *both* holds clear.** Whichever
  recovery driver observes both-clear performs the restart; each driver clears
  *its own* marker when its condition lifts and hands the restart off to the other,
  so there is no mutual-deferral deadlock.
- **Audio families are owned solely by `LowDisk`** (DPMS/lock never suspend them),
  so free-space recovery resumes the microphone even while the screen stays
  DPMS-paused.

The only net-new cross-check beyond the un-gated macOS code is **one guard**: the
Windows transient-liveness screen-*resume* must check `!is_low_disk_suspended()`
before restarting the screen session, so a display waking onto a still-full disk
does not reopen a segment. The reverse direction (low-disk recovery refraining
from restarting a DPMS-asleep screen, and resuming the microphone independently)
already falls out of the existing `!is_screen_paused()` / `!is_microphone_paused()`
guards in `resume_all_sources_after_low_disk`.

**3. The Windows all-source suspend/resume explicitly handles the independent
system-audio WASAPI session.** On macOS system audio rides the screen backend, so
`suspend_screen_system_audio_capture` stops only screen + microphone. On Windows
system audio is an independent source ([ADR 0022](0022-system-audio-is-an-independent-source-on-windows.md))
with its own `active_system_audio_session` WASAPI render-loopback client. The
Windows `LowDisk` suspend must therefore *also*
`stop_and_detach_windows_audio_session` that client (the leaf the `SystemSuspend`
path already uses), and recovery must recreate it via `start_windows_active_segment`
— otherwise the system-audio client is orphaned and keeps writing to the full
disk, defeating the all-source guarantee. This is a named Windows addition to those
two helpers, not a mechanical cfg-widen.

## Considered Options

- **Model `LowDisk` through the Windows inactivity path** (the ADR 0023 shape).
  Rejected: the inactivity families resume on activity/display-present, so low disk
  would need its own side-channel snapshot, tick short-circuit, and free-space
  probe — re-creating the `SystemSuspend` machinery a second time — *and* would
  model low disk differently on Windows than on macOS. More divergence, no less
  wiring.
- **Pure cfg-widen of `suspend_screen_system_audio_capture` onto Windows.**
  Rejected: it stops only screen + microphone (macOS folds system audio into the
  screen backend), so it would orphan the independent Windows system-audio WASAPI
  client mid-suspension.

## Consequences

- Windows now carries two suspension stores. The DPMS×LowDisk collision, the
  `!is_low_disk_suspended()` resume guard, and the both-holds-clear screen-restart
  hand-off must be unit-tested for both orderings (free space recovers first vs.
  display wakes first).
- Low-disk recovery on Windows is a **separate ~10 s re-probe branch on the
  existing Windows rotation tick** (reusing `low_disk_can_resume`), *not* the macOS
  recovery driver and *not* the 2 s transient-liveness display-present cadence — a
  disk recovers far slower than a display wakes.
- `inactivity.screen_paused` is no longer a reliable proxy for "the screen session
  is live": the screen can be stopped by `LowDisk` while `screen_paused` is false,
  and the DPMS resume may clear `screen_paused` while deferring the actual restart.
  Rotation/resume code that assumed that equivalence must consult both stores.
