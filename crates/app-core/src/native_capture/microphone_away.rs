//! The microphone's away hold (founder rulings 2026-10-23: lock is away;
//! 2026-10-29: fast user switching is away): the predicates over the runtime.
//! The hold itself and the reopen drive the segment machinery, so they stay
//! with it in the desktop crate until S3 part b.

#[cfg(target_os = "macos")]
use super::runtime::{CaptureSuspensionKind, NativeCaptureRuntime};
#[cfg(target_os = "macos")]
use crate::native_capture_inactivity::ActivitySnapshot;

/// Whether only the microphone family's own state keeps it from recording:
/// capture runs and wants it, and nothing outside the family holds it — not
/// the user, voice enrollment, the away hold, low disk, or a waited-for device
/// that is gone. Each of those brings the microphone back itself.
#[cfg(target_os = "macos")]
pub fn microphone_free_to_record(runtime: &NativeCaptureRuntime) -> bool {
    runtime.is_running
        && !runtime.user_capture_paused
        && runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
        && !runtime.microphone_lent_out
        && !runtime.microphone_away_hold
        && !runtime.microphone_waiting_for_device
        && !runtime
            .capture_suspension
            .as_ref()
            .is_some_and(|suspension| suspension.kind == CaptureSuspensionKind::LowDisk)
}

/// Lock is away (founder ruling 2026-10-23): the Mac is locked, another user
/// holds the console (fast user switching, ruling 2026-10-29), or every
/// display is asleep. Read by the tick and by every path that opens the
/// microphone on its own initiative.
#[cfg(target_os = "macos")]
pub fn mac_is_away(runtime: &NativeCaptureRuntime) -> bool {
    match runtime.away_probe {
        Some(probe) => probe(),
        None => {
            crate::native_capture_system_idle::session_is_away()
                || !capture_screen::screen_display_available()
        }
    }
}

/// Whether the microphone session's device runs; `None` with no session.
#[cfg(target_os = "macos")]
pub fn microphone_device_running(runtime: &NativeCaptureRuntime) -> Option<bool> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(device) = runtime.test_microphone_device.as_ref() {
        return Some(device.running);
    }
    runtime
        .active_microphone_session
        .as_ref()
        .map(|session| session.capture_running())
}

/// Starts or stops the session's device; whether it reached that state.
#[cfg(target_os = "macos")]
pub fn set_microphone_device_running(runtime: &mut NativeCaptureRuntime, running: bool) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(device) = runtime.test_microphone_device.as_mut() {
        device.running = running && !device.gone;
        return device.running == running;
    }
    runtime
        .active_microphone_session
        .as_mut()
        .is_some_and(|session| session.set_capture_running(running))
}

/// Whether the tick must bring the microphone back itself, because nothing
/// else will: with no session there is no sound for activity to hear, a
/// session whose device stopped hears nothing either, and with inactivity
/// monitoring off nothing ever resumes a pause.
///
/// Only the away hold stops a device on purpose, so a free microphone's
/// stopped device died — `startRunning` can succeed and the device vanish
/// moments later. An inactivity pause leaves the device running (listen-only).
#[cfg(target_os = "macos")]
pub fn should_reopen_microphone(runtime: &NativeCaptureRuntime) -> bool {
    microphone_free_to_record(runtime)
        && match microphone_device_running(runtime) {
            None | Some(false) => true,
            Some(true) => runtime.inactivity.is_microphone_paused() && !runtime.inactivity.enabled,
        }
}

/// Inactivity resumes the microphone on activity only while nothing outside the
/// family holds it ([`microphone_free_to_record`]); enrollment's own speech, for
/// one, would otherwise "resume" a no-op every tick.
#[cfg(target_os = "macos")]
pub fn should_resume_microphone_after_activity(
    runtime: &mut NativeCaptureRuntime,
    now: u64,
    activity_snapshot: ActivitySnapshot,
) -> bool {
    microphone_free_to_record(runtime)
        && runtime
            .inactivity
            .should_resume_microphone_from_inactivity(now, activity_snapshot)
}
