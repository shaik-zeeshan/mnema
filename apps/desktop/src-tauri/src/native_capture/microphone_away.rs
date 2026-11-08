//! The microphone's away hold and its reopen (founder rulings 2026-10-23: lock
//! is away; 2026-10-29: fast user switching is away). The tick drives both;
//! the Recording Lifecycle owns when they run.

use app_core::native_capture::microphone_away::{
    mac_is_away, microphone_device_running, set_microphone_device_running,
    should_reopen_microphone,
};
use app_core::native_capture::runtime::NativeCaptureRuntime;
use super::segments::{
    mark_microphone_family_paused, pause_microphone_for_inactivity_with_app_handle,
    resume_microphone_from_inactivity,
};
use capture_types::CaptureErrorResponse;
/// Stops and drops a session whose device is gone, booking its file for the
/// segment like any stop, so the reopen path starts a fresh session on
/// whatever device is current. A session kept on a dead device never records
/// again.
#[cfg(target_os = "macos")]
fn drop_dead_microphone_session(runtime: &mut NativeCaptureRuntime) {
    if let Err(error) = super::segments::stop_microphone_session_for_runtime(runtime) {
        app_core::debug_log::log_warn(format!(
            "failed to close the file of a microphone whose device stopped; dropping it anyway: [{}] {}",
            error.code, error.message
        ));
    }
    runtime.active_microphone_session = None;
    runtime.microphone_recording_file = None;
    #[cfg(test)]
    {
        runtime.test_microphone_device = None;
    }
}

/// Applies the away hold for a tick that found the Mac `away` or not.
///
/// Away: stop the device and pause the output for inactivity, so the file
/// closes and nothing is heard while locked; the hold keeps anything from
/// reopening it meanwhile. Back: restart the device and leave the output to
/// the activity policy, whose ring carries the lead-in — resuming here would
/// open a file that the next idle tick closes again. A device that does not
/// come back (AirPods that disconnected while locked) is dropped instead, and
/// the tick reopens the microphone on whatever device is current.
#[cfg(target_os = "macos")]
pub(super) fn apply_microphone_away(
    runtime: &mut NativeCaptureRuntime,
    away: bool,
    app_handle: Option<&tauri::AppHandle>,
) {
    // While away a running device belongs to a session opened before the hold
    // (every path that opens one checks for away first and stays down under the
    // hold instead), or in the moment before this tick saw the Mac go; either
    // way it is held now.
    if away && microphone_device_running(runtime) == Some(true) {
        // Device first, so nothing reaches the paused ring while the file
        // finalizes; the pause below works on a stopped device.
        set_microphone_device_running(runtime, false);
        if let Err(error) = pause_microphone_for_inactivity_with_app_handle(runtime, app_handle) {
            app_core::debug_log::log_warn(format!(
                "failed to close the microphone file while the Mac is away; device stopped anyway: [{}] {}",
                error.code, error.message
            ));
            // The pause swapped the output to listen-only before it failed, so
            // the session writes nothing now. Book the family as paused, or it
            // would read as recording and never be resumed into a fresh file.
            mark_microphone_family_paused(runtime);
        }
        app_core::debug_log::log_info(
            "stopped microphone while the Mac is locked or its displays are asleep",
        );
    }
    if away {
        runtime.microphone_away_hold = true;
        return;
    }
    if !runtime.microphone_away_hold {
        return;
    }
    if microphone_device_running(runtime).is_some() {
        if set_microphone_device_running(runtime, true) {
            app_core::debug_log::log_info("restarted microphone after the Mac came back");
        } else {
            app_core::debug_log::log_warn(
                "microphone did not restart after the Mac came back; reopening it on the current device",
            );
            drop_dead_microphone_session(runtime);
        }
    }
    // With no session (there was none, or low disk, a vanished device or
    // enrollment took it while away), the tick reopens it once nothing else
    // holds it.
    runtime.microphone_away_hold = false;
}

/// Stop the microphone device while the Mac is locked or every display is
/// asleep, and restart it once neither holds (founder ruling 2026-10-23: lock
/// is away, so speech said while locked is not recorded).
///
/// The probe runs 2-3×/s, so a session without the microphone skips it. Its
/// sources are fixed for the session, and every path that sets the hold wants
/// the microphone, so there is no hold to clear here.
#[cfg(target_os = "macos")]
pub(super) fn tick_microphone_away(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) {
    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.microphone)
    {
        return;
    }
    let away = mac_is_away(runtime);
    apply_microphone_away(runtime, away, app_handle);
}

/// Brings back a microphone nothing else will ([`should_reopen_microphone`]),
/// retrying every tick until it records: a paused one is resumed, an unpaused
/// one restarted. Without this a microphone left with no session — the Mac came
/// back while its device was gone or the disk was full, a resume or an
/// enrollment restore failed — stays down for the rest of the session.
#[cfg(target_os = "macos")]
pub(super) fn reopen_unwakeable_microphone(
    runtime: &mut NativeCaptureRuntime,
    failure_logged: &mut bool,
    current_device_id: impl FnOnce() -> Option<Option<String>>,
) {
    if !should_reopen_microphone(runtime) {
        return;
    }
    if microphone_device_running(runtime) == Some(false) {
        app_core::debug_log::log_warn("microphone device stopped unexpectedly; reopening it");
        drop_dead_microphone_session(runtime);
    }
    // The device the session started on may be the one that went; reopen on
    // whatever the preferences resolve to now.
    if let Some(device_id) = current_device_id() {
        runtime.microphone_device_id_for_capture = device_id;
    }
    let reopened = if runtime.inactivity.is_microphone_paused() {
        resume_microphone_from_inactivity(runtime)
    } else {
        let device_id = runtime.microphone_device_id_for_capture.clone();
        app_core::native_capture::microphone::restart_microphone_session_for_runtime(
            runtime,
            device_id.as_deref(),
            "reopening microphone",
        )
    };
    match settle_microphone_reopen(runtime, reopened) {
        Ok(()) => {
            *failure_logged = false;
            app_core::debug_log::log_info("reopened microphone capture");
        }
        Err(error) => app_core::debug_log::log_at(
            app_core::debug_log::streak_level(failure_logged),
            &format!(
                "failed to reopen microphone capture; retrying: [{}] {}",
                error.code, error.message
            ),
        ),
    }
}

/// A start can return with a device that resolves but will not run. That is a
/// failed reopen: the session is dropped and the error joins the tick's failure
/// streak, rather than reading as dead next tick and cycling a Warn
/// "stopped unexpectedly" and an Info "reopened" every second.
#[cfg(target_os = "macos")]
pub(super) fn settle_microphone_reopen(
    runtime: &mut NativeCaptureRuntime,
    reopened: Result<(), CaptureErrorResponse>,
) -> Result<(), CaptureErrorResponse> {
    reopened?;
    if microphone_device_running(runtime) == Some(false) {
        drop_dead_microphone_session(runtime);
        return Err(CaptureErrorResponse {
            code: "microphone_device_not_running".to_string(),
            message: "The microphone session started but its device is not running".to_string(),
        });
    }
    Ok(())
}
