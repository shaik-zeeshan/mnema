//! The segment machinery under the Recording Lifecycle: starting and stopping
//! the runtime, segment rotation, inactivity pause/resume, wake recovery,
//! privacy/display and low-disk suspension, and committing audio segments.

use crate::disk_space;
use crate::native_capture::{metadata, privacy};
use crate::native_capture_output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file, strip_live_system_audio_output_file,
};
use crate::recording_settings::compute_effective_screen_bitrate_bps;
use crate::{AppCore, CoreEvent};
use capture_microphone as microphone_capture;
use capture_runtime::{
    parse_audio_restart_started_at_unix_ms, CaptureClock, RuntimeController, RuntimeSignal,
    RuntimeState, SegmentPlanner, SegmentSchedule,
};
use capture_screen::StopScreenCaptureSessionArgs;
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CaptureSources, RecordingSettings, SourceSessionMeta,
    SourceSessions,
};
use capture_vad::MicrophoneVadRuntime;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc;

use super::app_notifications::{
    clear_app_notification_by_id, push_error_app_notification,
    push_privacy_recovery_restart_required_notification, push_warning_app_notification,
};
use super::lifecycle::{NativeCaptureState, TickOutcome};
use crate::native_capture::frame_artifacts::{
    flush_frame_artifacts, try_forward_frame_artifact, FrameArtifactForwardingResult,
    FrameArtifactMessage,
};
use crate::native_capture::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal,
    ensure_microphone_planner_for_runtime, ensure_system_audio_planner_for_runtime,
    has_any_capture_sources, mark_runtime_session_failed, now_monotonic_marker_ms, now_unix_ms,
    prefixed_capture_id, privacy_suspended_sources_for_runtime_state,
    refresh_runtime_planner_dates, reset_runtime_after_start_error, screen_planner_for_runtime,
    should_recover_from_segment_finalize_error, CaptureSuspension, CaptureSuspensionKind,
    CaptureSuspensionStatus, NativeCaptureRuntime, SegmentLoopControl,
};
#[cfg(target_os = "macos")]
use crate::native_capture::runtime::{
    next_segment_estimate_for_runtime, recordings_root_for_runtime,
};

// Keep frame artifact persistence off the capture callback thread while bounding
// in-memory buffering. Backpressure is applied on a dedicated worker thread so
// exported frame artifacts are not dropped and the synchronous callback stays
// non-blocking.
const FRAME_ARTIFACT_BUFFER_CAPACITY: usize = 64;
const SEGMENT_LOOP_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const PRIVACY_FILTER_POLL_INTERVAL: Duration = Duration::from_secs(1);
// While capture is suspended because the display is unavailable (display sleep,
// screen lock, lid close, monitor disconnect), throttle recovery attempts to
// this cadence so we don't churn ScreenCaptureKit restarts on every 1s poll.
#[cfg(target_os = "macos")]
const DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL: Duration = Duration::from_secs(2);
// While capture is suspended because the recordings volume is low on free space,
// throttle recovery attempts to this cadence. Disk recovers far more slowly than
// a display waking, so re-probing every ~10s (vs the 2s display cadence) is plenty
// responsive without spinning `statvfs` on every 1s poll.
#[cfg(target_os = "macos")]
const LOW_DISK_RECOVERY_INTERVAL: Duration = Duration::from_secs(10);
// Stable id for the low-disk suspension warning notification, pushed when a
// Low-Disk Suspension is entered and cleared on resume.
#[cfg(target_os = "macos")]
const LOW_DISK_NOTIFICATION_ID: &str = "capture_low_disk";
// Stable id for the disk-full graceful-stop ERROR notification, pushed when free
// space drops below the critical floor and the session stops to protect the
// app's own storage (ADR 0040). Distinct from the suspend warning so the two can
// coexist in the cleared-on-resume vs persistent-stop lifecycles.
#[cfg(target_os = "macos")]
const DISK_FULL_STOPPED_NOTIFICATION_ID: &str = "capture_disk_full_stopped";
// The exact user-facing graceful-stop message (ADR 0040 surfacing).
#[cfg(target_os = "macos")]
const DISK_FULL_STOPPED_MESSAGE: &str = "Recording stopped — disk full.";
// The graceful-stop notification title.
#[cfg(target_os = "macos")]
const DISK_FULL_STOPPED_TITLE: &str = "Recording stopped — disk full";
// Trial-lapse graceful-stop ERROR notification: the trial ended while a session
// was recording, so the segment-rotation boundary committed the current segment
// and stopped instead of opening the next one (mirrors the read-only start
// refusal copy in `lifecycle.rs`).
#[cfg(target_os = "macos")]
const TRIAL_ENDED_STOPPED_NOTIFICATION_ID: &str = "capture_trial_ended_stopped";
#[cfg(target_os = "macos")]
const TRIAL_ENDED_STOPPED_TITLE: &str = "Recording stopped — trial ended";
#[cfg(target_os = "macos")]
const TRIAL_ENDED_STOPPED_MESSAGE: &str = "Your trial has ended. Buy a license to resume recording — everything you already recorded stays browsable and searchable.";
// License-revoked graceful-stop ERROR notification: a stored key that landed on
// the signed revocation list (refund/leak) blocks capture just like a lapsed
// trial, but must read as "revoked" — never conflated with the trial copy.
#[cfg(target_os = "macos")]
const REVOKED_STOPPED_NOTIFICATION_ID: &str = "capture_license_revoked_stopped";
#[cfg(target_os = "macos")]
const REVOKED_STOPPED_TITLE: &str = "Recording stopped — license revoked";
#[cfg(target_os = "macos")]
const REVOKED_STOPPED_MESSAGE: &str = "This license has been revoked. Contact support if you think this is a mistake — everything you already recorded stays browsable and searchable.";

#[cfg(target_os = "macos")]
fn persist_capture_session_started(
    core: &Arc<AppCore>,
    capture_session_id: String,
    started_at_unix_ms: u64,
    sources: &CaptureSources,
    source_sessions: &SourceSessions,
    segment_duration_seconds: u64,
) {
    let infra = Arc::clone(&*core.state::<crate::AppInfraState>());
    let started_at = rfc3339_from_unix_ms(started_at_unix_ms);
    let session = ::app_infra::NewCaptureSession {
        capture_session_id,
        started_at,
        requested_screen: sources.screen,
        requested_microphone: sources.microphone,
        requested_system_audio: sources.system_audio,
        screen_source_session_id: source_sessions
            .screen
            .as_ref()
            .map(|s| s.session_id.clone()),
        microphone_source_session_id: source_sessions
            .microphone
            .as_ref()
            .map(|s| s.session_id.clone()),
        system_audio_source_session_id: source_sessions
            .system_audio
            .as_ref()
            .map(|s| s.session_id.clone()),
        segment_duration_seconds: segment_duration_seconds.min(i64::MAX as u64) as i64,
    };
    match run_native_capture_async("capture-session-persistence", async move {
        infra
            .capture_retention()
            .create_capture_session(&session)
            .await
    }) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::debug_log::log_error(format!(
            "failed to persist native capture session: {error}"
        )),
        Err(error) => {
            crate::debug_log::log_error(format!(
                "native capture session persistence failed: {error}"
            ));
        }
    }
}

#[cfg(target_os = "macos")]
fn run_native_capture_async<F, R>(context: &'static str, future: F) -> Result<R, String>
where
    F: std::future::Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let run_future = move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                format!("failed to initialize native capture {context} runtime: {error}")
            })?;

        Ok(runtime.block_on(future))
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        let worker = thread::Builder::new()
            .name(format!("mnema-native-capture-{context}"))
            .spawn(run_future)
            .map_err(|error| format!("failed to spawn native capture {context} worker: {error}"))?;

        worker
            .join()
            .map_err(|_| format!("native capture {context} worker panicked"))?
    } else {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_future))
            .map_err(|_| format!("native capture {context} worker panicked"))?
    }
}

#[cfg(target_os = "macos")]
fn microphone_tail_trim_activity_mode_for_vad(
    vad: &MicrophoneVadRuntime,
) -> microphone_capture::MicrophoneInactivityTailTrimActivityMode {
    if vad.uses_vad_adapter() {
        microphone_capture::MicrophoneInactivityTailTrimActivityMode::VadSpeech
    } else {
        microphone_capture::MicrophoneInactivityTailTrimActivityMode::PeakLevel
    }
}

#[cfg(target_os = "macos")]
fn microphone_tail_trim_activity_mode_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> microphone_capture::MicrophoneInactivityTailTrimActivityMode {
    microphone_tail_trim_activity_mode_for_vad(&runtime.microphone_vad)
}

#[cfg(target_os = "macos")]
pub(super) fn stop_active_sessions_after_failure(runtime: &mut NativeCaptureRuntime) {
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let _ = session.stop();
    }
    runtime.active_microphone_session = None;

    let _ = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
    });

    crate::native_capture::runtime::stop_system_audio_session(runtime);
}

#[cfg(target_os = "macos")]
pub(super) fn cleanup_failed_segment_dirs(
    segment_dir: &Path,
    microphone_audio_dir: Option<&Path>,
    system_audio_dir: Option<&Path>,
) {
    let _ = microphone_audio_dir;
    let _ = system_audio_dir;

    if let Err(error) = std::fs::remove_dir_all(segment_dir) {
        if error.kind() != std::io::ErrorKind::NotFound {
            crate::debug_log::log_warn(format!(
                "failed removing unusable capture output directory {}: {}",
                segment_dir.display(),
                error
            ));
        }
    }
}

#[cfg(target_os = "macos")]
fn cleanup_failed_audio_outputs(
    microphone_output_path: Option<&Path>,
    system_audio_output_path: Option<&Path>,
) {
    for path in [microphone_output_path, system_audio_output_path]
        .into_iter()
        .flatten()
    {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::debug_log::log_warn(format!(
                    "failed removing unusable capture output file {}: {}",
                    path.display(),
                    error
                ));
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn create_segment_output_dirs(
    segment_dir: &Path,
    microphone_audio_dir: Option<&Path>,
    system_audio_dir: Option<&Path>,
    sources: &CaptureSources,
) -> Result<(), CaptureErrorResponse> {
    std::fs::create_dir_all(segment_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture segment directory: {error}"),
    })?;

    if sources.microphone || sources.system_audio {
        for audio_dir in [microphone_audio_dir, system_audio_dir]
            .into_iter()
            .flatten()
            .filter(|audio_dir| *audio_dir != segment_dir)
        {
            if let Err(error) = std::fs::create_dir_all(audio_dir) {
                cleanup_failed_segment_dirs(segment_dir, microphone_audio_dir, system_audio_dir);
                return Err(CaptureErrorResponse {
                    code: "io_error".to_string(),
                    message: format!("Failed to create capture audio directory: {error}"),
                });
            }
        }
    }

    Ok(())
}

pub(super) fn empty_output_files() -> CaptureOutputFiles {
    CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    }
}

/// The segment's output map, created if the runtime has none.
///
/// `current_segment_output_files` is `None` whenever nothing was live at the last
/// screen transition — which an audio family can leave and re-enter on its own,
/// because neither the microphone nor the tap needs the screen (ADR 0052). Every
/// path that opens an audio file must track it through here: a family resuming
/// into a `None` map used to silently drop the path it had just opened, leaving a
/// real file on disk that no rotation ever commits.
#[cfg(target_os = "macos")]
pub(super) fn current_segment_output_files_mut(
    runtime: &mut NativeCaptureRuntime,
) -> &mut CaptureOutputFiles {
    runtime
        .current_segment_output_files
        .get_or_insert_with(empty_output_files)
}

mod audio_outputs;
mod audio_persist;
mod graceful_stop;
mod inactivity_audio;
mod inactivity_screen;
mod low_disk;
mod rotation;
mod segment_loop;
mod segment_start;
mod start_stop;
mod suspension;
mod wake;

pub(crate) use audio_outputs::*;
pub(crate) use audio_persist::*;
pub(crate) use graceful_stop::*;
pub(crate) use inactivity_audio::*;
pub(crate) use inactivity_screen::*;
pub(crate) use low_disk::*;
pub(crate) use rotation::*;
pub(crate) use segment_loop::*;
pub(crate) use segment_start::*;
pub(crate) use start_stop::*;
pub(crate) use suspension::*;
pub(crate) use wake::*;

#[cfg(all(test, target_os = "macos"))]
mod tests;
