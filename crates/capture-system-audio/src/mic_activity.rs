//! Per-process mic-in-use snapshots for meeting detection (ADR 0057).
//!
//! The Triggers meeting detector needs the "orange dot" signal per app: which
//! bundle ids currently hold the microphone. That is the same Core Audio
//! process-object enumeration the tap exclude list reads
//! (`kAudioHardwarePropertyProcessObjectList`, see [`crate::exclude`]), plus
//! one more per-process property — `kAudioProcessPropertyIsRunningInput` —
//! which cidre exposes as [`cidre::core_audio::Process::is_running_input`].
//!
//! This module lives here because this crate alone enables cidre's
//! `core_audio` feature (CLAUDE.md); the meeting state machine that consumes
//! these snapshots lives in the desktop crate's `triggers::meeting`.

use std::collections::BTreeSet;

use capture_types::CaptureErrorResponse;

#[cfg(target_os = "macos")]
fn mic_activity_error(context: &str, error: impl std::fmt::Debug) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "system_audio_mic_activity_failed".to_string(),
        message: format!("{context}: {error:?}"),
    }
}

/// The set of bundle ids whose processes are currently running audio INPUT
/// (holding the microphone), deduplicated across helper processes.
///
/// An empty *process list* is treated as a failed read, exactly as
/// [`crate::exclude`] does — Core Audio reports a zero-sized property read as
/// `Ok(vec![])`, and the list is never legitimately empty on a machine where
/// anything holds an audio client. An empty *holder set* over a non-empty list
/// is the normal "nobody is on the mic" answer.
#[cfg(target_os = "macos")]
pub fn snapshot_mic_holding_bundle_ids() -> Result<BTreeSet<String>, CaptureErrorResponse> {
    use cidre::core_audio as ca;

    let processes = ca::System::processes()
        .map_err(|error| mic_activity_error("read audio process list", error))?;
    if processes.is_empty() {
        return Err(mic_activity_error(
            "read audio process list",
            "empty process list",
        ));
    }

    let mut holders = BTreeSet::new();
    for process in processes {
        // ponytail: a process whose input state or bundle id cannot be read
        // counts as not holding — a single missed tick is absorbed by the
        // detector's release grace, same exposure exclude.rs accepts for a
        // failed bundle-id read.
        if !process.is_running_input().unwrap_or(false) {
            continue;
        }
        let Ok(bundle_id) = process.bundle_id() else {
            continue;
        };
        let bundle_id = bundle_id.to_string();
        if !bundle_id.is_empty() {
            holders.insert(bundle_id);
        }
    }
    Ok(holders)
}

/// Meeting detection is macOS-only (SUPPORTS.md): elsewhere every snapshot
/// fails, so the detector worker just idles.
#[cfg(not(target_os = "macos"))]
pub fn snapshot_mic_holding_bundle_ids() -> Result<BTreeSet<String>, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "system_audio_mic_activity_failed".to_string(),
        message: "mic-activity snapshots are macOS-only".to_string(),
    })
}
