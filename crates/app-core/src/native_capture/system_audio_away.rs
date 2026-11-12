//! System audio's away hold (founder ruling 2026-11-12): while the Mac is away
//! ([`super::microphone_away::mac_is_away`]) the tap is torn down, because a
//! running tap's IOProc holds coreaudiod's `PreventUserIdleSystemSleep`
//! assertion and the Mac could never idle-sleep. Audio played while away is not
//! recorded. The tick rebuilds the tap once the Mac is back.

#[cfg(target_os = "macos")]
use super::runtime::{stop_system_audio_session, NativeCaptureRuntime};
#[cfg(target_os = "macos")]
use super::segments::{clear_live_system_audio_outputs, commit_closed_system_audio_outputs};
#[cfg(target_os = "macos")]
use crate::AppCore;
#[cfg(target_os = "macos")]
use std::sync::Arc;

/// Applies the hold for a tick that found the Mac `away` or not, and says
/// whether the Mac just came back to a held tap, which the tick then rebuilds.
///
/// Away: stop the tap outright — the tap and its aggregate device must be gone
/// to release the assertion — and commit the file it closed against the
/// current segment, as an inactivity pause does. Committing now matters: a
/// pause while away clears the live bookkeeping, and with it any file still
/// waiting for the segment boundary.
#[cfg(target_os = "macos")]
pub(super) fn apply_system_audio_away(
    runtime: &mut NativeCaptureRuntime,
    away: bool,
    core: Option<&Arc<AppCore>>,
) -> bool {
    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
    {
        return false;
    }
    if !away {
        return std::mem::take(&mut runtime.system_audio_away_hold);
    }

    let had_tap = runtime.active_system_audio_session.is_some();
    if had_tap || runtime.system_audio_recording_file.is_some() {
        // Stop first: it finalizes the writer, so the commit reads a whole file.
        stop_system_audio_session(runtime);
        if let Err(error) = commit_closed_system_audio_outputs(runtime, core) {
            crate::debug_log::log_warn(format!(
                "{} failed to commit the system audio file while the Mac is away; tap stopped anyway: [{}] {}",
                capture_system_audio::LOG_PREFIX,
                error.code,
                error.message
            ));
        }
        clear_live_system_audio_outputs(runtime);
    }
    if had_tap {
        crate::debug_log::log_info(format!(
            "{} stopped system audio while the Mac is locked or its displays are asleep",
            capture_system_audio::LOG_PREFIX
        ));
    }
    runtime.system_audio_away_hold = true;
    false
}

/// How long the current tap has run, the window its silence is judged over
/// for the permission evidence. Not the recording's age: time with no tap (the
/// Mac away, a user pause) gave it nothing to hear.
#[cfg(target_os = "macos")]
pub(super) fn system_audio_tap_age_ms(runtime: &NativeCaptureRuntime, now_unix_ms: u64) -> u64 {
    runtime
        .system_audio_tap_started_at_unix_ms
        .map_or(0, |started| now_unix_ms.saturating_sub(started))
}
