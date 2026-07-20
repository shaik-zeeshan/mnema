//! System audio capture on Core Audio process taps (ADR 0052).
//!
//! A tap generation is cheap and disposable: build one with
//! [`SystemAudioTapSession::start`], drop it to tear it down, build another to
//! recover from a wedge.

mod activity;
mod exclude;
mod permission;
#[cfg(target_os = "macos")]
mod process;
#[cfg(target_os = "macos")]
mod rebuild;
#[cfg(target_os = "macos")]
mod tap;
mod watchdog;
#[cfg(target_os = "macos")]
mod writer;

pub use activity::{
    last_system_audio_activity_unix_ms, peek_system_audio_activity_window_peak_level,
    record_system_audio_activity_for_tests, reset_system_audio_activity,
    system_audio_activity_idle_ms, system_audio_activity_level, system_audio_sound_observed,
    take_system_audio_activity_window_peak_level,
};
pub use permission::{
    system_audio_permission_state, SystemAudioEvidence, SILENT_SESSION_AFTER_MS,
};
pub use exclude::{compute_exclude_list, AudioProcess, ExcludeList};
#[cfg(target_os = "macos")]
pub use exclude::{read_audio_processes, SystemAudioExcludeWatcher};
#[cfg(target_os = "macos")]
pub use cidre::cat::{AudioBuf, AudioStreamBasicDesc, AudioTimeStamp};
#[cfg(target_os = "macos")]
pub use process::own_process_object_id;
#[cfg(target_os = "macos")]
pub use rebuild::{RebuildReason, SystemAudioCaptureSession, SystemAudioSegmentHooks};
#[cfg(target_os = "macos")]
pub use tap::{
    cleanup_stale_aggregate_devices, prompt_for_system_audio_permission, SystemAudioTapSession,
};
pub use watchdog::{ZeroWatchdog, ZERO_WATCHDOG_INITIAL_DELAY, ZERO_WATCHDOG_MAX_DELAY};
#[cfg(target_os = "macos")]
pub use writer::SystemAudioSegmentFinalization;

/// The macOS runtime gate for system audio: Core Audio process taps exist from
/// 14.2, but the product gate stays 15.0 (ADR 0052, lowering deferred).
#[cfg(target_os = "macos")]
pub fn supports_system_audio_capture() -> bool {
    cidre::api::version!(macos = 15.0)
}

#[cfg(not(target_os = "macos"))]
pub fn supports_system_audio_capture() -> bool {
    false
}

/// Greppable marker for every tap lifecycle and rebuild event in `rust.log`.
pub const LOG_PREFIX: &str = "system-audio-tap:";

const AGGREGATE_UID_PREFIX: &str = "mnema-system-audio-";

/// Aggregate UIDs must be unique per pid *and* per instance: concurrent
/// instances sharing a UID was a real cpal bug, and a rebuild mints a new
/// aggregate while the old one may not be reaped yet.
fn system_audio_aggregate_uid(pid: u32, instance: &str) -> String {
    format!("{AGGREGATE_UID_PREFIX}{pid}-{instance}")
}

/// Recovers the minting pid from one of our aggregate UIDs; `None` for any UID
/// we did not mint.
fn aggregate_uid_pid(uid: &str) -> Option<u32> {
    let (pid, instance) = uid.strip_prefix(AGGREGATE_UID_PREFIX)?.split_once('-')?;
    if instance.is_empty() {
        return None;
    }
    pid.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_uid_round_trips_the_minting_pid() {
        let uid = system_audio_aggregate_uid(4242, "6C6A2B0E-1111-2222-3333-444455556666");
        assert_eq!(
            uid,
            "mnema-system-audio-4242-6C6A2B0E-1111-2222-3333-444455556666"
        );
        assert_eq!(aggregate_uid_pid(&uid), Some(4242));
    }

    #[test]
    fn aggregate_uid_is_unique_per_instance() {
        assert_ne!(
            system_audio_aggregate_uid(1, "instance-a"),
            system_audio_aggregate_uid(1, "instance-b")
        );
    }

    #[test]
    fn foreign_uids_are_never_claimed_as_ours() {
        for uid in [
            "BuiltInSpeakerDevice",
            "AppleHDAEngineOutput:1B,0,1,1:0",
            "mnema-system-audio",
            "mnema-system-audio-",
            "mnema-system-audio-4242",
            "mnema-system-audio-4242-",
            "mnema-system-audio-notapid-instance",
            "mnema-system-audio--instance",
            "other-mnema-system-audio-4242-instance",
        ] {
            assert_eq!(aggregate_uid_pid(uid), None, "{uid}");
        }
    }
}
