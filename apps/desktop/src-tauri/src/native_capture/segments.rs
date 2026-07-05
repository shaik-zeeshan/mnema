use super::output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
use super::settings::compute_effective_screen_bitrate_bps;
use super::{disk_space, metadata, privacy};
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
use tauri::Manager;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc;

use super::emit_audio_segments_changed;
use super::lifecycle::TickOutcome;
use super::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal,
    ensure_microphone_planner_for_runtime, ensure_system_audio_planner_for_runtime,
    has_any_capture_sources, mark_runtime_session_failed, now_monotonic_marker_ms, now_unix_ms,
    prefixed_capture_id, privacy_suspended_sources_for_runtime_state,
    refresh_runtime_planner_dates, reset_runtime_after_start_error, screen_planner_for_runtime,
    should_recover_from_segment_finalize_error, CaptureSuspensionKind, NativeCaptureRuntime,
    CaptureSuspension, CaptureSuspensionStatus, SegmentLoopControl,
};
use super::NativeCaptureState;

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

#[cfg(target_os = "macos")]
fn persist_capture_session_started(
    app_handle: &tauri::AppHandle,
    capture_session_id: String,
    started_at_unix_ms: u64,
    sources: &CaptureSources,
    source_sessions: &SourceSessions,
    segment_duration_seconds: u64,
) {
    let infra = Arc::clone(&*app_handle.state::<crate::app_infra::AppInfraState>());
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
        Ok(Err(error)) => {
            super::debug_log::log(format!("failed to persist native capture session: {error}"))
        }
        Err(error) => {
            super::debug_log::log(format!(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameArtifactForwardingResult {
    Enqueued,
    ReceiverClosed,
}

pub(crate) enum FrameArtifactMessage {
    Artifact(FrameArtifactEnvelope),
    Flush(std::sync::mpsc::SyncSender<()>),
}

pub(crate) struct FrameArtifactEnvelope {
    pub artifact: capture_screen::ScreenFrameArtifact,
    pub metadata_snapshot: Option<capture_metadata::FrameMetadataSnapshot>,
}

#[cfg(test)]
impl FrameArtifactMessage {
    pub(super) fn unwrap_artifact(self) -> capture_screen::ScreenFrameArtifact {
        match self {
            Self::Artifact(envelope) => envelope.artifact,
            Self::Flush(_) => panic!("expected Artifact, got Flush"),
        }
    }

    pub(super) fn unwrap_artifact_envelope(self) -> FrameArtifactEnvelope {
        match self {
            Self::Artifact(envelope) => envelope,
            Self::Flush(_) => panic!("expected Artifact, got Flush"),
        }
    }
}

pub(super) fn try_forward_frame_artifact(
    frame_artifact_tx: &mpsc::Sender<FrameArtifactMessage>,
    artifact: capture_screen::ScreenFrameArtifact,
    metadata_snapshot: Option<capture_metadata::FrameMetadataSnapshot>,
) -> FrameArtifactForwardingResult {
    match frame_artifact_tx.blocking_send(FrameArtifactMessage::Artifact(FrameArtifactEnvelope {
        artifact,
        metadata_snapshot,
    })) {
        Ok(()) => FrameArtifactForwardingResult::Enqueued,
        Err(_) => FrameArtifactForwardingResult::ReceiverClosed,
    }
}

/// Blocks until all frame artifacts currently in the queue have been persisted.
///
/// Sends a flush marker through the same FIFO channel and waits for the worker
/// to acknowledge it. Because the channel is ordered, every artifact enqueued
/// before the marker is guaranteed to be fully persisted when this returns.
///
/// Uses `try_send` with a brief retry so the call is safe from any thread
/// context (including tokio-associated threads where `blocking_send` would
/// panic).
pub(super) fn flush_frame_artifacts(frame_artifact_tx: &mpsc::Sender<FrameArtifactMessage>) {
    let (response_tx, response_rx) = std::sync::mpsc::sync_channel(1);
    let mut message = FrameArtifactMessage::Flush(response_tx);
    loop {
        match frame_artifact_tx.try_send(message) {
            Ok(()) => break,
            Err(mpsc::error::TrySendError::Full(returned)) => {
                message = returned;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
        }
    }
    // 30 s is deliberately generous; the worker should ack almost immediately
    // after draining the buffered artifacts ahead of the flush marker.
    let _ = response_rx.recv_timeout(Duration::from_secs(30));
}

#[cfg(target_os = "macos")]
pub(super) fn stop_active_sessions_after_failure(runtime: &mut NativeCaptureRuntime) {
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let _ = session.stop();
    }
    runtime.active_microphone_session = None;

    let _ = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
        inactivity_tail_trim_seconds: 0,
    });
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
            super::debug_log::log(format!(
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
                super::debug_log::log(format!(
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

#[cfg(target_os = "macos")]
pub(super) fn apply_microphone_output_finalization(
    output_files: Option<&mut CaptureOutputFiles>,
    finalization: &microphone_capture::MicrophoneOutputFinalization,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
) {
    let Some(output_files) = output_files else {
        return;
    };

    let Some(original_output_file) = finalization.output_file.as_deref() else {
        let current_file = output_files.microphone_file.clone();
        output_files.microphone_files.retain(|file| {
            Some(file.as_str()) != current_file.as_deref()
                && Some(file.as_str()) != finalization.source_file.as_deref()
        });
        output_files.microphone_file = output_files.microphone_files.last().cloned();
        return;
    };
    let mut output_file = original_output_file.to_string();

    if finalization.trim_start_offset_ms > 0 {
        if let (Some(source_session), Some(schedule)) = (
            source_sessions.and_then(|sessions| sessions.microphone.as_ref()),
            schedule,
        ) {
            let base_started_at = audio_segment_started_at_unix_ms_for_file(
                source_session,
                segment_index,
                schedule,
                &output_file,
            );
            let shifted_started_at =
                base_started_at.saturating_add(finalization.trim_start_offset_ms);
            if let Some(shifted) =
                timestamped_microphone_output_file(&output_file, shifted_started_at)
            {
                if shifted != output_file {
                    if let Err(error) = std::fs::rename(&output_file, &shifted) {
                        super::debug_log::log(format!(
                            "failed to rename trimmed microphone output {} to {}: {}",
                            output_file, shifted, error
                        ));
                    } else {
                        output_file = shifted;
                    }
                }
            }
        }
    }

    output_files.microphone_file = Some(output_file.clone());
    if let Some(existing) = output_files.microphone_files.iter_mut().rfind(|file| {
        file.as_str() == original_output_file
            || Some(file.as_str()) == finalization.source_file.as_deref()
    }) {
        *existing = output_file;
    } else if !output_files
        .microphone_files
        .iter()
        .any(|file| file == &output_file)
    {
        output_files.microphone_files.push(output_file);
    }
}

#[cfg(target_os = "macos")]
fn timestamped_microphone_output_file(file_path: &str, started_at_unix_ms: u64) -> Option<String> {
    let path = Path::new(file_path);
    let parent = path.parent();
    let stem = path.file_stem()?.to_str()?;
    let extension = path.extension().and_then(|extension| extension.to_str());
    let base_stem = microphone_output_timestamp_base_stem(stem);
    let file_name = match extension {
        Some(extension) => format!("{base_stem}-{started_at_unix_ms}.{extension}"),
        None => format!("{base_stem}-{started_at_unix_ms}"),
    };
    Some(
        parent
            .map(|parent| parent.join(&file_name))
            .unwrap_or_else(|| PathBuf::from(file_name))
            .to_string_lossy()
            .to_string(),
    )
}

#[cfg(target_os = "macos")]
fn microphone_output_timestamp_base_stem(stem: &str) -> &str {
    let marker = "-segment-";
    let Some(marker_start) = stem.rfind(marker) else {
        return stem;
    };
    let after_marker_start = marker_start + marker.len();
    let after_marker = &stem[after_marker_start..];
    if after_marker.len() < 4 {
        return stem;
    }

    let (segment_index, remainder) = after_marker.split_at(4);
    if !segment_index.bytes().all(|byte| byte.is_ascii_digit()) {
        return stem;
    }
    let Some(timestamp_with_suffix) = remainder.strip_prefix('-') else {
        return stem;
    };
    let (timestamp, suffix) = timestamp_with_suffix
        .split_once('-')
        .map_or((timestamp_with_suffix, None), |(timestamp, suffix)| {
            (timestamp, Some(suffix))
        });

    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return stem;
    }
    if suffix.is_some_and(|suffix| {
        suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return stem;
    }

    &stem[..after_marker_start + segment_index.len()]
}

#[cfg(target_os = "macos")]
fn audio_only_output_files(
    output_files: Option<&CaptureOutputFiles>,
    include_microphone: bool,
    include_system_audio: bool,
) -> Option<CaptureOutputFiles> {
    let output_files = output_files?;
    let mut audio_only = empty_output_files();

    if include_microphone {
        audio_only.microphone_file = output_files.microphone_file.clone();
        audio_only.microphone_files = output_files.microphone_files.clone();
    }

    if include_system_audio {
        audio_only.system_audio_file = output_files.system_audio_file.clone();
        audio_only.system_audio_files = output_files.system_audio_files.clone();
    }

    let has_audio = audio_only.microphone_file.is_some()
        || !audio_only.microphone_files.is_empty()
        || audio_only.system_audio_file.is_some()
        || !audio_only.system_audio_files.is_empty();

    has_audio.then_some(audio_only)
}

#[cfg(target_os = "macos")]
fn append_and_persist_committed_audio_outputs(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
    output_files: Option<&CaptureOutputFiles>,
) {
    let Some(output_files) = output_files else {
        return;
    };

    if let Some(committed) = runtime.output_files.as_mut() {
        append_committed_segment_output_files(committed, output_files);
    }

    persist_committed_audio_segments(
        app_handle,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
        Some(output_files),
    );
}

#[cfg(target_os = "macos")]
fn ensure_audio_dir_exists(audio_dir: &Path) -> Result<(), CaptureErrorResponse> {
    std::fs::create_dir_all(audio_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture audio directory: {error}"),
    })
}

#[cfg(target_os = "macos")]
fn next_reanchored_microphone_output_file(
    runtime: &mut NativeCaptureRuntime,
    next_index: u64,
    context: &str,
) -> Result<Option<String>, CaptureErrorResponse> {
    if runtime.inactivity.is_microphone_paused()
        || runtime.active_microphone_session.is_none()
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
    {
        return Ok(None);
    }

    let planner = ensure_microphone_planner_for_runtime(runtime, context)?.ok_or_else(|| {
        CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture microphone planner missing while {context}"),
        }
    })?;
    ensure_audio_dir_exists(&planner.audio_dir())?;

    Ok(Some(
        planner
            .microphone_reconnect_file(next_index, now_unix_ms())
            .to_string_lossy()
            .to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn next_reanchored_system_audio_output_file(
    runtime: &mut NativeCaptureRuntime,
    next_index: u64,
    context: &str,
) -> Result<Option<String>, CaptureErrorResponse> {
    if runtime.inactivity.is_system_audio_paused()
        || runtime.system_audio_recording_file.is_none()
        || !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref())
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.system_audio)
    {
        return Ok(None);
    }

    let planner = ensure_system_audio_planner_for_runtime(runtime, context)?.ok_or_else(|| {
        CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture system-audio planner missing while {context}"),
        }
    })?;
    ensure_audio_dir_exists(&planner.audio_dir())?;

    Ok(Some(
        planner
            .system_audio_resume_file(next_index, now_unix_ms())
            .to_string_lossy()
            .to_string(),
    ))
}

pub(super) fn reanchor_active_segment_timing(
    runtime: &mut NativeCaptureRuntime,
    context: &str,
) -> Result<(), CaptureErrorResponse> {
    let Some(schedule) = runtime.segment_schedule.as_ref() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture schedule missing while {context}"),
        });
    };

    if runtime.capture_clock.is_none() {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture clock missing while {context}"),
        });
    }

    runtime.capture_clock = Some(CaptureClock::start_now());
    reanchor_source_session_timing(
        runtime.source_sessions.as_mut(),
        runtime.current_segment_index,
        schedule,
        super::runtime::now_unix_ms(),
    );

    Ok(())
}

fn reanchored_source_session_started_at_unix_ms(
    now_unix_ms: u64,
    current_segment_index: u64,
    schedule: &SegmentSchedule,
) -> u64 {
    let duration_ms = schedule
        .segment_duration()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let offset_ms = current_segment_index
        .saturating_sub(1)
        .saturating_mul(duration_ms);

    now_unix_ms.saturating_sub(offset_ms)
}

fn reanchor_source_session_timing(
    source_sessions: Option<&mut SourceSessions>,
    current_segment_index: u64,
    schedule: &SegmentSchedule,
    now_unix_ms: u64,
) {
    let Some(source_sessions) = source_sessions else {
        return;
    };
    let started_at_unix_ms =
        reanchored_source_session_started_at_unix_ms(now_unix_ms, current_segment_index, schedule);

    if let Some(session) = source_sessions.screen.as_mut() {
        session.started_at_unix_ms = started_at_unix_ms;
    }
    if let Some(session) = source_sessions.microphone.as_mut() {
        session.started_at_unix_ms = started_at_unix_ms;
    }
    if let Some(session) = source_sessions.system_audio.as_mut() {
        session.started_at_unix_ms = started_at_unix_ms;
    }
}

pub(super) fn next_emitted_segment_index(current_segment_index: u64) -> u64 {
    current_segment_index + 1
}

pub(super) fn segment_loop_sleep_duration(
    schedule: &SegmentSchedule,
    clock: &CaptureClock,
) -> Duration {
    if schedule.segment_duration().is_zero() {
        return SEGMENT_LOOP_IDLE_POLL_INTERVAL;
    }

    schedule
        .sleep_until_next_boundary(clock)
        .min(SEGMENT_LOOP_IDLE_POLL_INTERVAL)
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(super) struct PlannedSegmentRotation {
    pub(super) next_index: u64,
    pub(super) segment_dir: PathBuf,
    pub(super) screen_output_file: PathBuf,
    pub(super) microphone_output_path: Option<PathBuf>,
    pub(super) system_audio_output_path: Option<PathBuf>,
}

#[cfg(target_os = "macos")]
pub(super) fn plan_live_rotation_segment(
    runtime: &NativeCaptureRuntime,
    sources: &CaptureSources,
    screen_planner: &SegmentPlanner,
    microphone_planner: Option<&SegmentPlanner>,
    system_audio_planner: Option<&SegmentPlanner>,
    schedule: &SegmentSchedule,
    clock: &CaptureClock,
) -> Option<PlannedSegmentRotation> {
    if !has_any_capture_sources(sources) {
        return None;
    }

    if !schedule.segment_duration_reached(clock.elapsed()) {
        return None;
    }

    let next_index = next_emitted_segment_index(runtime.current_segment_index);

    Some(PlannedSegmentRotation {
        next_index,
        segment_dir: screen_planner.segment_dir(next_index),
        screen_output_file: screen_planner.segment_screen_output(next_index),
        system_audio_output_path: sources
            .system_audio
            .then(|| {
                system_audio_planner
                    .as_ref()
                    .map(|planner| planner.system_audio_file(next_index))
            })
            .flatten(),
        microphone_output_path: sources
            .microphone
            .then(|| {
                microphone_planner
                    .as_ref()
                    .map(|planner| planner.microphone_file(next_index))
            })
            .flatten(),
    })
}

#[cfg(target_os = "macos")]
pub(super) fn recover_from_segment_finalize_error(
    context: &str,
    error: &CaptureErrorResponse,
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) -> bool {
    if !should_recover_from_segment_finalize_error(error) {
        return false;
    }

    cleanup_unusable_segment_artifacts(
        output_files,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
    );
    super::debug_log::log(format!(
        "recovered native capture segment finalization failure while {context}: [{}] {}",
        error.code, error.message
    ));

    true
}

fn capture_session_options(
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
    metadata_snapshot_provider: Option<metadata::FrameMetadataSnapshotProvider>,
    system_audio_inactivity_tail_trim_seconds: u64,
    initial_privacy_filter: Option<capture_screen::PrivacyContentFilter>,
) -> capture_screen::ScreenCaptureSessionOptions {
    let Some(frame_artifact_tx) = frame_artifact_tx else {
        return capture_screen::ScreenCaptureSessionOptions {
            system_audio_inactivity_tail_trim_seconds,
            initial_privacy_filter,
            ..Default::default()
        };
    };

    if !capture_screen::supports_frame_export() {
        return capture_screen::ScreenCaptureSessionOptions {
            system_audio_inactivity_tail_trim_seconds,
            initial_privacy_filter,
            ..Default::default()
        };
    }

    capture_screen::ScreenCaptureSessionOptions {
        frame_export: Some(capture_screen::ScreenFrameExportConfig {
            minimum_interval: capture_screen::DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL,
            on_frame_exported: Arc::new(move |artifact| {
                let metadata_snapshot = metadata_snapshot_provider
                    .as_ref()
                    .and_then(|provider| provider(artifact.captured_at_unix_ms));
                match try_forward_frame_artifact(&frame_artifact_tx, artifact, metadata_snapshot) {
                    FrameArtifactForwardingResult::Enqueued => {}
                    FrameArtifactForwardingResult::ReceiverClosed => {
                        super::debug_log::log(
                            "failed to forward native frame artifact for persistence: worker channel closed",
                        );
                    }
                }
            }),
        }),
        system_audio_inactivity_tail_trim_seconds,
        system_audio_writer_active: None,
        initial_privacy_filter,
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivacySuspensionRecoveryOutcome {
    NotSuspended,
    RetryPending,
    RestartRequired,
    Recovered,
}

#[cfg(target_os = "macos")]
fn screen_system_output_files(
    output_files: Option<&CaptureOutputFiles>,
    include_screen: bool,
    include_system_audio: bool,
) -> Option<CaptureOutputFiles> {
    let output_files = output_files?;
    let mut selected = empty_output_files();

    if include_screen {
        selected.screen_file = output_files.screen_file.clone();
        selected.screen_files = output_files.screen_files.clone();
    }

    if include_system_audio {
        selected.system_audio_file = output_files.system_audio_file.clone();
        selected.system_audio_files = output_files.system_audio_files.clone();
    }

    (selected.screen_file.is_some()
        || !selected.screen_files.is_empty()
        || selected.system_audio_file.is_some()
        || !selected.system_audio_files.is_empty())
    .then_some(selected)
}

#[cfg(target_os = "macos")]
fn explicit_privacy_suspension_sources(runtime: &NativeCaptureRuntime) -> CaptureSources {
    privacy_suspended_sources_for_runtime_state(runtime, runtime.inactivity.is_microphone_paused())
        .unwrap_or(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        })
}

#[cfg(target_os = "macos")]
fn active_sources_for_runtime_pause_state(
    runtime: &NativeCaptureRuntime,
    requested_sources: &CaptureSources,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
) -> Option<CaptureSources> {
    if runtime.capture_suspension.is_some() {
        return privacy_suspended_sources_for_runtime_state(runtime, microphone_paused);
    }

    active_sources_for_inactivity_paused_state(
        requested_sources,
        screen_paused,
        microphone_paused,
        system_audio_paused,
    )
}

// ---------------------------------------------------------------------------
// Low-disk safety (ADR 0040): preflight + segment-open boundary check.
//
// Free space is checked exactly when a new segment file is about to be opened —
// the preflight is this check applied to the first segment, and each rotation is
// the same check applied to the next segment. There is no continuous poll. All
// three of these helpers are pure logic over a measured free-space reading, so
// the suspend/resume behavior is exercisable with a scripted probe (see the
// `free_space_probe` runtime seam) without a real full disk.
// ---------------------------------------------------------------------------

/// The next-segment size estimate derived from a running runtime's stored screen
/// bitrate, segment duration, and requested audio sources. Mirrors the preflight
/// estimate so the rotation boundary reserves the same amount the preflight did.
#[cfg(target_os = "macos")]
fn next_segment_estimate_for_runtime(runtime: &NativeCaptureRuntime) -> u64 {
    let bitrate_bps = runtime.effective_screen_bitrate_bps.unwrap_or(0) as u64;
    let (microphone, system_audio) = runtime
        .requested_sources
        .as_ref()
        .map(|sources| (sources.microphone, sources.system_audio))
        .unwrap_or((false, false));
    let audio_bytes_per_sec =
        disk_space::audio_bytes_per_sec_for_sources(microphone, system_audio);
    let segment_duration_seconds = runtime
        .segment_schedule
        .as_ref()
        .map(|schedule| schedule.segment_duration().as_secs())
        .unwrap_or(0);
    disk_space::next_segment_estimate_bytes(
        bitrate_bps,
        audio_bytes_per_sec,
        segment_duration_seconds,
    )
}

/// Best-effort low-disk decision at a segment-open boundary using the runtime's
/// recordings root and injected probe. `None` means the free space could not be
/// measured (no existing ancestor to stat, or the probe errored) — best-effort
/// semantics, so the caller must not act on `None`. A `Some(decision)` reflects
/// the measured reading against the next-segment estimate.
#[cfg(target_os = "macos")]
pub(super) fn low_disk_decision_at_boundary(
    runtime: &NativeCaptureRuntime,
) -> Option<disk_space::LowDiskDecision> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    let free = disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())?;
    let estimate = next_segment_estimate_for_runtime(runtime);
    Some(disk_space::classify_free_space(free, estimate))
}

/// The recordings root the runtime is writing into, recovered from any source
/// planner (screen/mic/system-audio all share the same recordings root). Used to
/// probe free space at rotation/recovery time.
#[cfg(target_os = "macos")]
fn recordings_root_for_runtime(runtime: &NativeCaptureRuntime) -> Option<PathBuf> {
    screen_planner_for_runtime(runtime)
        .or(runtime.microphone_planner.as_ref())
        .or(runtime.system_audio_planner.as_ref())
        .map(|planner| PathBuf::from(planner.save_root_dir()))
}

/// Preflight free-space check for `start_capture_runtime`, applied to the first
/// segment before any file is opened. Best-effort: an unmeasurable reading
/// (`None`) never blocks the start; only a measured shortfall below the pause
/// threshold refuses with `insufficient_disk_space`.
#[cfg(target_os = "macos")]
pub(super) fn preflight_disk_space_check(
    recordings_root: &Path,
    effective_screen_bitrate_bps: Option<u32>,
    sources: &CaptureSources,
    segment_duration_seconds: u64,
    probe: disk_space::FreeSpaceProbe,
) -> Result<(), CaptureErrorResponse> {
    let bitrate_bps = effective_screen_bitrate_bps.unwrap_or(0) as u64;
    let audio_bytes_per_sec =
        disk_space::audio_bytes_per_sec_for_sources(sources.microphone, sources.system_audio);
    let estimate = disk_space::next_segment_estimate_bytes(
        bitrate_bps,
        audio_bytes_per_sec,
        segment_duration_seconds,
    );

    // Best-effort: if we cannot measure, do not block the start.
    let Some(free) = disk_space::measure_free_space(recordings_root, probe) else {
        return Ok(());
    };

    let pause_threshold = disk_space::pause_threshold_bytes(estimate);
    if free < pause_threshold {
        return Err(CaptureErrorResponse {
            code: "insufficient_disk_space".to_string(),
            message: format!(
                "Only {} free; Mnema needs ~{} to record.",
                disk_space::human_bytes(free),
                disk_space::human_bytes(pause_threshold)
            ),
        });
    }

    Ok(())
}

/// The outcome of a boundary low-disk check: whether the caller should skip
/// opening the next segment, and if so whether the session has been ended (a
/// graceful stop, so the loop must stop) or merely suspended (recovery resumes).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LowDiskBoundaryOutcome {
    /// Free space is sufficient (or unmeasurable): open the next segment.
    Proceed,
    /// Below the pause threshold but at/above the floor: entered a Low-Disk
    /// Suspension across all sources; skip the rotation, recovery will resume.
    Suspended,
    /// Below the critical floor: stopped the session gracefully; the segment loop
    /// must stop.
    Stopped,
}

/// Boundary low-disk check for the rotation path. Called when a new segment is
/// about to be opened (a rotation is due) and the runtime is not already
/// suspended. Best-effort: an unmeasurable reading returns `Proceed` and lets the
/// rotation proceed.
///
/// - `Pause` (free below the pause threshold but at/above the reserve floor):
///   enter the Low-Disk Suspension via the shared suspend entry and return
///   `Suspended` so the caller skips opening the next segment.
/// - `Critical` (free below the reserve floor): the app's own storage is at risk,
///   so stop gracefully (commit the healthy current segment, end the session) and
///   return `Stopped`.
#[cfg(target_os = "macos")]
pub(super) fn maybe_suspend_for_low_disk_at_boundary(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
) -> LowDiskBoundaryOutcome {
    // Recovery owns an already-suspended session; never re-enter here.
    if runtime.capture_suspension.is_some() {
        return LowDiskBoundaryOutcome::Proceed;
    }

    let Some(decision) = low_disk_decision_at_boundary(runtime) else {
        // Best-effort: cannot measure -> let the rotation proceed.
        return LowDiskBoundaryOutcome::Proceed;
    };

    match decision {
        disk_space::LowDiskDecision::Sufficient => LowDiskBoundaryOutcome::Proceed,
        // Below the critical reserve floor: stop gracefully rather than suspend.
        // The current segment is still healthy (only the *next* file can't be
        // opened), so commit it before ending.
        disk_space::LowDiskDecision::Critical => {
            let free = low_disk_free_at_boundary(runtime)
                .unwrap_or_else(disk_space::critical_threshold_bytes);
            graceful_stop_for_low_disk(Some(app_handle), runtime, free, true);
            LowDiskBoundaryOutcome::Stopped
        }
        disk_space::LowDiskDecision::Pause => {
            let estimate = next_segment_estimate_for_runtime(runtime);
            let pause_threshold = disk_space::pause_threshold_bytes(estimate);
            let error = CaptureErrorResponse {
                code: LOW_DISK_NOTIFICATION_ID.to_string(),
                message: format!(
                    "Capture paused: recordings volume low on free space (need ~{} to open the next segment).",
                    disk_space::human_bytes(pause_threshold)
                ),
            };
            if let Err(stop_error) = suspend_screen_system_audio_capture(
                Some(app_handle),
                runtime,
                &error,
                CaptureSuspensionKind::LowDisk,
            ) {
                super::debug_log::log(format!(
                    "low-disk suspension could not stop screen/system-audio capture; preserving runtime state: [{}] {}",
                    stop_error.code, stop_error.message
                ));
                return LowDiskBoundaryOutcome::Proceed;
            }
            super::debug_log::log(
                "capture paused: recordings volume low on disk; suspending all sources until free space recovers",
            );
            LowDiskBoundaryOutcome::Suspended
        }
    }
}

/// Re-measure the raw free bytes at the recordings root for the boundary stop
/// log. `None` when unmeasurable (the caller substitutes the floor for the log).
#[cfg(target_os = "macos")]
fn low_disk_free_at_boundary(runtime: &NativeCaptureRuntime) -> Option<u64> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())
}

#[cfg(target_os = "macos")]
pub(super) fn suspend_screen_system_audio_capture(
    app_handle: Option<&tauri::AppHandle>,
    runtime: &mut NativeCaptureRuntime,
    error: &CaptureErrorResponse,
    kind: CaptureSuspensionKind,
) -> Result<(), CaptureErrorResponse> {
    if let Err(stop_error) =
        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut runtime.active_screen_session,
            inactivity_tail_trim_seconds: 0,
        })
    {
        if capture_screen::should_preserve_runtime_on_stop_error(&stop_error) {
            return Err(stop_error);
        }
    }
    // Low disk is the only kind that suspends the microphone too: every source
    // writes to the same recordings volume, so the mic cannot keep writing while
    // the disk is too full to open the next segment. The other kinds keep the mic
    // alive (they only affect screen/system-audio capability), so gate the mic
    // stop on LowDisk. Recovery restarts the mic session when free space returns.
    if kind == CaptureSuspensionKind::LowDisk {
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            let _ = session.stop();
        }
        runtime.active_microphone_session = None;
        // `session.stop()` above finalized the current segment's mic `.m4a` on
        // disk. Commit it before dropping the handle: LowDisk uniquely stops the
        // mic, and `commit_suspended_screen_system_outputs` below only commits
        // screen/system-audio (microphone: false), so without this the finalized
        // mic file is orphaned — a real file on disk with no audio_segment row,
        // never transcribed and untracked by retention. That is up to ~5 min of
        // microphone audio lost on every low-disk pause; the normal rotation and
        // inactivity-pause paths both commit this file (ADR 0040 "the current
        // segment is still healthy ... so commit it before ending").
        commit_suspended_microphone_outputs(app_handle, runtime);
        runtime.microphone_recording_file = None;
    }
    runtime.current_segment_sources = Some(explicit_privacy_suspension_sources(runtime));
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(kind, error));
    // Commit the in-flight segment for every suspension kind, including
    // DisplayUnavailable: the stop above finalizes the writers even when the
    // delegate already reported the stream dead (terminated streams skip the
    // doomed second stop but still finish their writers), so the tail `.mov` is
    // openable and committing it preserves the last partial segment instead of
    // orphaning it. A finalize failure is logged and skipped inside.
    commit_suspended_screen_system_outputs(app_handle, runtime);
    runtime.recording_file = None;
    runtime.system_audio_recording_file = None;
    preserve_live_microphone_continuation_outputs(runtime);

    // Notify on entering a Low-Disk Suspension (unlike the silent
    // display-unavailable case): low disk only heals if the user frees space.
    // Emitting it from the shared suspend entry means Slice 5's mid-segment fill
    // path, which reuses this entry, gets the warning for free.
    if kind == CaptureSuspensionKind::LowDisk {
        if let Some(app_handle) = app_handle {
            super::push_warning_app_notification(
                app_handle,
                LOW_DISK_NOTIFICATION_ID,
                "Capture paused — low disk space",
                "Capture paused — low disk space. Free up space and recording resumes automatically.",
                None,
                now_unix_ms(),
            );
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn commit_suspended_screen_system_outputs(
    app_handle: Option<&tauri::AppHandle>,
    runtime: &mut NativeCaptureRuntime,
) {
    let Some(requested_sources) = runtime.requested_sources.as_ref() else {
        return;
    };
    let commit_sources = CaptureSources {
        screen: requested_sources.screen,
        microphone: false,
        system_audio: requested_sources.system_audio,
    };
    let Some(mut output_files) = screen_system_output_files(
        runtime.current_segment_output_files.as_ref(),
        commit_sources.screen,
        commit_sources.system_audio,
    ) else {
        return;
    };

    match finalize_capture_outputs(
        Some(&mut output_files),
        runtime.recording_file.as_deref(),
        None,
        runtime.system_audio_recording_file.as_deref(),
        Some(&commit_sources),
    ) {
        Ok(()) => {
            if let Some(committed) = runtime.output_files.as_mut() {
                append_committed_segment_output_files(committed, &output_files);
            }
            persist_committed_audio_segments(
                app_handle,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                Some(&output_files),
            );
            warm_scrub_previews_for_committed_screen_outputs(app_handle, Some(&output_files));
        }
        Err(error) => {
            super::debug_log::log(format!(
                "failed to finalize suspended screen/system-audio outputs; continuing privacy handling: [{}] {}",
                error.code, error.message
            ));
        }
    }
}

/// Commit the current segment's microphone audio when entering a Low-Disk
/// Suspension. LowDisk is the only kind that stops the mic; its `.m4a` was just
/// finalized on disk by `session.stop()` and lives in
/// `current_segment_output_files`. Mirrors [`commit_suspended_screen_system_outputs`]
/// for the mic source: validate + finalize the already-closed file, append it to
/// the committed `output_files`, and persist its audio segment row. Must run
/// before `preserve_live_microphone_continuation_outputs` clears
/// `current_segment_output_files` and before `microphone_recording_file` is
/// nulled. Best-effort: a finalize failure is logged and skipped; a missing mic
/// output is a no-op.
#[cfg(target_os = "macos")]
fn commit_suspended_microphone_outputs(
    app_handle: Option<&tauri::AppHandle>,
    runtime: &mut NativeCaptureRuntime,
) {
    let Some(output_files) = runtime.current_segment_output_files.as_ref() else {
        return;
    };
    if output_files.microphone_file.is_none() && output_files.microphone_files.is_empty() {
        return;
    }
    let mut microphone_outputs = empty_output_files();
    microphone_outputs.microphone_file = output_files.microphone_file.clone();
    microphone_outputs.microphone_files = output_files.microphone_files.clone();

    match finalize_capture_outputs(
        Some(&mut microphone_outputs),
        None,
        runtime.microphone_recording_file.as_deref(),
        None,
        Some(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
    ) {
        Ok(()) => {
            if let Some(committed) = runtime.output_files.as_mut() {
                append_committed_segment_output_files(committed, &microphone_outputs);
            }
            persist_committed_audio_segments(
                app_handle,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                Some(&microphone_outputs),
            );
        }
        Err(error) => {
            super::debug_log::log(format!(
                "failed to commit microphone outputs while entering low-disk suspension; continuing: [{}] {}",
                error.code, error.message
            ));
        }
    }
}

/// Graceful stop when free space has fallen below the critical reserve floor
/// (ADR 0040 "Graceful stop is spatial"). At or below `CRITICAL_FLOOR_BYTES` the
/// app's own SQLite DB / OCR / OS storage is at risk, so the session ends rather
/// than waiting for recovery.
///
/// Spatial, not timed: the caller passes the measured `free_bytes` that crossed
/// the critical floor purely for the single-line stop log.
///
/// `commit_current_segment` controls whether the in-flight segment is clean-
/// finalized before ending: a boundary/recovery stop has a healthy current
/// segment to commit (only the *next* file can't be opened), whereas a mid-
/// segment write-failure stop has already discarded its partial and passes
/// `false` so no half-written file is committed.
///
/// Ends via [`mark_runtime_session_failed`], which already clears the suspension
/// and (through the segment loop's fall-out broadcast) refreshes the frontend and
/// the native status bar. Clears the low-disk warning (if present) and pushes the
/// `error`-severity "Recording stopped — disk full." notification.
#[cfg(target_os = "macos")]
pub(super) fn graceful_stop_for_low_disk(
    app_handle: Option<&tauri::AppHandle>,
    runtime: &mut NativeCaptureRuntime,
    free_bytes: u64,
    commit_current_segment: bool,
) {
    // Clean-finalize what is safely closable: a still-writing current segment.
    // The mid-segment-fill caller already discarded its partial and passes
    // `false`, so only a boundary/recovery stop (healthy current segment, the
    // disk is merely too low to open the *next* file) commits here.
    if commit_current_segment {
        // Stop the screen session *before* committing: a ScreenCaptureKit `.mov`
        // is only finalized (moov atom written, file readable/convertible) by
        // `stop()` -> `finish_writing()`. `commit_suspended_screen_system_outputs`
        // reads/converts the `.mov` via `finalize_capture_outputs`, so reading it
        // while the session is still live yields an un-finalized file and drops
        // the current segment. Every other commit site stops first (see
        // `suspend_screen_system_audio_capture`); the later
        // `stop_active_sessions_after_failure` is then idempotent for the screen
        // session. The mic is finalized through its own live session next, so it
        // must NOT be stopped here.
        if let Err(stop_error) =
            capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                active_session: &mut runtime.active_screen_session,
                inactivity_tail_trim_seconds: 0,
            })
        {
            super::debug_log::log(format!(
                "failed stopping screen session before low-disk graceful-stop commit; continuing: [{}] {}",
                stop_error.code, stop_error.message
            ));
        }
        commit_suspended_screen_system_outputs(app_handle, runtime);
        finalize_live_microphone_continuation_on_stop(app_handle, runtime);
    }

    // Stop every live source so no writer keeps a file open as we end. (The mic is
    // included; LowDisk is the only kind that stops it.)
    stop_active_sessions_after_failure(runtime);

    // End the session: this broadcasts native_capture_session_changed + refreshes
    // the status bar via the segment loop's internal-end fall-out, and clears the
    // suspension slot.
    super::runtime::mark_runtime_session_failed(runtime);

    if let Some(app_handle) = app_handle {
        // Replace the (transient, cleared-on-resume) low-disk warning with the
        // persistent graceful-stop error: capture has stopped, not paused.
        super::clear_app_notification_by_id(app_handle, LOW_DISK_NOTIFICATION_ID);
        super::push_error_app_notification(
            app_handle,
            DISK_FULL_STOPPED_NOTIFICATION_ID,
            DISK_FULL_STOPPED_TITLE,
            DISK_FULL_STOPPED_MESSAGE,
            None,
            now_unix_ms(),
        );
    }

    super::debug_log::log(format!(
        "capture stopped: recordings volume free space fell below the critical reserve floor; ending session to protect app storage ({} free, floor {})",
        disk_space::human_bytes(free_bytes),
        disk_space::human_bytes(disk_space::critical_threshold_bytes()),
    ));
}

/// Finalize a still-live microphone continuation when stopping, mirroring the
/// commit path [`commit_suspended_screen_system_outputs`] does for screen/system
/// audio. Best-effort: a finalize failure is logged and skipped (we are stopping
/// regardless), and a missing session/output is a no-op.
#[cfg(target_os = "macos")]
fn finalize_live_microphone_continuation_on_stop(
    app_handle: Option<&tauri::AppHandle>,
    runtime: &mut NativeCaptureRuntime,
) {
    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let Some(session) = runtime.active_microphone_session.as_mut() else {
        return;
    };
    let finalization = match session.pause_output_file_for_inactivity(0, 0.0) {
        Ok(finalization) => finalization,
        Err(error) => {
            super::debug_log::log(format!(
                "failed to finalize microphone continuation during low-disk graceful stop; continuing stop: [{}] {}",
                error.code, error.message
            ));
            return;
        }
    };

    let Some(output_files) = runtime.current_segment_output_files.as_ref() else {
        return;
    };
    let mut microphone_outputs = empty_output_files();
    microphone_outputs.microphone_file = output_files.microphone_file.clone();
    microphone_outputs.microphone_files = output_files.microphone_files.clone();
    apply_microphone_output_finalization(
        Some(&mut microphone_outputs),
        &finalization,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
    );
    if let Err(error) = finalize_capture_outputs(
        Some(&mut microphone_outputs),
        None,
        finalization
            .output_file
            .as_deref()
            .or(microphone_recording_file.as_deref()),
        None,
        Some(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
    ) {
        super::debug_log::log(format!(
            "failed to commit microphone continuation during low-disk graceful stop; continuing stop: [{}] {}",
            error.code, error.message
        ));
        return;
    }

    if let Some(committed) = runtime.output_files.as_mut() {
        append_committed_segment_output_files(committed, &microphone_outputs);
    }
    persist_committed_audio_segments(
        app_handle,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
        Some(&microphone_outputs),
    );
    runtime.microphone_recording_file = None;
}

/// Best-effort discard of a mid-segment partial that failed to write because the
/// disk filled (ADR 0040 "Mid-segment disk-full → no corrupt segment"). There is
/// no temp-file-then-atomic-rename, so the writer leaves a corrupt/partial file at
/// its *final* path; this deletes each such path (`remove_file`, NotFound ignored)
/// and commits NO Capture Segment row — the caller drops the partial entirely and
/// then suspends (Pause) or stops (Critical).
///
/// Returns `false` to signal "no segment row committed" so a caller can treat the
/// rotation's `previous_segment_committed` flag uniformly. Pure I/O over the paths
/// it is given, so it is unit-testable without a real full disk.
#[cfg(target_os = "macos")]
pub(super) fn discard_partial_segment_on_disk_full(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) -> bool {
    // The in-flight recording-file handles are the live final paths; the
    // committed output_files list may also carry the partial. Delete every
    // distinct final path we know about so no corrupt file survives.
    let mut paths: Vec<String> = Vec::new();
    let mut push = |candidate: Option<&str>| {
        if let Some(path) = candidate {
            if !path.is_empty() && !paths.iter().any(|existing| existing == path) {
                paths.push(path.to_string());
            }
        }
    };
    push(recording_file);
    push(microphone_recording_file);
    push(system_audio_recording_file);
    if let Some(output_files) = output_files {
        push(output_files.screen_file.as_deref());
        push(output_files.microphone_file.as_deref());
        push(output_files.system_audio_file.as_deref());
        for path in output_files
            .screen_files
            .iter()
            .chain(output_files.microphone_files.iter())
            .chain(output_files.system_audio_files.iter())
        {
            push(Some(path.as_str()));
        }
    }

    for path in &paths {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                super::debug_log::log(format!(
                    "failed removing disk-full partial capture file {path}: {error}"
                ));
            }
        }
    }

    // No Capture Segment row is committed for the discarded partial.
    false
}

/// Re-probe free space at the recordings root after a writer append/finalize
/// failure to decide whether the failure coincides with low free space (i.e. it
/// is a disk-full failure). `None` means the failure did NOT coincide with low
/// free space (healthy disk, or unmeasurable — keep the existing failure
/// behavior); `Some((decision, free))` means low disk and the caller should
/// discard the partial and then suspend (`Pause`) or stop (`Critical`).
///
/// Best-effort: an unmeasurable probe returns `None` so an unrelated write
/// failure on a healthy-but-unstatable volume still falls back to existing
/// behavior and is never forced into a stop.
#[cfg(target_os = "macos")]
pub(super) fn classify_write_failure_disk_full(
    runtime: &NativeCaptureRuntime,
) -> Option<(disk_space::LowDiskDecision, u64)> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    let free = disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())?;
    let estimate = next_segment_estimate_for_runtime(runtime);
    match disk_space::classify_free_space(free, estimate) {
        // A failure while free space is healthy is unrelated to disk space: keep
        // the existing failure behavior (return None).
        disk_space::LowDiskDecision::Sufficient => None,
        decision @ (disk_space::LowDiskDecision::Pause | disk_space::LowDiskDecision::Critical) => {
            Some((decision, free))
        }
    }
}

/// What a mid-segment writer failure resolved to once free space was re-probed.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriteFailureDiskFullOutcome {
    /// The failure did not coincide with low free space (healthy or unmeasurable
    /// disk): keep the EXISTING failure behavior unchanged.
    NotDiskFull,
    /// Disk-full at/above the floor: the partial was discarded and a Low-Disk
    /// Suspension entered; the loop should skip the rotation and let recovery
    /// resume once free space returns.
    Suspended,
    /// Disk-full below the floor: the partial was discarded and the session was
    /// stopped gracefully; the loop should stop.
    Stopped,
}

/// Reactive mid-segment disk-full handling (ADR 0040 "Mid-segment disk-full → no
/// corrupt segment"). Called from the segment loop's existing writer
/// append/finalize-failure handling. Re-probes free space at the recordings root;
/// if the failure coincides with low free space it is treated as disk-full:
/// best-effort delete the partial file at its FINAL path (committing NO Capture
/// Segment row for it) and then either enter a Low-Disk Suspension (`Pause`) or
/// stop gracefully (`Critical`).
///
/// When free space is healthy (or unmeasurable), returns `NotDiskFull` and does
/// nothing — the caller keeps its existing failure behavior. This is purely
/// additive for the disk-full case.
#[cfg(target_os = "macos")]
pub(super) fn handle_mid_segment_write_failure_for_low_disk(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) -> WriteFailureDiskFullOutcome {
    let Some((decision, free)) = classify_write_failure_disk_full(runtime) else {
        // Healthy or unmeasurable disk: not a disk-full failure — leave the
        // existing failure behavior intact.
        return WriteFailureDiskFullOutcome::NotDiskFull;
    };

    // Disk-full: discard the partial at its final path(s) and commit no row. The
    // discard is the single guarantee against a corrupt/locked file surviving, run
    // before either the suspend or the stop branch.
    let _committed = discard_partial_segment_on_disk_full(
        output_files,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
    );

    // Stop every live source so no broken writer keeps a file open.
    stop_active_sessions_after_failure(runtime);

    match decision {
        // Below the reserve floor: stop gracefully. The partial was already
        // discarded, so do not re-commit a current segment.
        disk_space::LowDiskDecision::Critical => {
            graceful_stop_for_low_disk(Some(app_handle), runtime, free, false);
            super::debug_log::log(format!(
                "mid-segment disk-full below the reserve floor; discarded the partial and stopped gracefully ({} free)",
                disk_space::human_bytes(free)
            ));
            WriteFailureDiskFullOutcome::Stopped
        }
        // At/above the floor: discard the partial and enter the Low-Disk
        // Suspension so recovery resumes once free space returns.
        disk_space::LowDiskDecision::Pause => {
            enter_low_disk_suspension_after_partial_discard(app_handle, runtime);
            super::debug_log::log(format!(
                "mid-segment disk-full; discarded the partial and suspended all sources until free space recovers ({} free)",
                disk_space::human_bytes(free)
            ));
            WriteFailureDiskFullOutcome::Suspended
        }
        disk_space::LowDiskDecision::Sufficient => {
            // Unreachable: classify_write_failure_disk_full returns None for
            // Sufficient. Kept total for safety — fall back to existing behavior.
            WriteFailureDiskFullOutcome::NotDiskFull
        }
    }
}

/// Enter a Low-Disk Suspension after a mid-segment partial has already been
/// discarded. Unlike [`suspend_screen_system_audio_capture`], this does NOT try
/// to commit the in-flight (failed) segment — its writers are broken and its
/// partial was just deleted. It only clears the in-flight segment refs, sets the
/// suspension slot across all sources, and pushes the low-disk warning. Recovery
/// (`resume_all_sources_after_low_disk`) starts a fresh segment from
/// `requested_sources` + `current_segment_index`, so no live segment state is
/// needed to resume.
#[cfg(target_os = "macos")]
fn enter_low_disk_suspension_after_partial_discard(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
) {
    runtime.active_microphone_session = None;
    runtime.microphone_recording_file = None;
    runtime.recording_file = None;
    runtime.system_audio_recording_file = None;
    runtime.current_segment_output_files = None;
    runtime.current_segment_sources = Some(explicit_privacy_suspension_sources(runtime));

    let estimate = next_segment_estimate_for_runtime(runtime);
    let pause_threshold = disk_space::pause_threshold_bytes(estimate);
    let error = CaptureErrorResponse {
        code: LOW_DISK_NOTIFICATION_ID.to_string(),
        message: format!(
            "Capture paused: recordings volume filled mid-segment (need ~{} to open the next segment).",
            disk_space::human_bytes(pause_threshold)
        ),
    };
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(
        CaptureSuspensionKind::LowDisk,
        &error,
    ));

    super::push_warning_app_notification(
        app_handle,
        LOW_DISK_NOTIFICATION_ID,
        "Capture paused — low disk space",
        "Capture paused — low disk space. Free up space and recording resumes automatically.",
        None,
        now_unix_ms(),
    );
}

#[cfg(target_os = "macos")]
fn attempt_privacy_suspension_recovery(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
) -> PrivacySuspensionRecoveryOutcome {
    let suspension_kind = match runtime.capture_suspension.as_ref() {
        Some(suspension) => suspension.kind,
        None => return PrivacySuspensionRecoveryOutcome::NotSuspended,
    };

    // Low disk recovers on a different axis (free space, not a returning display
    // or a re-applicable privacy filter) and uniquely must restart the mic too, so
    // it owns a dedicated recovery path.
    if suspension_kind == CaptureSuspensionKind::LowDisk {
        return attempt_low_disk_recovery(app_handle, runtime);
    }

    // For a transient display loss, don't attempt a (noisy, churny) capture
    // restart until a display is actually back — otherwise every poll would hit
    // ScreenCaptureKit with a doomed start and log another "no displays" error
    // while the screen is locked or the display is asleep. Waiting is not a
    // recovery failure, so leave the retry budget untouched.
    if suspension_kind == CaptureSuspensionKind::DisplayUnavailable
        && !capture_screen::screen_display_available()
    {
        return PrivacySuspensionRecoveryOutcome::RetryPending;
    }

    let can_retry = runtime
        .capture_suspension
        .as_ref()
        .is_some_and(CaptureSuspension::can_retry);
    if !can_retry {
        return PrivacySuspensionRecoveryOutcome::RestartRequired;
    }

    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return PrivacySuspensionRecoveryOutcome::RestartRequired;
    };
    let recover_sources = CaptureSources {
        screen: requested_sources.screen && !runtime.inactivity.is_screen_paused(),
        microphone: false,
        system_audio: requested_sources.system_audio
            && !runtime.inactivity.is_screen_paused()
            && !runtime.inactivity.is_system_audio_paused(),
    };
    if !recover_sources.screen && !recover_sources.system_audio {
        runtime.capture_suspension = None;
        return PrivacySuspensionRecoveryOutcome::NotSuspended;
    }

    let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
        if let Some(suspension) = runtime.capture_suspension.as_mut() {
            suspension.record_recovery_failure(&CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: "Capture screen planner missing while recovering privacy suspension"
                    .to_string(),
            });
        }
        return PrivacySuspensionRecoveryOutcome::RetryPending;
    };
    let system_audio_planner = if recover_sources.system_audio {
        match ensure_system_audio_planner_for_runtime(runtime, "recovering privacy suspension") {
            Ok(planner) => planner,
            Err(error) => {
                if let Some(suspension) = runtime.capture_suspension.as_mut() {
                    suspension.record_recovery_failure(&error);
                }
                return PrivacySuspensionRecoveryOutcome::RetryPending;
            }
        }
    } else {
        None
    };

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);
    let system_audio_output_path = recover_sources.system_audio.then(|| {
        system_audio_planner
            .as_ref()
            .expect("system audio planner should exist when recovering system audio")
            .system_audio_file(next_index)
    });

    let started_segment = start_segment_with_current_privacy_filter(
        app_handle,
        &segment_dir,
        Some(&screen_output_file),
        system_audio_output_path.as_deref(),
        &recover_sources,
        runtime.screen_frame_rate,
        &runtime.screen_resolution,
        runtime.effective_screen_bitrate_bps,
        None,
        runtime.frame_artifact_tx.clone(),
        None,
    );

    let (
        mut segment_outputs,
        recording_file,
        _microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        _active_microphone_session,
    ) = match started_segment {
        Ok(value) => value,
        Err(error) => {
            if let Some(suspension) = runtime.capture_suspension.as_mut() {
                suspension.record_recovery_failure(&error);
                if suspension.status == CaptureSuspensionStatus::RestartRequired {
                    super::debug_log::log(format!(
                        "privacy recovery restart attempts exhausted; screen/system-audio require manual stop/start: [{}] {}",
                        error.code, error.message
                    ));
                    super::push_privacy_recovery_restart_required_notification(app_handle);
                    return PrivacySuspensionRecoveryOutcome::RestartRequired;
                }
            }
            super::debug_log::log(format!(
                "privacy recovery restart failed; screen/system-audio remain suspended: [{}] {}",
                error.code, error.message
            ));
            return PrivacySuspensionRecoveryOutcome::RetryPending;
        }
    };

    commit_suspended_screen_system_outputs(Some(app_handle), runtime);
    merge_live_microphone_continuation_into_segment_outputs(runtime, &mut segment_outputs);
    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.recording_file = recording_file;
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    runtime.capture_suspension = None;
    if let Err(error) = reanchor_active_segment_timing(runtime, "recovering privacy suspension") {
        super::debug_log::log(format!(
            "failed to reanchor segment timing after privacy recovery: [{}] {}",
            error.code, error.message
        ));
    }

    PrivacySuspensionRecoveryOutcome::Recovered
}

/// Whether a Low-Disk Suspension may resume right now, re-probing free space at
/// the recordings root. `None` means free space is currently unmeasurable (stay
/// suspended, keep retrying); `Some(true)` means it has climbed to the resume
/// threshold (hysteresis) and `Some(false)` means it is still short. Pure over
/// the injected probe — the testable resume-decision seam mirroring the gate in
/// [`attempt_low_disk_recovery`] (which inlines the same `disk_space` primitives
/// because it also needs the measured free bytes for classify + logging).
#[cfg(all(target_os = "macos", test))]
pub(super) fn low_disk_can_resume(runtime: &NativeCaptureRuntime) -> Option<bool> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    let free = disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())?;
    let estimate = next_segment_estimate_for_runtime(runtime);
    Some(disk_space::can_resume(free, estimate))
}

/// Recovery path for a [`CaptureSuspensionKind::LowDisk`] suspension. Re-probes
/// free space and, once it has recovered to the resume threshold, restarts a
/// fresh segment across all suspended sources — including the microphone, which
/// only LowDisk stops. Otherwise it stays suspended and keeps retrying.
#[cfg(target_os = "macos")]
fn attempt_low_disk_recovery(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
) -> PrivacySuspensionRecoveryOutcome {
    let recordings_root = match recordings_root_for_runtime(runtime) {
        Some(root) => root,
        // No planner to derive a recordings root from: cannot probe, so wait.
        None => return PrivacySuspensionRecoveryOutcome::RetryPending,
    };
    let estimate = next_segment_estimate_for_runtime(runtime);
    let free = match disk_space::measure_free_space(&recordings_root, runtime.free_space_probe()) {
        Some(free) => free,
        // Best-effort: cannot measure right now -> stay suspended, retry later.
        None => return PrivacySuspensionRecoveryOutcome::RetryPending,
    };

    match disk_space::classify_free_space(free, estimate) {
        // Free space kept falling while suspended and crossed the critical reserve
        // floor: the app's own storage is now at risk, so stop gracefully rather
        // than keep waiting. The suspended session has no live current segment to
        // commit (it was committed at suspend time), so do not re-commit here.
        disk_space::LowDiskDecision::Critical => {
            graceful_stop_for_low_disk(Some(app_handle), runtime, free, false);
            return PrivacySuspensionRecoveryOutcome::NotSuspended;
        }
        // Still below the pause threshold but above the floor: stay suspended.
        disk_space::LowDiskDecision::Pause => {
            return PrivacySuspensionRecoveryOutcome::RetryPending;
        }
        // Sufficient (cleared the pause threshold) is necessary but not sufficient
        // to resume: hysteresis requires the higher resume threshold. Fall through
        // to the resume gate below.
        disk_space::LowDiskDecision::Sufficient => {}
    }

    if !disk_space::can_resume(free, estimate) {
        // In the hysteresis band: cleared pause but not yet resume. Stay suspended
        // so we don't flap back into a pause as soon as one more segment is opened.
        return PrivacySuspensionRecoveryOutcome::RetryPending;
    }

    match resume_all_sources_after_low_disk(app_handle, runtime) {
        Ok(()) => {
            runtime.capture_suspension = None;
            super::clear_app_notification_by_id(app_handle, LOW_DISK_NOTIFICATION_ID);
            super::debug_log::log(format!(
                "recovered: recordings volume free space back above resume threshold; restarted all sources ({} free)",
                disk_space::human_bytes(free)
            ));
            PrivacySuspensionRecoveryOutcome::Recovered
        }
        Err(error) => {
            if let Some(suspension) = runtime.capture_suspension.as_mut() {
                suspension.record_recovery_failure(&error);
            }
            super::debug_log::log(format!(
                "low-disk recovery restart failed; sources remain suspended: [{}] {}",
                error.code, error.message
            ));
            PrivacySuspensionRecoveryOutcome::RetryPending
        }
    }
}

/// Restart a fresh segment across every source a Low-Disk Suspension stopped:
/// screen + system audio (via the shared privacy-filter segment start) and the
/// microphone (its own native session). Mutates the runtime to point at the new
/// segment on success; leaves it suspended on error so the caller can retry.
#[cfg(target_os = "macos")]
fn resume_all_sources_after_low_disk(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Requested sources missing while recovering low-disk suspension".to_string(),
        });
    };

    let recover_sources = CaptureSources {
        screen: requested_sources.screen && !runtime.inactivity.is_screen_paused(),
        microphone: requested_sources.microphone && !runtime.inactivity.is_microphone_paused(),
        system_audio: requested_sources.system_audio
            && !runtime.inactivity.is_screen_paused()
            && !runtime.inactivity.is_system_audio_paused(),
    };

    let next_index = next_emitted_segment_index(runtime.current_segment_index);

    // --- Screen + system audio (shared screen-capture backend) ---
    let screen_or_system = recover_sources.screen || recover_sources.system_audio;
    let mut next_segment_outputs = empty_output_files();
    let mut next_recording_file: Option<String> = None;
    let mut next_system_audio_recording_file: Option<String> = None;

    if screen_or_system {
        let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
            return Err(CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: "Capture screen planner missing while recovering low-disk suspension"
                    .to_string(),
            });
        };
        let system_audio_planner = if recover_sources.system_audio {
            ensure_system_audio_planner_for_runtime(runtime, "recovering low-disk suspension")?
        } else {
            None
        };
        let segment_dir = screen_planner.segment_dir(next_index);
        let screen_output_file = screen_planner.segment_screen_output(next_index);
        let system_audio_output_path = recover_sources.system_audio.then(|| {
            system_audio_planner
                .as_ref()
                .expect("system audio planner should exist when recovering system audio")
                .system_audio_file(next_index)
        });
        let screen_system_sources = CaptureSources {
            screen: recover_sources.screen,
            microphone: false,
            system_audio: recover_sources.system_audio,
        };

        let (
            segment_outputs,
            recording_file,
            _microphone_recording_file,
            system_audio_recording_file,
            active_screen_session,
            _active_microphone_session,
        ) = start_segment_with_current_privacy_filter(
            app_handle,
            &segment_dir,
            Some(&screen_output_file),
            system_audio_output_path.as_deref(),
            &screen_system_sources,
            runtime.screen_frame_rate,
            &runtime.screen_resolution,
            runtime.effective_screen_bitrate_bps,
            None,
            runtime.frame_artifact_tx.clone(),
            None,
        )?;

        next_segment_outputs = segment_outputs;
        next_recording_file = recording_file;
        next_system_audio_recording_file = system_audio_recording_file;
        runtime.active_screen_session = active_screen_session;
    }

    // --- Microphone (separate native session) ---
    // The screen/system-audio session above is now live on `runtime`. If any
    // microphone-restart step fails we must stop the live sessions before
    // returning Err: the caller (`attempt_low_disk_recovery`) records the failure
    // but leaves the runtime suspended and retries every ~10s, so a leftover live
    // screen session is an orphaned writer the next attempt would overwrite —
    // leaking it (there is no `Drop` that stops a screen session) and opening a
    // second writer over overlapping segment files. Mirrors the rollback the
    // combined start path already does on a mic-start failure.
    if recover_sources.microphone {
        let restart_result = (|| -> Result<(), CaptureErrorResponse> {
            ensure_microphone_planner_for_runtime(runtime, "recovering low-disk suspension")?;
            refresh_runtime_planner_dates(runtime);
            let microphone_output_file =
                super::microphone::next_microphone_output_file_for_runtime(runtime)?;
            let session =
                microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
                    &microphone_output_file,
                    runtime.microphone_device_id_for_capture.as_deref(),
                )?;
            runtime.active_microphone_session = Some(session);
            runtime.microphone_recording_file = Some(microphone_output_file.clone());
            set_current_microphone_output_file(&mut next_segment_outputs, microphone_output_file);
            Ok(())
        })();
        if let Err(error) = restart_result {
            stop_active_sessions_after_failure(runtime);
            return Err(error);
        }
    }

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(next_segment_outputs);
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.recording_file = next_recording_file;
    runtime.system_audio_recording_file = next_system_audio_recording_file;

    if let Err(error) = reanchor_active_segment_timing(runtime, "recovering low-disk suspension") {
        super::debug_log::log(format!(
            "failed to reanchor segment timing after low-disk recovery: [{}] {}",
            error.code, error.message
        ));
    }

    Ok(())
}

fn spawn_frame_artifact_worker(
    app_handle: &tauri::AppHandle,
    session_id: String,
) -> mpsc::Sender<FrameArtifactMessage> {
    let (tx, mut rx) = mpsc::channel(FRAME_ARTIFACT_BUFFER_CAPACITY);
    let app_handle = app_handle.clone();
    let infra = Arc::clone(&*app_handle.state::<crate::app_infra::AppInfraState>());

    tauri::async_runtime::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                FrameArtifactMessage::Artifact(envelope) => {
                    if let Err(error) = crate::app_infra::persist_screen_frame_artifact(
                        infra.as_ref(),
                        app_handle
                            .state::<crate::native_capture::RecordingSettingsState>()
                            .inner(),
                        envelope.metadata_snapshot,
                        &session_id,
                        envelope.artifact,
                    )
                    .await
                    {
                        super::debug_log::log(format!(
                            "failed to persist native frame artifact: {error}"
                        ));
                    }
                }
                FrameArtifactMessage::Flush(response_tx) => {
                    let _ = response_tx.send(());
                }
            }
        }
    });

    tx
}

#[cfg(target_os = "macos")]
fn rfc3339_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(target_os = "macos")]
fn audio_file_duration_ms(file_path: &str) -> Option<u64> {
    use cidre::{av, ns};

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let result = {
        let url = ns::Url::with_fs_path_str(file_path, false);
        let asset = av::UrlAsset::with_url(&url, None)?;
        audio_duration_time_to_ms(asset.duration())
    };

    result
}

#[cfg(target_os = "macos")]
pub(super) fn audio_duration_time_to_ms(duration: cidre::cm::Time) -> Option<u64> {
    if !duration.is_numeric() || duration.value <= 0 || duration.scale <= 0 {
        return None;
    }

    let value_ms = i128::from(duration.value)
        .checked_mul(1_000)?
        .checked_add(i128::from(duration.scale / 2))?
        / i128::from(duration.scale);

    u64::try_from(value_ms).ok()
}

#[cfg(target_os = "macos")]
fn scheduled_audio_segment_started_at_unix_ms(
    source_session: &SourceSessionMeta,
    segment_index: u64,
    schedule: &SegmentSchedule,
) -> u64 {
    let duration_ms = schedule
        .segment_duration()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let offset_ms = segment_index.saturating_sub(1).saturating_mul(duration_ms);
    source_session.started_at_unix_ms.saturating_add(offset_ms)
}

#[cfg(target_os = "macos")]
fn audio_segment_window_for_file(
    source_session: &SourceSessionMeta,
    segment_index: u64,
    schedule: &SegmentSchedule,
    file_path: &str,
) -> (String, String) {
    let started_at_unix_ms = audio_segment_started_at_unix_ms_for_file(
        source_session,
        segment_index,
        schedule,
        file_path,
    );
    let scheduled_duration_ms = schedule
        .segment_duration()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let duration_ms = audio_file_duration_ms(file_path).unwrap_or(scheduled_duration_ms);

    audio_segment_window_from_duration_ms(started_at_unix_ms, duration_ms)
}

#[cfg(target_os = "macos")]
pub(super) fn audio_segment_started_at_unix_ms_for_file(
    source_session: &SourceSessionMeta,
    segment_index: u64,
    schedule: &SegmentSchedule,
    file_path: &str,
) -> u64 {
    parse_audio_restart_started_at_unix_ms(file_path).unwrap_or_else(|| {
        scheduled_audio_segment_started_at_unix_ms(source_session, segment_index, schedule)
    })
}

#[cfg(target_os = "macos")]
pub(super) fn audio_segment_window_from_duration_ms(
    started_at_unix_ms: u64,
    duration_ms: u64,
) -> (String, String) {
    let ended_at_unix_ms = started_at_unix_ms.saturating_add(duration_ms);

    (
        rfc3339_from_unix_ms(started_at_unix_ms),
        rfc3339_from_unix_ms(ended_at_unix_ms),
    )
}

#[cfg(target_os = "macos")]
pub(super) fn committed_audio_segments_for_output_files(
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) -> Vec<::app_infra::NewAudioSegment> {
    let (Some(source_sessions), Some(schedule), Some(output_files)) =
        (source_sessions, schedule, output_files)
    else {
        return Vec::new();
    };
    let Ok(segment_index_i64) = i64::try_from(segment_index) else {
        return Vec::new();
    };

    let mut segments = Vec::new();
    if let Some(source_session) = source_sessions.microphone.as_ref() {
        segments.extend(
            output_files
                .microphone_files
                .iter()
                .filter_map(|file_path| {
                    if !Path::new(file_path).is_file() {
                        return None;
                    }
                    let (started_at, ended_at) = audio_segment_window_for_file(
                        source_session,
                        segment_index,
                        schedule,
                        file_path,
                    );
                    Some(::app_infra::NewAudioSegment::new(
                        ::app_infra::AudioSegmentSourceKind::Microphone,
                        source_session.session_id.clone(),
                        segment_index_i64,
                        file_path.clone(),
                        started_at.clone(),
                        ended_at.clone(),
                    ))
                }),
        );
    }
    if let Some(source_session) = source_sessions.system_audio.as_ref() {
        segments.extend(
            output_files
                .system_audio_files
                .iter()
                .filter_map(|file_path| {
                    if !Path::new(file_path).is_file() {
                        return None;
                    }
                    let (started_at, ended_at) = audio_segment_window_for_file(
                        source_session,
                        segment_index,
                        schedule,
                        file_path,
                    );
                    Some(::app_infra::NewAudioSegment::new(
                        ::app_infra::AudioSegmentSourceKind::SystemAudio,
                        source_session.session_id.clone(),
                        segment_index_i64,
                        file_path.clone(),
                        started_at.clone(),
                        ended_at.clone(),
                    ))
                }),
        );
    }

    segments
}

#[cfg(target_os = "macos")]
fn transcription_admission_for_app_handle(
    app_handle: &tauri::AppHandle,
) -> ::app_infra::AudioSegmentTranscriptionAdmission {
    let transcription_settings = match app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.transcription.clone(),
        Err(_) => {
            super::debug_log::log(
                "failed to read recording settings for audio transcription admission".to_string(),
            );
            return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
        }
    };

    if !transcription_settings.enabled || !transcription_settings.microphone_enabled {
        return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
    }

    let available = match app_handle.path().app_data_dir() {
        Ok(app_data_dir) => {
            crate::audio_transcription_models::selected_audio_transcription_model_available(
                &app_data_dir,
                &transcription_settings,
            )
        }
        Err(error) => {
            super::debug_log::log(format!(
                "failed to resolve app data directory for audio transcription admission: {error}"
            ));
            return ::app_infra::AudioSegmentTranscriptionAdmission::unavailable();
        }
    };

    match available {
        Ok(true) => {
            let provider = crate::audio_transcription_models::provider_id_for_settings(
                transcription_settings.provider,
            );
            let mut payload = ::app_infra::AudioTranscriptionJobPayload::new(
                provider,
                transcription_settings.model_id.clone(),
                transcription_settings.language.clone(),
            );
            payload.options =
                crate::audio_transcription_models::transcription_request_options_for_settings(
                    &transcription_settings,
                );
            let speaker_admission = speaker_analysis_admission_for_app_handle(app_handle);
            if speaker_admission.enabled && speaker_admission.provider_available {
                if let Some(payload_json) = speaker_admission.payload_json.as_deref() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) {
                        payload.options.insert(
                            ::app_infra::SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                            value,
                        );
                    }
                }
            }
            match serde_json::to_string(&payload) {
                Ok(payload_json) => {
                    ::app_infra::AudioSegmentTranscriptionAdmission::available(payload_json)
                }
                Err(error) => {
                    super::debug_log::log(format!(
                        "failed to serialize audio transcription admission payload: {error}"
                    ));
                    ::app_infra::AudioSegmentTranscriptionAdmission::unavailable()
                }
            }
        }
        Ok(false) => ::app_infra::AudioSegmentTranscriptionAdmission::unavailable(),
        Err(error) => {
            super::debug_log::log(format!(
                "failed to inspect selected audio transcription model: {error}"
            ));
            ::app_infra::AudioSegmentTranscriptionAdmission::unavailable()
        }
    }
}

#[cfg(target_os = "macos")]
fn system_audio_speech_admission_for_app_handle(
    app_handle: &tauri::AppHandle,
) -> ::app_infra::SystemAudioSpeechActivityAdmission {
    let settings = match app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.clone(),
        Err(_) => {
            super::debug_log::log(
                "failed to read recording settings for system-audio speech admission".to_string(),
            );
            return ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
        }
    };

    if !settings.transcription.enabled || !settings.transcription.system_audio_enabled {
        return ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
    }
    if settings.audio_speech_detection.detector == capture_types::AudioSpeechDetector::Off {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    }

    let transcription_admission = {
        let available = match app_handle.path().app_data_dir() {
            Ok(app_data_dir) => {
                crate::audio_transcription_models::selected_audio_transcription_model_available(
                    &app_data_dir,
                    &settings.transcription,
                )
            }
            Err(error) => {
                super::debug_log::log(format!(
                    "failed to resolve app data directory for system-audio speech admission: {error}"
                ));
                return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
            }
        };
        match available {
            Ok(true) => {
                let provider = crate::audio_transcription_models::provider_id_for_settings(
                    settings.transcription.provider,
                );
                let mut payload = ::app_infra::AudioTranscriptionJobPayload::new(
                    provider,
                    settings.transcription.model_id.clone(),
                    settings.transcription.language.clone(),
                );
                payload.options =
                    crate::audio_transcription_models::transcription_request_options_for_settings(
                        &settings.transcription,
                    );
                let speaker_admission = speaker_analysis_admission_for_app_handle(app_handle);
                if speaker_admission.enabled && speaker_admission.provider_available {
                    if let Some(payload_json) = speaker_admission.payload_json.as_deref() {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) {
                            payload.options.insert(
                                ::app_infra::SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                                value,
                            );
                        }
                    }
                }
                serde_json::to_string(&payload).ok()
            }
            Ok(false) | Err(_) => None,
        }
    };

    let Some(transcription_payload) = transcription_admission else {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    };
    let payload = ::app_infra::SystemAudioSpeechActivityJobPayload {
        detector: settings.audio_speech_detection.detector,
        transcription_payload,
        speaker_analysis_payload: speaker_analysis_admission_for_app_handle(app_handle)
            .payload_json,
    };
    match serde_json::to_string(&payload) {
        Ok(payload_json) => {
            ::app_infra::SystemAudioSpeechActivityAdmission::available(payload_json)
        }
        Err(error) => {
            super::debug_log::log(format!(
                "failed to serialize system-audio speech admission payload: {error}"
            ));
            ::app_infra::SystemAudioSpeechActivityAdmission::unavailable()
        }
    }
}

#[cfg(target_os = "macos")]
fn speaker_analysis_admission_for_app_handle(
    app_handle: &tauri::AppHandle,
) -> ::app_infra::AudioSegmentSpeakerAnalysisAdmission {
    let speaker_settings = match app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.speaker_analysis.clone(),
        Err(_) => {
            super::debug_log::log(
                "failed to read recording settings for speaker analysis admission".to_string(),
            );
            return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
        }
    };
    if !speaker_settings.separate_speakers {
        return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
    }
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(app_data_dir) => app_data_dir,
        Err(error) => {
            super::debug_log::log(format!(
                "failed to resolve app data directory for speaker analysis admission: {error}"
            ));
            return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable();
        }
    };
    let models_dir = speaker_analysis::speaker_analysis_models_dir(&app_data_dir);
    let manifest = speaker_analysis::builtin_model_manifest();
    let Some(descriptor) = speaker_analysis::find_model_descriptor(
        &manifest,
        &speaker_settings.provider,
        speaker_settings.model_id.as_deref(),
    ) else {
        return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable();
    };
    match speaker_analysis::detect_model_status(&models_dir, descriptor) {
        Ok(status) if status.status == speaker_analysis::ModelStatusKind::Installed => {
            let mut payload = ::app_infra::SpeakerAnalysisJobPayload::new(
                speaker_settings.provider.clone(),
                speaker_settings.model_id.clone(),
            );
            payload.normalize_model_selection();
            payload.recognize_people = speaker_settings.recognize_saved_people;
            match serde_json::to_string(&payload) {
                Ok(payload_json) => {
                    ::app_infra::AudioSegmentSpeakerAnalysisAdmission::available(payload_json)
                }
                Err(error) => {
                    super::debug_log::log(format!(
                        "failed to serialize speaker analysis admission payload: {error}"
                    ));
                    ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable()
                }
            }
        }
        Ok(_) => ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable(),
        Err(error) => {
            super::debug_log::log(format!(
                "failed to inspect selected speaker analysis model: {error}"
            ));
            ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable()
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn persist_committed_audio_segments(
    app_handle: Option<&tauri::AppHandle>,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) {
    let (Some(app_handle), Some(source_sessions), Some(schedule), Some(output_files)) =
        (app_handle, source_sessions, schedule, output_files)
    else {
        return;
    };
    let segments = committed_audio_segments_for_output_files(
        Some(source_sessions),
        Some(schedule),
        segment_index,
        Some(output_files),
    );

    if segments.is_empty() {
        return;
    }

    let infra = Arc::clone(&*app_handle.state::<crate::app_infra::AppInfraState>());
    let app_handle = app_handle.clone();
    let persistence = run_native_capture_async("audio-segment-persistence", async move {
        let mut persisted_any = false;
        let transcription_admission = transcription_admission_for_app_handle(&app_handle);
        let speaker_admission = speaker_analysis_admission_for_app_handle(&app_handle);
        let system_audio_speech_admission =
            system_audio_speech_admission_for_app_handle(&app_handle);
        for segment in segments {
            match infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &transcription_admission,
                    &speaker_admission,
                    &system_audio_speech_admission,
                )
                .await
            {
                Ok(_) => {
                    persisted_any = true;
                }
                Err(error) => {
                    super::debug_log::log(format!(
                        "failed to persist native audio segment {}: {}",
                        segment.file_path, error
                    ));
                }
            }
        }

        if persisted_any {
            emit_audio_segments_changed(&app_handle);
        }
    });

    if let Err(error) = persistence {
        super::debug_log::log(format!("native audio segment persistence failed: {error}"));
    }
}

#[cfg(target_os = "macos")]
pub(super) fn warm_scrub_previews_for_committed_screen_outputs(
    app_handle: Option<&tauri::AppHandle>,
    output_files: Option<&CaptureOutputFiles>,
) {
    let (Some(app_handle), Some(output_files)) = (app_handle, output_files) else {
        return;
    };
    let screen_files = if output_files.screen_files.is_empty() {
        output_files.screen_file.iter().cloned().collect::<Vec<_>>()
    } else {
        output_files.screen_files.clone()
    };
    if screen_files.is_empty() {
        return;
    }

    match crate::app_infra::frame_preview::enqueue_scrub_preview_generation_for_screen_files(
        app_handle,
        &screen_files,
    ) {
        Ok(_) => {}
        Err(error) => {
            super::debug_log::log_warn(format!(
                "failed to enqueue scrub previews for finalized screen segment: {error}"
            ));
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) async fn close_frame_batches_for_stopped_screen_session_id_async(
    infra: &crate::app_infra::AppInfraState,
    session_id: &str,
) -> Result<(), CaptureErrorResponse> {
    let session_id = session_id.to_string();
    let infra = Arc::clone(infra);

    infra
        .close_and_schedule_all_frame_batches_for_session(&session_id)
        .await
        .map(|_| ())
        .map_err(|error| {
            super::debug_log::log(format!(
                "failed to close frame batches for stopped screen session {session_id}: {error}"
            ));
            CaptureErrorResponse {
                code: "frame_batch_close_failed".to_string(),
                message: format!(
                    "Failed to close frame batches for stopped screen session {session_id}: {error}"
                ),
            }
        })
}

#[cfg(target_os = "macos")]
pub(super) fn close_frame_batches_for_stopped_screen_session_id(
    infra: &crate::app_infra::AppInfraState,
    session_id: &str,
) -> Result<(), CaptureErrorResponse> {
    let infra = Arc::clone(infra);
    let session_id = session_id.to_string();

    match run_native_capture_async("frame-batch-close", async move {
        close_frame_batches_for_stopped_screen_session_id_async(&infra, &session_id).await
    }) {
        Ok(result) => result,
        Err(error) => Err(CaptureErrorResponse {
            code: "frame_batch_close_failed".to_string(),
            message: format!("Failed to close frame batches for stopped screen session: {error}"),
        }),
    }
}

#[cfg(target_os = "macos")]
fn close_frame_batches_for_stopped_screen_session(
    app_handle: Option<&tauri::AppHandle>,
    source_sessions: Option<&SourceSessions>,
) -> Result<(), CaptureErrorResponse> {
    let Some(screen_session) = source_sessions.and_then(|sessions| sessions.screen.as_ref()) else {
        return Ok(());
    };

    let Some(app_handle) = app_handle else {
        return Ok(());
    };

    let infra = Arc::clone(&*app_handle.state::<crate::app_infra::AppInfraState>());
    close_frame_batches_for_stopped_screen_session_id(&infra, &screen_session.session_id)
}

#[cfg(target_os = "macos")]
fn persist_committed_microphone_segments(
    app_handle: Option<&tauri::AppHandle>,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) {
    let Some(output_files) = output_files else {
        return;
    };
    let mut microphone_only = empty_output_files();
    microphone_only.microphone_file = output_files.microphone_file.clone();
    microphone_only.microphone_files = output_files.microphone_files.clone();
    persist_committed_audio_segments(
        app_handle,
        source_sessions,
        schedule,
        segment_index,
        Some(&microphone_only),
    );
}

#[cfg(target_os = "macos")]
fn persist_committed_system_audio_segments(
    app_handle: Option<&tauri::AppHandle>,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) {
    let Some(output_files) = output_files else {
        return;
    };
    let mut system_audio_only = empty_output_files();
    system_audio_only.system_audio_file = output_files.system_audio_file.clone();
    system_audio_only.system_audio_files = output_files.system_audio_files.clone();
    persist_committed_audio_segments(
        app_handle,
        source_sessions,
        schedule,
        segment_index,
        Some(&system_audio_only),
    );
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn pause_microphone_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_microphone_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(super) fn pause_microphone_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_microphone_paused() {
        return Ok(());
    }

    // Skip when microphone is not a requested source.
    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|s| s.microphone)
    {
        return Ok(());
    }

    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let microphone_tail_activity_mode = microphone_tail_trim_activity_mode_for_runtime(runtime);
    let mut finalized_microphone_outputs = None;
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let finalization = session.pause_output_file_for_inactivity_with_tail_activity_mode(
            runtime.inactivity.idle_timeout_seconds,
            runtime.inactivity.microphone_activity_threshold(),
            microphone_tail_activity_mode,
        )?;

        if let Some(output_files) = runtime.current_segment_output_files.as_ref() {
            let mut microphone_outputs = empty_output_files();
            microphone_outputs.microphone_file = output_files.microphone_file.clone();
            microphone_outputs.microphone_files = output_files.microphone_files.clone();
            apply_microphone_output_finalization(
                Some(&mut microphone_outputs),
                &finalization,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
            );
            finalize_capture_outputs(
                Some(&mut microphone_outputs),
                None,
                finalization
                    .output_file
                    .as_deref()
                    .or(microphone_recording_file.as_deref()),
                None,
                Some(&CaptureSources {
                    screen: false,
                    microphone: true,
                    system_audio: false,
                }),
            )?;
            finalized_microphone_outputs = Some(microphone_outputs);
        }
    }

    if let Some(finalized) = finalized_microphone_outputs.as_ref() {
        if let Some(committed) = runtime.output_files.as_mut() {
            append_committed_segment_output_files(committed, finalized);
        }
        persist_committed_microphone_segments(
            app_handle,
            runtime.source_sessions.as_ref(),
            runtime.segment_schedule.as_ref(),
            runtime.current_segment_index,
            Some(finalized),
        );
    }

    if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
        output_files.microphone_file = None;
        output_files.microphone_files.clear();
    }
    runtime.microphone_recording_file = None;

    runtime.inactivity.set_family_paused_states(
        runtime.inactivity.screen_paused,
        true,
        runtime.inactivity.system_audio_paused,
    );

    runtime.current_segment_sources = active_sources_for_runtime_pause_state(
        runtime,
        runtime.requested_sources.as_ref().unwrap(),
        runtime.inactivity.screen_paused,
        true, // microphone is now paused
        runtime.inactivity.system_audio_paused,
    );

    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn process_inactivity_audio_transitions_for_snapshot(
    runtime: &mut NativeCaptureRuntime,
    now: u64,
    activity_snapshot: super::inactivity::ActivitySnapshot,
) -> Result<(), CaptureErrorResponse> {
    if runtime
        .inactivity
        .should_resume_microphone_from_inactivity(now, activity_snapshot)
    {
        resume_microphone_from_inactivity(runtime)?;
    }

    if runtime
        .inactivity
        .should_pause_microphone_for_inactivity(now, activity_snapshot)
    {
        pause_microphone_for_inactivity_with_app_handle(runtime, None)?;
    }

    if runtime
        .inactivity
        .should_resume_system_audio_from_inactivity(now, activity_snapshot)
    {
        resume_system_audio_from_inactivity(runtime)?;
    }

    if runtime
        .inactivity
        .should_pause_system_audio_for_inactivity(now, activity_snapshot)
    {
        pause_system_audio_for_inactivity(runtime)?;
    }

    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn pause_system_audio_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_system_audio_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(super) fn pause_system_audio_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_system_audio_paused() {
        return Ok(());
    }

    // Skip when system audio is not a requested source.
    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|s| s.system_audio)
    {
        return Ok(());
    }

    // The writer must be detached whenever the backing stream is alive — even
    // while the screen family is soft-paused (the ScreenCaptureKit stream stays
    // live through a screen soft-pause and keeps feeding the writer). Gating
    // this on screen-pause state used to leave the writer attached and
    // recording while the family was marked paused.
    {
        if capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
            let system_audio_recording_file = runtime.system_audio_recording_file.clone();
            // Soft-pause: tell the screen backend to finalize and detach its
            // system-audio writer without stopping/restarting the screen session.
            capture_screen::pause_system_audio_writer_for_inactivity(
                &mut runtime.active_screen_session,
                runtime.inactivity.idle_timeout_seconds,
            )?;

            let mut finalized_system_audio_outputs = None;
            if let Some(output_files) = runtime.current_segment_output_files.as_ref() {
                let mut system_audio_outputs = empty_output_files();
                system_audio_outputs.system_audio_file = output_files.system_audio_file.clone();
                system_audio_outputs.system_audio_files = output_files.system_audio_files.clone();
                finalize_capture_outputs(
                    Some(&mut system_audio_outputs),
                    None,
                    None,
                    system_audio_recording_file.as_deref(),
                    Some(&CaptureSources {
                        screen: false,
                        microphone: false,
                        system_audio: true,
                    }),
                )?;
                finalized_system_audio_outputs = Some(system_audio_outputs);
            }

            if let Some(finalized) = finalized_system_audio_outputs.as_ref() {
                if let Some(committed) = runtime.output_files.as_mut() {
                    append_committed_segment_output_files(committed, finalized);
                }
                persist_committed_system_audio_segments(
                    app_handle,
                    runtime.source_sessions.as_ref(),
                    runtime.segment_schedule.as_ref(),
                    runtime.current_segment_index,
                    Some(finalized),
                );
            }

            // The finished system-audio file has already been appended to the
            // committed output list and persisted. Remove it from the live
            // segment bookkeeping so a later screen pause/rotation/stop cannot
            // upsert the same audio path again with the later segment clock.
            if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                output_files.system_audio_file = None;
                output_files.system_audio_files.clear();
            }
            runtime.system_audio_recording_file = None;
        } else {
            // No active screen session backend (e.g. tests/headless) — just
            // reconcile bookkeeping without attempting a pause that would
            // fail in the native capture stack.
            if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                output_files.system_audio_file = None;
                output_files.system_audio_files.clear();
            }
            runtime.system_audio_recording_file = None;
        }
    }

    runtime.inactivity.set_family_paused_states(
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        true,
    );

    runtime.current_segment_sources = active_sources_for_runtime_pause_state(
        runtime,
        runtime.requested_sources.as_ref().unwrap(),
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        true, // system audio is now paused
    );

    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn resume_microphone_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_microphone_paused() {
        return Ok(());
    }

    let Some(sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming microphone from inactivity"
                .to_string(),
        });
    };

    // Skip when microphone is not a requested source.
    if !sources.microphone {
        return Ok(());
    }

    refresh_runtime_planner_dates(runtime);
    ensure_microphone_planner_for_runtime(runtime, "resuming microphone from inactivity")?;

    if sources.microphone && runtime.microphone_planner.is_some() {
        let microphone_recording_file =
            super::microphone::next_microphone_output_file_for_runtime(runtime)?;
        let microphone_tail_activity_mode = microphone_tail_trim_activity_mode_for_runtime(runtime);

        if let Some(session) = runtime.active_microphone_session.as_mut() {
            session.resume_output_file_with_inactivity_tail_trim_activity_mode(
                &microphone_recording_file,
                runtime.inactivity.idle_timeout_seconds,
                runtime.inactivity.microphone_activity_threshold(),
                microphone_tail_activity_mode,
            )?;
            runtime.microphone_recording_file = Some(microphone_recording_file.clone());
            if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                super::output::set_current_microphone_output_file(
                    output_files,
                    microphone_recording_file,
                );
            }
        } else {
            let mic_start =
                microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id_and_inactivity_tail_trim_activity_mode(
                    &microphone_recording_file,
                    runtime.microphone_device_id_for_capture.as_deref(),
                    runtime.inactivity.idle_timeout_seconds,
                    runtime.inactivity.microphone_activity_threshold(),
                    microphone_tail_activity_mode,
                );

            match mic_start {
                Ok(session) => {
                    runtime.active_microphone_session = Some(session);
                    runtime.microphone_recording_file = Some(microphone_recording_file.clone());
                    if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                        super::output::set_current_microphone_output_file(
                            output_files,
                            microphone_recording_file,
                        );
                    }
                }
                Err(error) => {
                    return Err(error);
                }
            }
        }
    }

    runtime.inactivity.set_family_paused_states(
        runtime.inactivity.screen_paused,
        false,
        runtime.inactivity.system_audio_paused,
    );

    runtime.current_segment_sources = active_sources_for_runtime_pause_state(
        runtime,
        &sources,
        runtime.inactivity.screen_paused,
        false, // microphone is now resumed
        runtime.inactivity.system_audio_paused,
    );

    Ok(())
}

/// How a system-audio inactivity resume must act given the screen backend
/// state. The wrong arm here is what silently drops system audio: marking the
/// family resumed without attaching a writer leaves
/// `system_audio_recording_file` as `None`, so the screen soft-resume path
/// skips the writer and nothing records until the next rotation replans it.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SystemAudioResumeAction {
    /// Screen session live and the screen family active: attach a fresh writer.
    ResumeWriter,
    /// Cannot attach a writer right now (screen family soft-paused on a live
    /// stream, or the session is gone outside a screen pause). Keep the family
    /// marked paused so the tick after conditions clear re-fires this resume
    /// and actually attaches the writer.
    DeferKeepPaused,
    /// Cold screen pause (session torn down): no writer can exist yet, but the
    /// cold screen-resume path honors the unpaused flag and recreates the
    /// writer with the new session, so flipping the flag now is correct.
    MarkResumedOnly,
}

#[cfg(target_os = "macos")]
pub(super) fn system_audio_resume_action(
    session_live: bool,
    screen_paused: bool,
) -> SystemAudioResumeAction {
    match (session_live, screen_paused) {
        (true, false) => SystemAudioResumeAction::ResumeWriter,
        (false, true) => SystemAudioResumeAction::MarkResumedOnly,
        (true, true) | (false, false) => SystemAudioResumeAction::DeferKeepPaused,
    }
}

#[cfg(target_os = "macos")]
pub(super) fn resume_system_audio_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_system_audio_paused() {
        return Ok(());
    }

    let Some(sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming system audio from inactivity"
                .to_string(),
        });
    };

    // Skip when system audio is not a requested source.
    if !sources.system_audio {
        return Ok(());
    }

    // If system audio was soft-paused while the screen session is still live,
    // allocate a fresh output path and resume the writer in-place.
    if sources.system_audio && sources.screen {
        match system_audio_resume_action(
            capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()),
            runtime.inactivity.is_screen_paused(),
        ) {
            SystemAudioResumeAction::DeferKeepPaused => return Ok(()),
            SystemAudioResumeAction::MarkResumedOnly => {}
            SystemAudioResumeAction::ResumeWriter => {
                refresh_runtime_planner_dates(runtime);
                // Always try to seed the planner for real writer resumes so future
                // resumes/rotations preserve the dedicated system-audio session.
                let system_audio_planner = ensure_system_audio_planner_for_runtime(
                    runtime,
                    "resuming system audio from inactivity",
                )?;

                let planner = system_audio_planner.ok_or_else(|| CaptureErrorResponse {
                    code: "invalid_runtime_state".to_string(),
                    message: "Capture system-audio planner missing while resuming system audio"
                        .to_string(),
                })?;
                let audio_dir = planner.audio_dir();
                std::fs::create_dir_all(&audio_dir).map_err(|error| CaptureErrorResponse {
                    code: "io_error".to_string(),
                    message: format!("Failed to create capture audio directory: {error}"),
                })?;
                let new_system_audio_file = planner
                    .system_audio_resume_file(
                        runtime.current_segment_index,
                        super::runtime::now_unix_ms(),
                    )
                    .to_string_lossy()
                    .to_string();

                capture_screen::resume_system_audio_writer(
                    &mut runtime.active_screen_session,
                    &new_system_audio_file,
                )?;

                runtime.system_audio_recording_file = Some(new_system_audio_file.clone());
                if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                    set_current_system_audio_output_file(output_files, new_system_audio_file);
                }
            }
        }
    }

    runtime.inactivity.set_family_paused_states(
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        false,
    );

    runtime.current_segment_sources = active_sources_for_runtime_pause_state(
        runtime,
        &sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        false, // system audio is now resumed
    );

    Ok(())
}

#[cfg(target_os = "macos")]
#[cfg(test)]
pub(super) fn pause_screen_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_screen_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(super) fn pause_screen_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_screen_paused() {
        return Ok(());
    }

    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.screen)
    {
        return Ok(());
    }

    let mut current_segment_output_files = runtime.current_segment_output_files.clone();
    let recording_file = runtime.recording_file.clone();
    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let system_audio_recording_file = runtime.system_audio_recording_file.clone();
    let requested_sources = runtime.requested_sources.clone();
    let mut segment_committed = false;

    if capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        capture_screen::pause_screen_outputs_for_inactivity(&mut runtime.active_screen_session)?;

        if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
            flush_frame_artifacts(tx);
        }

        if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
            output_files.screen_file = None;
        }
        runtime.recording_file = None;

        runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
            active_sources_for_runtime_pause_state(
                runtime,
                sources,
                true,
                runtime.inactivity.microphone_paused,
                runtime.inactivity.system_audio_paused,
            )
        });

        mark_screen_paused_for_inactivity(runtime);

        return Ok(());
    }

    let screen_finalize_recovered =
        match capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut runtime.active_screen_session,
            inactivity_tail_trim_seconds: runtime.inactivity.idle_timeout_seconds,
        }) {
            Ok(()) => false,
            Err(error)
                if recover_from_segment_finalize_error(
                    "pausing screen for inactivity",
                    &error,
                    current_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                ) =>
            {
                true
            }
            Err(error) => {
                // The screen session backend is already stopped; reconcile
                // bookkeeping to match the actual backend state before
                // propagating the error.
                runtime.recording_file = None;
                runtime.system_audio_recording_file = None;
                runtime.current_segment_output_files = None;
                runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
                    active_sources_for_runtime_pause_state(
                        runtime,
                        sources,
                        true, // screen is now stopped
                        runtime.inactivity.microphone_paused,
                        runtime.inactivity.system_audio_paused,
                    )
                });
                mark_screen_paused_for_inactivity(runtime);
                return Err(error);
            }
        };

    if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
        flush_frame_artifacts(tx);
    }

    if !screen_finalize_recovered {
        match finalize_capture_outputs(
            current_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        ) {
            Ok(()) => segment_committed = true,
            Err(error)
                if recover_from_segment_finalize_error(
                    "pausing screen for inactivity",
                    &error,
                    current_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                ) => {}
            Err(error) => {
                // Finalization failed fatally; reconcile bookkeeping to
                // match the already-stopped backend state.
                runtime.recording_file = None;
                runtime.system_audio_recording_file = None;
                runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
                    active_sources_for_runtime_pause_state(
                        runtime,
                        sources,
                        true, // screen is now stopped
                        runtime.inactivity.microphone_paused,
                        runtime.inactivity.system_audio_paused,
                    )
                });
                // If audio is still live, preserve continuation bookkeeping
                // so the ongoing microphone capture remains trackable.
                let has_live_microphone = !runtime.inactivity.microphone_paused
                    && (runtime.active_microphone_session.is_some()
                        || runtime.microphone_recording_file.is_some());
                if has_live_microphone {
                    let mut audio_continuation = empty_output_files();
                    if let Some(mic_file) = runtime.microphone_recording_file.as_ref() {
                        set_current_microphone_output_file(
                            &mut audio_continuation,
                            mic_file.clone(),
                        );
                    }
                    runtime.current_segment_output_files = Some(audio_continuation);
                } else {
                    runtime.current_segment_output_files = None;
                }
                mark_screen_paused_for_inactivity(runtime);
                return Err(error);
            }
        }
    }

    if segment_committed {
        if let (Some(committed), Some(segment)) = (
            runtime.output_files.as_mut(),
            current_segment_output_files.as_ref(),
        ) {
            append_committed_segment_output_files(committed, segment);
        }
        persist_committed_audio_segments(
            app_handle,
            runtime.source_sessions.as_ref(),
            runtime.segment_schedule.as_ref(),
            runtime.current_segment_index,
            current_segment_output_files.as_ref(),
        );
        warm_scrub_previews_for_committed_screen_outputs(
            app_handle,
            current_segment_output_files.as_ref(),
        );
    }

    runtime.recording_file = None;
    runtime.system_audio_recording_file = None;

    // Recompute current_segment_sources: if audio is still active, the
    // audio-only subset becomes the active set; otherwise clear it.
    runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
        active_sources_for_runtime_pause_state(
            runtime,
            sources,
            true, // screen is now paused
            runtime.inactivity.microphone_paused,
            runtime.inactivity.system_audio_paused,
        )
    });

    // If audio is still active (not paused), preserve current-segment
    // bookkeeping so that the ongoing audio-only continuation remains
    // trackable by stop/rotation/finalization paths.  Create a fresh
    // output-files struct that carries only the live microphone file.
    //
    // Only do this when there is a real live microphone continuation
    // (active session or output file), not just requested-source intent.
    // System-audio-only does not qualify because it is captured through
    // the screen session which is now stopped.
    let has_live_microphone = !runtime.inactivity.microphone_paused
        && (runtime.active_microphone_session.is_some()
            || runtime.microphone_recording_file.is_some());
    if has_live_microphone {
        let mut audio_continuation = empty_output_files();
        if let Some(mic_file) = runtime.microphone_recording_file.as_ref() {
            set_current_microphone_output_file(&mut audio_continuation, mic_file.clone());
        }
        runtime.current_segment_output_files = Some(audio_continuation);
    } else {
        runtime.current_segment_output_files = None;
    }

    mark_screen_paused_for_inactivity(runtime);

    Ok(())
}

#[cfg(target_os = "macos")]
fn mark_screen_paused_for_inactivity(runtime: &mut NativeCaptureRuntime) {
    runtime.inactivity.set_family_paused_states(
        true,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime
        .inactivity
        .mark_screen_pause_started(now_monotonic_marker_ms());
}

#[cfg(target_os = "macos")]
/// Whether an activity-triggered screen resume should wait instead of starting
/// capture: with the ScreenCaptureKit stream torn down and no drawable display
/// (lid closed, display asleep or unplugged), a cold start is doomed to fail
/// with `capture_display_unavailable`. Deferring keeps the screen paused so the
/// next tick re-evaluates — the same wait-quietly stance display-unavailable
/// suspension recovery takes (ADR 0021), which this path previously lacked,
/// letting dark-wake activity blips hammer ScreenCaptureKit multiple times per
/// second.
pub(super) fn should_defer_screen_resume_for_missing_display(
    screen_stream_live: bool,
    display_available: bool,
) -> bool {
    !screen_stream_live && !display_available
}

#[cfg(target_os = "macos")]
pub(super) fn resume_screen_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if should_defer_screen_resume_for_missing_display(
        capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()),
        capture_screen::screen_display_available(),
    ) {
        return Ok(());
    }
    let tail_trim_seconds = runtime.inactivity.idle_timeout_seconds;
    let microphone_activity_threshold = runtime.inactivity.microphone_activity_threshold();
    let microphone_tail_activity_mode = microphone_tail_trim_activity_mode_for_runtime(runtime);
    let metadata_snapshot_provider = app_handle.map(metadata::frame_metadata_snapshot_provider);
    let initial_privacy_filter = app_handle.map(privacy::collect_initial_privacy_filter);
    resume_screen_from_inactivity_with_start_segment(
        runtime,
        app_handle,
        move |segment_dir,
              screen_output_file,
              system_audio_output_path,
              sources,
              screen_frame_rate,
              screen_resolution,
              effective_screen_bitrate_bps,
              microphone_device_id,
              frame_artifact_tx,
              microphone_output_path| {
            let started_segment = start_segment_with_inactivity_tail_trim_seconds(
                segment_dir,
                screen_output_file,
                system_audio_output_path,
                sources,
                screen_frame_rate,
                screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id,
                frame_artifact_tx,
                metadata_snapshot_provider,
                microphone_output_path,
                tail_trim_seconds,
                microphone_activity_threshold,
                microphone_tail_activity_mode,
                initial_privacy_filter
                    .as_ref()
                    .and_then(|initial| initial.screen_capture_filter()),
            )?;
            if let (Some(app_handle), Some(initial)) = (app_handle, initial_privacy_filter) {
                initial.mark_applied(app_handle);
                if let Some(settings) = app_handle
                    .try_state::<crate::native_capture::RecordingSettingsState>()
                    .map(|state| {
                        state
                            .lock()
                            .expect("recording settings state poisoned")
                            .settings
                            .clone()
                    })
                {
                    privacy::record_initial_privacy_filter_outcome(
                        app_handle,
                        &settings,
                        started_segment.6,
                    );
                }
            }
            Ok((
                started_segment.0,
                started_segment.1,
                started_segment.2,
                started_segment.3,
                started_segment.4,
                started_segment.5,
            ))
        },
    )
}

#[cfg(target_os = "macos")]
pub(super) fn resume_screen_from_inactivity_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
    start_segment_fn: F,
) -> Result<(), CaptureErrorResponse>
where
    F: FnOnce(
        &Path,
        Option<&Path>,
        Option<&Path>,
        &CaptureSources,
        f64,
        &capture_types::ScreenResolution,
        Option<u32>,
        Option<&str>,
        Option<mpsc::Sender<FrameArtifactMessage>>,
        Option<&Path>,
    ) -> Result<StartedSegmentState, CaptureErrorResponse>,
{
    if !runtime.inactivity.is_screen_paused() {
        return Ok(());
    }

    refresh_runtime_planner_dates(runtime);

    let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture screen planner missing while resuming screen from inactivity"
                .to_string(),
        });
    };
    let Some(sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming screen from inactivity".to_string(),
        });
    };
    let Some(schedule) = runtime.segment_schedule.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture schedule missing while resuming screen from inactivity".to_string(),
        });
    };
    let Some(clock) = runtime.capture_clock.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture clock missing while resuming screen from inactivity".to_string(),
        });
    };
    let _ = schedule;
    let _ = clock;

    if capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        if let Some(app_handle) = app_handle {
            // This is the synchronous inactivity-resume path: it runs while the
            // `NativeCaptureState` mutex is held (segment loop -> tick_inactivity).
            // Use the cached browser URL so the bounded-but-slow Gecko AX read
            // cannot stall the held lock; the next off-lock metadata tick will
            // refresh the live URL.
            let (_, privacy_filter_update) = privacy::collect_privacy_filter_update(
                app_handle,
                privacy::PrivacyRefreshReason::FallbackPoll,
                crate::native_capture::metadata::BrowserUrlReadMode::Cached,
            );
            let _ =
                privacy::apply_privacy_filter_update(app_handle, runtime, privacy_filter_update)?;
        }

        let next_index = next_emitted_segment_index(runtime.current_segment_index);
        let segment_dir = screen_planner.segment_dir(next_index);
        let screen_output_file = screen_planner.segment_screen_output(next_index);
        let mut previous_microphone_outputs = audio_only_output_files(
            runtime.current_segment_output_files.as_ref(),
            runtime.active_microphone_session.is_some()
                && !runtime.inactivity.is_microphone_paused(),
            false,
        );
        let previous_system_audio_outputs = audio_only_output_files(
            runtime.current_segment_output_files.as_ref(),
            false,
            runtime.system_audio_recording_file.is_some()
                && !runtime.inactivity.is_system_audio_paused(),
        );

        capture_screen::resume_screen_outputs(
            &mut runtime.active_screen_session,
            &segment_dir,
            screen_output_file.to_string_lossy().as_ref(),
        )?;

        let next_system_audio_recording_file = next_reanchored_system_audio_output_file(
            runtime,
            next_index,
            "resuming screen outputs from inactivity",
        )?;
        if let Some(system_audio_output_file) = next_system_audio_recording_file.as_deref() {
            capture_screen::pause_system_audio_writer_for_inactivity(
                &mut runtime.active_screen_session,
                0,
            )?;
            capture_screen::resume_system_audio_writer(
                &mut runtime.active_screen_session,
                system_audio_output_file,
            )?;
            append_and_persist_committed_audio_outputs(
                runtime,
                app_handle,
                previous_system_audio_outputs.as_ref(),
            );
        }

        let next_microphone_recording_file = next_reanchored_microphone_output_file(
            runtime,
            next_index,
            "resuming screen outputs from inactivity",
        )?;
        if let Some(microphone_output_file) = next_microphone_recording_file.as_deref() {
            if let Some(session) = runtime.active_microphone_session.as_mut() {
                let mic_finalization =
                    session.rotate_output_file_returning_finalization(microphone_output_file)?;
                apply_microphone_output_finalization(
                    previous_microphone_outputs.as_mut(),
                    &mic_finalization,
                    runtime.source_sessions.as_ref(),
                    runtime.segment_schedule.as_ref(),
                    runtime.current_segment_index,
                );
                append_and_persist_committed_audio_outputs(
                    runtime,
                    app_handle,
                    previous_microphone_outputs.as_ref(),
                );
            }
        }

        let mut segment_outputs = empty_output_files();
        set_current_screen_output_file(
            &mut segment_outputs,
            screen_output_file.to_string_lossy().to_string(),
        );
        if let Some(microphone_output_file) = next_microphone_recording_file.as_ref() {
            set_current_microphone_output_file(
                &mut segment_outputs,
                microphone_output_file.clone(),
            );
        } else if !runtime.inactivity.is_microphone_paused() {
            if let Some(microphone_output_file) = runtime.microphone_recording_file.as_ref() {
                set_current_microphone_output_file(
                    &mut segment_outputs,
                    microphone_output_file.clone(),
                );
            }
        }
        if let Some(system_audio_output_file) = next_system_audio_recording_file.as_ref() {
            set_current_system_audio_output_file(
                &mut segment_outputs,
                system_audio_output_file.clone(),
            );
        } else if !runtime.inactivity.is_system_audio_paused() {
            if let Some(system_audio_output_file) = runtime.system_audio_recording_file.as_ref() {
                set_current_system_audio_output_file(
                    &mut segment_outputs,
                    system_audio_output_file.clone(),
                );
            }
        }

        runtime.current_segment_index = next_index;
        runtime.current_segment_output_files = Some(segment_outputs);
        runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
            &sources,
            false,
            runtime.inactivity.microphone_paused,
            runtime.inactivity.system_audio_paused,
        );
        runtime.recording_file = Some(screen_output_file.to_string_lossy().to_string());
        if next_microphone_recording_file.is_some() {
            runtime.microphone_recording_file = next_microphone_recording_file;
        }
        if next_system_audio_recording_file.is_some() {
            runtime.system_audio_recording_file = next_system_audio_recording_file;
        }
        reanchor_active_segment_timing(runtime, "resuming screen outputs from inactivity")?;
        runtime.inactivity.set_family_paused_states(
            false,
            runtime.inactivity.microphone_paused,
            runtime.inactivity.system_audio_paused,
        );

        return Ok(());
    }

    // Start only screen-family sources; microphone sessions remain untouched.
    // Keep the ScreenCaptureKit audio stream attached for requested system audio
    // even when the writer is paused; otherwise there is no activity signal to
    // trigger system-audio resume.
    let screen_only_sources = CaptureSources {
        screen: sources.screen,
        microphone: false,
        system_audio: sources.system_audio,
    };

    let system_audio_writer_paused = runtime.inactivity.is_system_audio_paused();
    let system_audio_planner = if screen_only_sources.system_audio && !system_audio_writer_paused {
        ensure_system_audio_planner_for_runtime(runtime, "resuming screen from inactivity")?
    } else {
        None
    };

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);
    let system_audio_output_path = (screen_only_sources.system_audio
        && !system_audio_writer_paused)
        .then(|| {
            system_audio_planner
                .as_ref()
                .map(|planner| planner.system_audio_file(next_index))
        })
        .flatten();

    let started_segment = start_segment_fn(
        &segment_dir,
        Some(&screen_output_file),
        system_audio_output_path.as_deref(),
        &screen_only_sources,
        runtime.screen_frame_rate,
        &runtime.screen_resolution,
        runtime.effective_screen_bitrate_bps,
        None, // no microphone restart
        runtime.frame_artifact_tx.clone(),
        None, // no microphone output path when screen-only resume
    )?;

    let mut previous_microphone_outputs = audio_only_output_files(
        runtime.current_segment_output_files.as_ref(),
        runtime.active_microphone_session.is_some() && !runtime.inactivity.is_microphone_paused(),
        false,
    );

    let (
        mut segment_outputs,
        recording_file,
        _microphone_recording_file,
        system_audio_recording_file,
        mut active_screen_session,
        _active_microphone_session,
    ) = started_segment;

    let next_microphone_recording_file = next_reanchored_microphone_output_file(
        runtime,
        next_index,
        "resuming screen from inactivity",
    )?;
    if let Some(microphone_output_file) = next_microphone_recording_file.as_deref() {
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            let mic_finalization = match session
                .rotate_output_file_returning_finalization(microphone_output_file)
            {
                Ok(finalization) => finalization,
                Err(error) => {
                    let _ =
                        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                            active_session: &mut active_screen_session,
                            inactivity_tail_trim_seconds: 0,
                        });
                    cleanup_unusable_segment_artifacts(
                        Some(&segment_outputs),
                        recording_file.as_deref(),
                        None,
                        system_audio_recording_file.as_deref(),
                    );
                    return Err(error);
                }
            };
            apply_microphone_output_finalization(
                previous_microphone_outputs.as_mut(),
                &mic_finalization,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
            );
            append_and_persist_committed_audio_outputs(
                runtime,
                app_handle,
                previous_microphone_outputs.as_ref(),
            );
            set_current_microphone_output_file(
                &mut segment_outputs,
                microphone_output_file.to_string(),
            );
        }
    } else {
        merge_live_microphone_continuation_into_segment_outputs(runtime, &mut segment_outputs);
    }

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    // Reflect the actual active source subset, not merely requested sources.
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        &sources,
        false, // screen is being resumed
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.recording_file = recording_file;
    if next_microphone_recording_file.is_some() {
        runtime.microphone_recording_file = next_microphone_recording_file;
    }
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    reanchor_active_segment_timing(runtime, "resuming screen from inactivity")?;

    runtime.inactivity.set_family_paused_states(
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );

    Ok(())
}

#[cfg(target_os = "macos")]
#[cfg(test)]
pub(super) fn pause_runtime_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_runtime_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(super) fn pause_runtime_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_paused {
        return Ok(());
    }

    let requested_sources = runtime.requested_sources.clone();
    if runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.microphone)
    {
        pause_microphone_for_inactivity_with_app_handle(runtime, app_handle)?;
    }

    if runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
    {
        pause_system_audio_for_inactivity_with_app_handle(runtime, app_handle)?;
    }

    if runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.screen)
    {
        pause_screen_for_inactivity_with_app_handle(runtime, app_handle)?;
    }

    runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
        active_sources_for_runtime_pause_state(runtime, sources, true, true, sources.system_audio)
    });
    runtime.inactivity.is_paused = true;

    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) type StartedSegmentState = (
    CaptureOutputFiles,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<capture_screen::ActiveCaptureSession>,
    Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
);

#[cfg(target_os = "macos")]
fn refresh_current_segment_sources_for_pause_state(
    runtime: &mut NativeCaptureRuntime,
    requested_sources: &CaptureSources,
) {
    runtime.current_segment_sources = active_sources_for_runtime_pause_state(
        runtime,
        requested_sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
}

#[cfg(target_os = "macos")]
fn preserve_live_microphone_continuation_outputs(runtime: &mut NativeCaptureRuntime) {
    let has_live_microphone = !runtime.inactivity.microphone_paused
        && (runtime.active_microphone_session.is_some()
            || runtime.microphone_recording_file.is_some());

    if has_live_microphone {
        let mut audio_continuation = empty_output_files();
        if let Some(mic_file) = runtime.microphone_recording_file.as_ref() {
            set_current_microphone_output_file(&mut audio_continuation, mic_file.clone());
        }
        runtime.current_segment_output_files = Some(audio_continuation);
    } else {
        runtime.current_segment_output_files = None;
    }
}

#[cfg(target_os = "macos")]
fn merge_live_microphone_continuation_into_segment_outputs(
    runtime: &NativeCaptureRuntime,
    segment_outputs: &mut CaptureOutputFiles,
) {
    let has_live_microphone = !runtime.inactivity.microphone_paused
        && (runtime.active_microphone_session.is_some()
            || runtime.microphone_recording_file.is_some());

    if has_live_microphone {
        if let Some(mic_file) = runtime.microphone_recording_file.as_ref() {
            set_current_microphone_output_file(segment_outputs, mic_file.clone());
        }
    }
}

#[cfg(target_os = "macos")]
fn current_screen_output_file(output_files: Option<&CaptureOutputFiles>) -> Option<&str> {
    let output_files = output_files?;

    output_files
        .screen_file
        .as_deref()
        .or_else(|| output_files.screen_files.last().map(String::as_str))
}

#[cfg(target_os = "macos")]
fn current_system_audio_output_file(output_files: Option<&CaptureOutputFiles>) -> Option<&str> {
    let output_files = output_files?;

    output_files
        .system_audio_file
        .as_deref()
        .or_else(|| output_files.system_audio_files.last().map(String::as_str))
}

#[cfg(target_os = "macos")]
pub(super) fn recover_screen_capture_after_wake_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
    start_segment_fn: F,
) -> Result<bool, CaptureErrorResponse>
where
    F: FnOnce(
        &Path,
        Option<&Path>,
        Option<&Path>,
        &CaptureSources,
        f64,
        &capture_types::ScreenResolution,
        Option<u32>,
        Option<&str>,
        Option<mpsc::Sender<FrameArtifactMessage>>,
        Option<&Path>,
    ) -> Result<StartedSegmentState, CaptureErrorResponse>,
{
    if !runtime.is_running {
        return Ok(false);
    }

    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Ok(false);
    };

    if !requested_sources.screen {
        return Ok(false);
    }

    // Never fight a deliberate suspension or manual pause. A DisplayUnavailable /
    // LowDisk / privacy suspension owns the screen state and re-arms through its
    // own path (`attempt_privacy_suspension_recovery`), and a user pause resumes
    // explicitly. The system-wake callbacks (`NSWorkspaceDidWake` and the Core
    // Graphics display-reconfiguration callback) fire on the same display events
    // that surface those suspensions, so recovering here would race the owner:
    // restart a segment the owner immediately re-tears-down, or leave a live
    // session with `capture_suspension` still set. Defer to the owner. This
    // mirrors the guard in `RecordingLifecycle::should_attempt_recovery_after_possible_wake`,
    // which only covered the frontend permission-poll path, not these callbacks.
    if runtime.capture_suspension.is_some() || runtime.user_capture_paused {
        return Ok(false);
    }

    // A display-reconfiguration callback fires for *any* display change
    // (resolution/SetMode, monitor connect/Add, set-main), not only a wake, and
    // `NSWorkspaceDidWake` fires alongside it — so this path can be entered while
    // the screen is still capturing normally. Recovering an already-live session
    // would tear it down and rotate the segment (a spurious mid-recording boundary
    // and a brief capture gap), and a second wake callback would re-rotate the
    // segment the first just started. If the screen session is already live there
    // is nothing to recover — this is the `!session_is_live` idempotency the wake
    // callbacks document but never enforced on this path (the legitimate
    // post-sleep path clears `active_screen_session`, so it still proceeds).
    if capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        return Ok(false);
    }

    let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture screen planner missing while recovering after system wake"
                .to_string(),
        });
    };
    let Some(_schedule) = runtime.segment_schedule.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture schedule missing while recovering after system wake".to_string(),
        });
    };
    let Some(_clock) = runtime.capture_clock.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture clock missing while recovering after system wake".to_string(),
        });
    };

    let system_audio_writer_paused = runtime.inactivity.is_system_audio_paused();
    let screen_sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: requested_sources.system_audio,
    };
    let system_audio_planner = if screen_sources.system_audio && !system_audio_writer_paused {
        ensure_system_audio_planner_for_runtime(runtime, "recovering after system wake")?
    } else {
        None
    };

    let mut previous_screen_outputs =
        runtime
            .current_segment_output_files
            .clone()
            .map(|mut outputs| {
                outputs.microphone_file = None;
                outputs.microphone_files.clear();
                outputs
            });
    let recording_file = runtime.recording_file.clone().or_else(|| {
        current_screen_output_file(previous_screen_outputs.as_ref()).map(str::to_owned)
    });
    let system_audio_recording_file = runtime.system_audio_recording_file.clone().or_else(|| {
        current_system_audio_output_file(previous_screen_outputs.as_ref()).map(str::to_owned)
    });

    if let Err(error) = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
        inactivity_tail_trim_seconds: 0,
    }) {
        if capture_screen::should_preserve_runtime_on_stop_error(&error) {
            return Err(error);
        }
    }

    if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
        flush_frame_artifacts(tx);
    }

    let previous_segment_committed = match finalize_capture_outputs(
        previous_screen_outputs.as_mut(),
        recording_file.as_deref(),
        None,
        system_audio_recording_file.as_deref(),
        Some(&screen_sources),
    ) {
        Ok(()) => true,
        Err(error)
            if recover_from_segment_finalize_error(
                "recovering after system wake",
                &error,
                previous_screen_outputs.as_ref(),
                recording_file.as_deref(),
                None,
                system_audio_recording_file.as_deref(),
            ) =>
        {
            false
        }
        Err(error) => {
            cleanup_unusable_segment_artifacts(
                previous_screen_outputs.as_ref(),
                recording_file.as_deref(),
                None,
                system_audio_recording_file.as_deref(),
            );
            super::debug_log::log(format!(
                "failed to finalize stale screen capture outputs while recovering after system wake: [{}] {}",
                error.code, error.message
            ));
            false
        }
    };

    if previous_segment_committed {
        if let (Some(committed), Some(segment)) = (
            runtime.output_files.as_mut(),
            previous_screen_outputs.as_ref(),
        ) {
            append_committed_segment_output_files(committed, segment);
        }
        persist_committed_system_audio_segments(
            app_handle,
            runtime.source_sessions.as_ref(),
            runtime.segment_schedule.as_ref(),
            runtime.current_segment_index,
            previous_screen_outputs.as_ref(),
        );
        warm_scrub_previews_for_committed_screen_outputs(
            app_handle,
            previous_screen_outputs.as_ref(),
        );
    }

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);
    let system_audio_output_path = (screen_sources.system_audio && !system_audio_writer_paused)
        .then(|| {
            system_audio_planner
                .as_ref()
                .map(|planner| planner.system_audio_file(next_index))
        })
        .flatten();

    let started_segment = match start_segment_fn(
        &segment_dir,
        Some(&screen_output_file),
        system_audio_output_path.as_deref(),
        &screen_sources,
        runtime.screen_frame_rate,
        &runtime.screen_resolution,
        runtime.effective_screen_bitrate_bps,
        None,
        runtime.frame_artifact_tx.clone(),
        None,
    ) {
        Ok(started_segment) => started_segment,
        Err(error) => {
            runtime.recording_file = None;
            runtime.system_audio_recording_file = None;
            runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
                &requested_sources,
                true,
                runtime.inactivity.microphone_paused,
                runtime.inactivity.system_audio_paused,
            );
            preserve_live_microphone_continuation_outputs(runtime);
            return Err(error);
        }
    };

    let mut previous_microphone_outputs = audio_only_output_files(
        runtime.current_segment_output_files.as_ref(),
        runtime.active_microphone_session.is_some() && !runtime.inactivity.is_microphone_paused(),
        false,
    );

    let (
        mut segment_outputs,
        recording_file,
        _microphone_recording_file,
        system_audio_recording_file,
        mut active_screen_session,
        _active_microphone_session,
    ) = started_segment;

    let next_microphone_recording_file = next_reanchored_microphone_output_file(
        runtime,
        next_index,
        "recovering after system wake",
    )?;
    if let Some(microphone_output_file) = next_microphone_recording_file.as_deref() {
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            let mic_finalization = match session
                .rotate_output_file_returning_finalization(microphone_output_file)
            {
                Ok(finalization) => finalization,
                Err(error) => {
                    let _ =
                        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                            active_session: &mut active_screen_session,
                            inactivity_tail_trim_seconds: 0,
                        });
                    cleanup_unusable_segment_artifacts(
                        Some(&segment_outputs),
                        recording_file.as_deref(),
                        None,
                        system_audio_recording_file.as_deref(),
                    );
                    return Err(error);
                }
            };
            apply_microphone_output_finalization(
                previous_microphone_outputs.as_mut(),
                &mic_finalization,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
            );
            append_and_persist_committed_audio_outputs(
                runtime,
                app_handle,
                previous_microphone_outputs.as_ref(),
            );
            set_current_microphone_output_file(
                &mut segment_outputs,
                microphone_output_file.to_string(),
            );
        }
    } else {
        merge_live_microphone_continuation_into_segment_outputs(runtime, &mut segment_outputs);
    }

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.inactivity.set_family_paused_states(
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.recording_file = recording_file;
    if next_microphone_recording_file.is_some() {
        runtime.microphone_recording_file = next_microphone_recording_file;
    }
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    reanchor_active_segment_timing(runtime, "recovering after system wake")?;

    Ok(true)
}

#[cfg(target_os = "macos")]
pub(super) fn recover_screen_capture_after_wake(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<bool, CaptureErrorResponse> {
    let metadata_snapshot_provider = app_handle.map(metadata::frame_metadata_snapshot_provider);
    let initial_privacy_filter = app_handle.map(privacy::collect_initial_privacy_filter);
    recover_screen_capture_after_wake_with_start_segment(
        runtime,
        app_handle,
        move |segment_dir,
              screen_output_file,
              system_audio_output_path,
              sources,
              screen_frame_rate,
              screen_resolution,
              effective_screen_bitrate_bps,
              microphone_device_id,
              frame_artifact_tx,
              microphone_output_path| {
            let started_segment = start_segment_with_inactivity_tail_trim_seconds(
                segment_dir,
                screen_output_file,
                system_audio_output_path,
                sources,
                screen_frame_rate,
                screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id,
                frame_artifact_tx,
                metadata_snapshot_provider,
                microphone_output_path,
                0,
                0.0,
                microphone_capture::MicrophoneInactivityTailTrimActivityMode::PeakLevel,
                initial_privacy_filter
                    .as_ref()
                    .and_then(|initial| initial.screen_capture_filter()),
            )?;
            if let (Some(app_handle), Some(initial)) = (app_handle, initial_privacy_filter) {
                initial.mark_applied(app_handle);
                if let Some(settings) = app_handle
                    .try_state::<crate::native_capture::RecordingSettingsState>()
                    .map(|state| {
                        state
                            .lock()
                            .expect("recording settings state poisoned")
                            .settings
                            .clone()
                    })
                {
                    privacy::record_initial_privacy_filter_outcome(
                        app_handle,
                        &settings,
                        started_segment.6,
                    );
                }
            }
            Ok((
                started_segment.0,
                started_segment.1,
                started_segment.2,
                started_segment.3,
                started_segment.4,
                started_segment.5,
            ))
        },
    )
}

#[cfg(target_os = "macos")]
pub(super) fn resume_runtime_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_paused {
        return Ok(());
    }

    let Some(sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming inactivity".to_string(),
        });
    };

    if runtime.inactivity.screen_paused
        || runtime.inactivity.microphone_paused
        || runtime.inactivity.system_audio_paused
    {
        refresh_current_segment_sources_for_pause_state(runtime, &sources);
        return Ok(());
    }

    runtime.inactivity.is_paused = false;
    runtime.current_segment_sources = if runtime.capture_suspension.is_some() {
        privacy_suspended_sources_for_runtime_state(
            runtime,
            runtime.inactivity.is_microphone_paused(),
        )
    } else {
        Some(sources)
    };

    Ok(())
}

#[cfg(target_os = "macos")]
fn should_fail_runtime_on_inactivity_resume_error(error: &CaptureErrorResponse) -> bool {
    // Missing core runtime state indicates a corrupted paused session; retrying will not recover it.
    error.code == "invalid_runtime_state"
}

#[cfg(target_os = "macos")]
pub(super) fn handle_inactivity_resume_error(
    runtime: &mut NativeCaptureRuntime,
    error: CaptureErrorResponse,
) -> bool {
    if should_fail_runtime_on_inactivity_resume_error(&error) {
        super::debug_log::log(format!(
            "fatal native capture inactivity resume failure: [{}] {}",
            error.code, error.message
        ));
        mark_runtime_session_failed(runtime);
        return true;
    }

    super::debug_log::log(format!(
        "failed to resume native capture after activity; keeping session paused for retry: [{}] {}",
        error.code, error.message
    ));

    false
}

#[cfg(target_os = "macos")]
pub(super) fn start_segment_with_current_privacy_filter(
    app_handle: &tauri::AppHandle,
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    system_audio_output_path: Option<&Path>,
    sources: &CaptureSources,
    screen_frame_rate: f64,
    screen_resolution: &capture_types::ScreenResolution,
    effective_screen_bitrate_bps: Option<u32>,
    microphone_device_id: Option<&str>,
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
    microphone_output_path: Option<&Path>,
) -> Result<StartedSegmentState, CaptureErrorResponse> {
    let initial_privacy_filter = privacy::collect_initial_privacy_filter(app_handle);
    let started_segment = start_segment_with_inactivity_tail_trim_seconds(
        session_dir,
        screen_output_file,
        system_audio_output_path,
        sources,
        screen_frame_rate,
        screen_resolution,
        effective_screen_bitrate_bps,
        microphone_device_id,
        frame_artifact_tx,
        Some(metadata::frame_metadata_snapshot_provider(app_handle)),
        microphone_output_path,
        0,
        0.0,
        microphone_capture::MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        initial_privacy_filter.screen_capture_filter(),
    )?;
    initial_privacy_filter.mark_applied(app_handle);
    if let Some(settings) = app_handle
        .try_state::<crate::native_capture::RecordingSettingsState>()
        .map(|state| {
            state
                .lock()
                .expect("recording settings state poisoned")
                .settings
                .clone()
        })
    {
        privacy::record_initial_privacy_filter_outcome(app_handle, &settings, started_segment.6);
    }
    Ok((
        started_segment.0,
        started_segment.1,
        started_segment.2,
        started_segment.3,
        started_segment.4,
        started_segment.5,
    ))
}

#[cfg(target_os = "macos")]
fn start_segment_with_inactivity_tail_trim_seconds(
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    system_audio_output_path: Option<&Path>,
    sources: &CaptureSources,
    screen_frame_rate: f64,
    screen_resolution: &capture_types::ScreenResolution,
    effective_screen_bitrate_bps: Option<u32>,
    microphone_device_id: Option<&str>,
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
    metadata_snapshot_provider: Option<metadata::FrameMetadataSnapshotProvider>,
    microphone_output_path: Option<&Path>,
    inactivity_tail_trim_seconds: u64,
    microphone_activity_threshold: f32,
    microphone_tail_activity_mode: microphone_capture::MicrophoneInactivityTailTrimActivityMode,
    initial_privacy_filter: Option<capture_screen::PrivacyContentFilter>,
) -> Result<
    (
        CaptureOutputFiles,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<capture_screen::ActiveCaptureSession>,
        Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
        Option<capture_screen::PrivacyFilterApplyOutcome>,
    ),
    CaptureErrorResponse,
> {
    let _ = &frame_artifact_tx;
    cleanup_failed_audio_outputs(microphone_output_path, system_audio_output_path);
    let microphone_audio_dir = microphone_output_path.and_then(|p| p.parent());
    let system_audio_dir = system_audio_output_path.and_then(|p| p.parent());
    create_segment_output_dirs(session_dir, microphone_audio_dir, system_audio_dir, sources)?;

    let mut output_files = empty_output_files();
    let mut recording_file: Option<String> = None;
    let mut microphone_recording_file: Option<String> = None;
    let mut system_audio_recording_file: Option<String> = None;
    let mut active_screen_session: Option<capture_screen::ActiveCaptureSession> = None;
    let mut active_microphone_session: Option<
        microphone_capture::AvFoundationMicrophoneCaptureSession,
    > = None;
    let mut initial_privacy_filter_outcome = None;

    if sources.screen || sources.system_audio {
        let screen_sources = capture_screen::ScreenCaptureSources {
            screen: sources.screen,
            system_audio: sources.system_audio,
        };
        let mut screen_options = capture_session_options(
            frame_artifact_tx,
            metadata_snapshot_provider,
            inactivity_tail_trim_seconds,
            initial_privacy_filter,
        );
        if sources.system_audio && system_audio_output_path.is_none() {
            screen_options.system_audio_writer_active = Some(false);
        }
        let screen_capture = match capture_screen::start_capture_session_with_options(
            session_dir,
            screen_output_file,
            system_audio_output_path,
            &screen_sources,
            screen_frame_rate,
            screen_resolution,
            effective_screen_bitrate_bps,
            screen_options,
        ) {
            Ok(screen_capture) => screen_capture,
            Err(error) => {
                if error.code != "capture_start_rollback_incomplete" {
                    cleanup_failed_segment_dirs(
                        session_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                }
                return Err(error);
            }
        };

        if let Some(screen_file) = screen_capture.output_files.screen_file {
            set_current_screen_output_file(&mut output_files, screen_file);
        }
        if let Some(system_audio_file) = screen_capture.output_files.system_audio_file {
            set_current_system_audio_output_file(&mut output_files, system_audio_file);
        }

        recording_file = Some(screen_capture.recording_file);
        system_audio_recording_file = screen_capture.system_audio_recording_file;
        initial_privacy_filter_outcome = screen_capture.initial_privacy_filter_outcome;
        active_screen_session = Some(screen_capture.session);
    }

    if sources.microphone {
        let microphone_output_file = microphone_output_path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let mic_start =
            microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id_and_inactivity_tail_trim_activity_mode(
                &microphone_output_file,
                microphone_device_id,
                inactivity_tail_trim_seconds,
                microphone_activity_threshold,
                microphone_tail_activity_mode,
            );

        match mic_start {
            Ok(session) => {
                set_current_microphone_output_file(
                    &mut output_files,
                    microphone_output_file.clone(),
                );
                microphone_recording_file = Some(microphone_output_file);
                active_microphone_session = Some(session);
            }
            Err(error) => {
                if let Err(rollback_error) =
                    capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                        active_session: &mut active_screen_session,
                        inactivity_tail_trim_seconds: 0,
                    })
                {
                    return Err(CaptureErrorResponse {
                        code: error.code,
                        message: format!(
                            "{}; additionally failed to rollback screen capture session: [{}] {}",
                            error.message, rollback_error.code, rollback_error.message
                        ),
                    });
                }

                cleanup_failed_segment_dirs(session_dir, microphone_audio_dir, system_audio_dir);
                return Err(error);
            }
        }
    }

    Ok((
        output_files,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        active_microphone_session,
        initial_privacy_filter_outcome,
    ))
}

#[cfg(target_os = "macos")]
fn spawn_segment_loop(app_handle: tauri::AppHandle) -> SegmentLoopControl {
    let control = SegmentLoopControl::new();
    let worker_control = control.clone();

    thread::spawn(move || {
        let mut last_privacy_filter_poll = Instant::now()
            .checked_sub(PRIVACY_FILTER_POLL_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut last_suspension_recovery_attempt = Instant::now()
            .checked_sub(DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut last_low_disk_recovery_attempt = Instant::now()
            .checked_sub(LOW_DISK_RECOVERY_INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            let sleep_duration = {
                let capture_state = app_handle.state::<NativeCaptureState>();
                let runtime = match capture_state.lock() {
                    Ok(runtime) => runtime,
                    Err(_) => break,
                };
                let runtime = runtime.runtime();

                if !runtime.is_running {
                    break;
                }

                let Some(schedule) = runtime.segment_schedule.as_ref() else {
                    break;
                };
                let Some(clock) = runtime.capture_clock.as_ref() else {
                    break;
                };

                segment_loop_sleep_duration(schedule, clock)
            };

            if !sleep_duration.is_zero() {
                worker_control.wait_timeout(sleep_duration);
            }

            if worker_control.stop.load(Ordering::Relaxed) {
                break;
            }

            if last_privacy_filter_poll.elapsed() >= PRIVACY_FILTER_POLL_INTERVAL {
                privacy::request_privacy_filter_refresh(
                    &app_handle,
                    privacy::PrivacyRefreshReason::FallbackPoll,
                );
                last_privacy_filter_poll = Instant::now();
            }
            privacy::maybe_start_privacy_filter_collection(&app_handle);
            let privacy_filter_update = privacy::take_completed_privacy_filter_update(&app_handle);
            privacy::maybe_start_privacy_filter_collection(&app_handle);

            let capture_state = app_handle.state::<NativeCaptureState>();
            let mut runtime = match capture_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => break,
            };

            if !runtime.runtime().is_running || worker_control.stop.load(Ordering::Relaxed) {
                break;
            }

            // Low-disk recovery runs on its own ~10s throttle, independent of the
            // privacy-filter update channel: free space recovers without any
            // privacy-filter event arriving, so this re-probes on a calm cadence
            // and resumes all sources (incl. the mic) once free space climbs back
            // above the resume threshold. Disk recovers far more slowly than a
            // display waking, hence the longer interval vs display recovery.
            let low_disk_suspended = runtime
                .runtime()
                .capture_suspension
                .as_ref()
                .is_some_and(|suspension| suspension.kind == CaptureSuspensionKind::LowDisk);
            if low_disk_suspended
                && last_low_disk_recovery_attempt.elapsed() >= LOW_DISK_RECOVERY_INTERVAL
            {
                last_low_disk_recovery_attempt = Instant::now();
                match attempt_privacy_suspension_recovery(&app_handle, runtime.runtime_mut()) {
                    PrivacySuspensionRecoveryOutcome::Recovered => {
                        super::debug_log::log(
                            "low-disk capture recovered; restarted all sources after suspension",
                        );
                    }
                    PrivacySuspensionRecoveryOutcome::RestartRequired
                    | PrivacySuspensionRecoveryOutcome::RetryPending
                    | PrivacySuspensionRecoveryOutcome::NotSuspended => {}
                }
            }

            if let Some(privacy_filter_update) = privacy_filter_update {
                if runtime.runtime().capture_suspension.is_some() {
                    // Throttle display-unavailable recovery so we probe for a
                    // returning display at a calm cadence instead of on every 1s
                    // poll. Privacy-filter recovery is left to retry promptly
                    // (it's capped at a few attempts).
                    let suspension_kind = runtime
                        .runtime()
                        .capture_suspension
                        .as_ref()
                        .map(|suspension| suspension.kind);
                    let throttle_display_recovery = suspension_kind
                        == Some(CaptureSuspensionKind::DisplayUnavailable)
                        && last_suspension_recovery_attempt.elapsed()
                            < DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL;
                    // Low-disk recovery is owned by the dedicated ~10s-throttled
                    // block above; never drive it from the privacy-update channel
                    // (which would bypass that throttle on every privacy event).
                    let is_low_disk = suspension_kind == Some(CaptureSuspensionKind::LowDisk);
                    if !throttle_display_recovery && !is_low_disk {
                        last_suspension_recovery_attempt = Instant::now();
                        match attempt_privacy_suspension_recovery(
                            &app_handle,
                            runtime.runtime_mut(),
                        ) {
                            PrivacySuspensionRecoveryOutcome::Recovered => {
                                super::debug_log::log(
                                    "screen/system-audio capture recovered; restarted after suspension",
                                );
                            }
                            PrivacySuspensionRecoveryOutcome::RestartRequired => {}
                            PrivacySuspensionRecoveryOutcome::RetryPending
                            | PrivacySuspensionRecoveryOutcome::NotSuspended => {}
                        }
                    }
                } else {
                    let apply_result = privacy::apply_privacy_filter_update(
                        &app_handle,
                        runtime.runtime_mut(),
                        privacy_filter_update.update,
                    );
                    match apply_result {
                        Ok(outcome) => {
                            if privacy::privacy_refresh_debug_log_enabled(
                                privacy_filter_update.reason,
                            ) {
                                super::debug_log::log(format!(
                                    "privacy refresh apply succeeded (reason={:?}, generation={}, mode={:?}, request_satisfied={})",
                                    privacy_filter_update.reason,
                                    privacy_filter_update.generation,
                                    privacy_filter_update.mode,
                                    outcome.request_satisfied
                                ));
                            }
                            privacy::record_privacy_filter_apply_outcome(
                                &app_handle,
                                privacy_filter_update.mode,
                                outcome,
                            );
                        }
                        Err(error) => {
                            let display_unavailable =
                                error.code == privacy::PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE;
                            let suspension_kind = if display_unavailable {
                                CaptureSuspensionKind::DisplayUnavailable
                            } else {
                                CaptureSuspensionKind::PrivacyFilter
                            };
                            if display_unavailable {
                                super::debug_log::log(format!(
                                    "capture display unavailable; suspending screen/system-audio until the display returns: [{}] {}",
                                    error.code, error.message
                                ));
                            } else {
                                super::debug_log::log(format!(
                                    "privacy filter update failed; suspending screen/system-audio capture: [{}] {}",
                                    error.code, error.message
                                ));
                            }
                            if let Err(stop_error) = suspend_screen_system_audio_capture(
                                Some(&app_handle),
                                runtime.runtime_mut(),
                                &error,
                                suspension_kind,
                            ) {
                                super::debug_log::log(format!(
                        "capture suspension could not stop screen/system-audio capture; preserving runtime state: [{}] {}",
                        stop_error.code, stop_error.message
                    ));
                                if !capture_screen::should_preserve_runtime_on_stop_error(
                                    &stop_error,
                                ) {
                                    mark_runtime_session_failed(runtime.runtime_mut());
                                    break;
                                }
                                continue;
                            }
                            // A transient display loss keeps the session alive so
                            // recovery can resume screen/system-audio when the
                            // display returns, even with no microphone. A genuine
                            // privacy-filter failure with no other live source
                            // can't make progress, so end the session.
                            if !display_unavailable
                                && !runtime
                                    .runtime()
                                    .requested_sources
                                    .as_ref()
                                    .is_some_and(|sources| sources.microphone)
                            {
                                mark_runtime_session_failed(runtime.runtime_mut());
                                break;
                            }
                        }
                    }
                }
            }

            match runtime.tick_inactivity(&app_handle) {
                TickOutcome::Continue => {}
                TickOutcome::SkipRotation => continue,
                TickOutcome::StopLoop => break,
            }

            match runtime.tick_rotation(&app_handle) {
                TickOutcome::Continue => {}
                TickOutcome::SkipRotation => continue,
                TickOutcome::StopLoop => break,
            }
        }

        // The loop only falls out on its own when the session ended internally
        // (a fatal capture failure, a privacy/display suspension that could not
        // continue, lost sources, etc.). User- and command-initiated stops set
        // the worker stop flag and broadcast the new session state themselves, so
        // skip those to avoid racing their teardown. For an internal end nobody
        // else announces it, so the frontend and the native status-bar tray would
        // otherwise keep showing a running session (e.g. "Stop Recording") that no
        // longer exists. Broadcast the real state so both surfaces resync.
        if !worker_control.stop.load(Ordering::Relaxed) {
            let session = app_handle
                .state::<NativeCaptureState>()
                .lock()
                .ok()
                .map(|runtime| runtime.session());
            if let Some(session) = session {
                if !session.is_running {
                    super::emit_native_capture_session_changed(&app_handle, &session);
                    crate::status_bar::refresh(&app_handle);
                }
            }
        }
    });

    control
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn privacy_failure_without_microphone_commits_current_screen_output_before_runtime_failure() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_path = temp_dir.path().join("screen-segment.mov");
        std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
            .expect("fake openable mov should be written");
        let screen_path = screen_path.to_string_lossy().into_owned();

        let mut runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: false,
                system_audio: false,
            }),
            current_segment_sources: Some(CaptureSources {
                screen: true,
                microphone: false,
                system_audio: false,
            }),
            output_files: Some(empty_output_files()),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some(screen_path.clone()),
                screen_files: vec![screen_path.clone()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            recording_file: Some(screen_path.clone()),
            ..Default::default()
        };
        let error = CaptureErrorResponse {
            code: "privacy_update_failed".to_string(),
            message: "privacy update failed".to_string(),
        };

        suspend_screen_system_audio_capture(
            None,
            &mut runtime,
            &error,
            CaptureSuspensionKind::PrivacyFilter,
        )
        .expect("privacy suspension should succeed");
        assert!(
            runtime.current_segment_output_files.is_none(),
            "without microphone continuation, suspended screen/system outputs should already be committed and detached"
        );
        mark_runtime_session_failed(&mut runtime);

        assert!(!runtime.is_running);
        assert!(runtime.current_segment_output_files.is_none());
        let output_files = runtime
            .output_files
            .expect("committed output files should be preserved after runtime failure");
        assert_eq!(
            output_files.screen_file.as_deref(),
            Some(screen_path.as_str())
        );
        assert_eq!(output_files.screen_files, vec![screen_path]);
    }

    #[test]
    fn display_unavailable_suspension_commits_tail_segment_and_stays_running() {
        // A display-unavailable suspension used to skip committing the in-flight
        // segment because the dead stream left it truncated. The stop path now
        // finalizes the writers even for a delegate-terminated stream, so the
        // tail `.mov` is openable — the suspend path must commit it (not orphan
        // it) and must not fail the session, so a screen-only recording resumes
        // automatically when the display returns (ADR 0021 amendment).
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_path = temp_dir.path().join("screen-segment.mov");
        std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
            .expect("fake openable mov should be written");
        let screen_path = screen_path.to_string_lossy().into_owned();

        let mut runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: false,
                system_audio: false,
            }),
            current_segment_sources: Some(CaptureSources {
                screen: true,
                microphone: false,
                system_audio: false,
            }),
            output_files: Some(empty_output_files()),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some(screen_path.clone()),
                screen_files: vec![screen_path.clone()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            recording_file: Some(screen_path.clone()),
            ..Default::default()
        };
        let error = CaptureErrorResponse {
            code: "privacy_filter_display_unavailable".to_string(),
            message: "no display".to_string(),
        };

        suspend_screen_system_audio_capture(
            None,
            &mut runtime,
            &error,
            CaptureSuspensionKind::DisplayUnavailable,
        )
        .expect("display-unavailable suspension should succeed");

        // The session is kept alive for recovery rather than failed.
        assert!(runtime.is_running);
        assert_eq!(
            runtime
                .capture_suspension
                .as_ref()
                .map(|suspension| suspension.kind),
            Some(CaptureSuspensionKind::DisplayUnavailable)
        );
        // The finalized in-flight tail segment is committed, not orphaned.
        assert!(runtime.current_segment_output_files.is_none());
        assert!(runtime.recording_file.is_none());
        let output_files = runtime
            .output_files
            .expect("output files collection should be preserved");
        assert!(
            output_files.screen_files.contains(&screen_path),
            "the finalized in-flight tail segment must be committed"
        );
    }

    #[test]
    fn privacy_failure_with_paused_microphone_keeps_suspended_sources_explicit() {
        let mut runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            }),
            current_segment_sources: Some(CaptureSources {
                screen: true,
                microphone: false,
                system_audio: true,
            }),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                screen_files: vec!["/tmp/screen.mov".to_string()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
                system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
            }),
            recording_file: Some("/tmp/screen.mov".to_string()),
            system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
            inactivity: super::super::inactivity::InactivityState {
                enabled: true,
                idle_timeout_seconds: 10,
                microphone_paused: true,
                is_paused: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let error = CaptureErrorResponse {
            code: "privacy_update_failed".to_string(),
            message: "privacy update failed".to_string(),
        };

        suspend_screen_system_audio_capture(
            None,
            &mut runtime,
            &error,
            CaptureSuspensionKind::PrivacyFilter,
        )
        .expect("privacy suspension should succeed");

        assert_eq!(
            runtime.current_segment_sources,
            Some(CaptureSources {
                screen: false,
                microphone: false,
                system_audio: false,
            })
        );
        assert!(
            super::super::runtime::current_segment_sources_for_runtime(&runtime).is_none(),
            "explicit all-paused privacy suspension must not fall back to stale screen/system-audio outputs"
        );
    }

    #[test]
    fn privacy_failure_with_active_microphone_detaches_suspended_screen_state() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_path = temp_dir.path().join("screen-segment.mov");
        std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
            .expect("fake openable mov should be written");
        let screen_path = screen_path.to_string_lossy().into_owned();
        let microphone_path = temp_dir
            .path()
            .join("microphone.m4a")
            .to_string_lossy()
            .into_owned();

        let mut runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            current_segment_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(empty_output_files()),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some(screen_path.clone()),
                screen_files: vec![screen_path.clone()],
                microphone_file: Some(microphone_path.clone()),
                microphone_files: vec![microphone_path.clone()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            recording_file: Some(screen_path.clone()),
            microphone_recording_file: Some(microphone_path.clone()),
            system_audio_recording_file: Some("/tmp/stale-system-audio.m4a".to_string()),
            ..Default::default()
        };
        let error = CaptureErrorResponse {
            code: "privacy_update_failed".to_string(),
            message: "privacy update failed".to_string(),
        };

        suspend_screen_system_audio_capture(
            None,
            &mut runtime,
            &error,
            CaptureSuspensionKind::PrivacyFilter,
        )
        .expect("privacy suspension should succeed");

        assert!(runtime.recording_file.is_none());
        assert!(runtime.system_audio_recording_file.is_none());
        assert_eq!(
            runtime.current_segment_sources,
            Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            })
        );

        let current_outputs = runtime
            .current_segment_output_files
            .as_ref()
            .expect("microphone continuation should remain current");
        assert!(current_outputs.screen_file.is_none());
        assert!(current_outputs.screen_files.is_empty());
        assert_eq!(
            current_outputs.microphone_file.as_deref(),
            Some(microphone_path.as_str())
        );
        assert_eq!(current_outputs.microphone_files, vec![microphone_path]);

        let committed = runtime
            .output_files
            .as_ref()
            .expect("suspended screen output should be committed");
        assert_eq!(committed.screen_file.as_deref(), Some(screen_path.as_str()));
        assert_eq!(committed.screen_files, vec![screen_path]);
    }
}

pub(super) fn start_capture_runtime(
    runtime: &mut NativeCaptureRuntime,
    app_handle: tauri::AppHandle,
    settings: &RecordingSettings,
    sources: CaptureSources,
    microphone_device_id_for_capture: Option<String>,
) -> Result<(), CaptureErrorResponse> {
    if settings.capture_screen || settings.capture_system_audio {
        let screen_ok = capture_screen::ensure_screen_permission();
        if !screen_ok {
            return Err(CaptureErrorResponse {
                code: "screen_permission_denied".to_string(),
                message: if settings.capture_system_audio {
                    "Screen capture permission is required for system audio capture"
                } else {
                    "Screen capture permission is required"
                }
                .to_string(),
            });
        }
    }

    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
    apply_runtime_signal(runtime, RuntimeSignal::StartRequested)?;

    if settings.capture_microphone {
        let microphone_ok = microphone_capture::ensure_microphone_permission();
        if !microphone_ok {
            return Err(CaptureErrorResponse {
                code: "microphone_permission_denied".to_string(),
                message: "Microphone permission is required".to_string(),
            });
        }
    }

    let start_result: Result<(), CaptureErrorResponse> = {
        #[cfg(target_os = "macos")]
        {
            let started = now_unix_ms();
            let started_monotonic = now_monotonic_marker_ms();
            let capture_id = prefixed_capture_id("cap")?;
            let session_id = prefixed_capture_id("screen")?;
            // Generate independent session IDs for microphone and system audio so each
            // source writes filenames tagged with its own logical source session.
            let microphone_session_id = if sources.microphone {
                Some(prefixed_capture_id("mic")?)
            } else {
                None
            };
            let system_audio_session_id = if sources.system_audio {
                Some(prefixed_capture_id("sysaudio")?)
            } else {
                None
            };
            let recordings_root =
                crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
                    &settings.save_directory,
                )
                .recordings_root();
            let segment_planner = SegmentPlanner::new(
                recordings_root.to_string_lossy().to_string(),
                session_id.clone(),
            );
            let microphone_planner = microphone_session_id.as_deref().map(|mic_id| {
                SegmentPlanner::new(recordings_root.to_string_lossy().to_string(), mic_id)
            });
            let system_audio_planner = system_audio_session_id.as_deref().map(|sa_id| {
                SegmentPlanner::new(recordings_root.to_string_lossy().to_string(), sa_id)
            });
            let segment_schedule =
                SegmentSchedule::new(Duration::from_secs(settings.segment_duration_seconds));
            let capture_clock = CaptureClock::start_now();
            let frame_artifact_tx = sources
                .screen
                .then(|| spawn_frame_artifact_worker(&app_handle, session_id.clone()));
            std::fs::create_dir_all(&recordings_root).map_err(|error| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!("Failed to create capture recordings directory: {error}"),
            })?;

            let segment_index = 1;
            let first_segment_dir = segment_planner.segment_dir(segment_index);
            let first_screen_output_file = segment_planner.segment_screen_output(segment_index);
            let first_system_audio_output_path = system_audio_planner
                .as_ref()
                .map(|p| p.system_audio_file(segment_index));
            let first_microphone_output_path = microphone_planner
                .as_ref()
                .map(|p| p.microphone_file(segment_index));
            let effective_screen_bitrate_bps = compute_effective_screen_bitrate_bps(settings);

            // Low-disk preflight (ADR 0040): refuse to start on a volume too full
            // to safely hold even the first segment, before any file is opened.
            // Best-effort — an unmeasurable reading never blocks the start; only a
            // measured shortfall below the pause threshold refuses here.
            preflight_disk_space_check(
                &recordings_root,
                effective_screen_bitrate_bps,
                &sources,
                settings.segment_duration_seconds,
                runtime.free_space_probe(),
            )?;

            capture_screen::reset_last_screen_activity_unix_ms();
            microphone_capture::reset_last_microphone_activity_unix_ms();
            let initial_inactivity = super::inactivity::InactivityState::from_recording_settings(
                settings,
                started_monotonic,
            );
            let inactivity_tail_trim_seconds = settings
                .pause_capture_on_inactivity
                .then_some(settings.idle_timeout_seconds)
                .unwrap_or(0);
            let microphone_activity_threshold = settings
                .pause_capture_on_inactivity
                .then(|| initial_inactivity.microphone_activity_threshold())
                .unwrap_or(0.0);
            let initial_microphone_vad = MicrophoneVadRuntime::new(settings.microphone_vad_adapter);
            let microphone_tail_activity_mode =
                microphone_tail_trim_activity_mode_for_vad(&initial_microphone_vad);
            privacy::reset_privacy_filter_refresh_state(&app_handle);
            let initial_privacy_filter = privacy::collect_initial_privacy_filter(&app_handle);

            let (
                segment_outputs,
                recording_file,
                microphone_recording_file,
                system_audio_recording_file,
                active_screen_session,
                active_microphone_session,
                initial_privacy_filter_outcome,
            ) = start_segment_with_inactivity_tail_trim_seconds(
                &first_segment_dir,
                Some(&first_screen_output_file),
                first_system_audio_output_path.as_deref(),
                &sources,
                settings.screen_frame_rate,
                &settings.screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id_for_capture.as_deref(),
                frame_artifact_tx.clone(),
                Some(metadata::frame_metadata_snapshot_provider(&app_handle)),
                first_microphone_output_path.as_deref(),
                inactivity_tail_trim_seconds,
                microphone_activity_threshold,
                microphone_tail_activity_mode,
                initial_privacy_filter.screen_capture_filter(),
            )?;
            initial_privacy_filter.mark_applied(&app_handle);
            privacy::record_initial_privacy_filter_outcome(
                &app_handle,
                &settings,
                initial_privacy_filter_outcome,
            );

            let output_files = empty_output_files();
            let segment_loop_control = spawn_segment_loop(app_handle.clone());
            let source_sessions = SourceSessions {
                screen: sources.screen.then(|| SourceSessionMeta {
                    session_id: session_id.clone(),
                    started_at_unix_ms: started,
                }),
                microphone: sources.microphone.then(|| SourceSessionMeta {
                    session_id: microphone_session_id
                        .clone()
                        .expect("microphone session id should exist when source is enabled"),
                    started_at_unix_ms: started,
                }),
                system_audio: sources.system_audio.then(|| SourceSessionMeta {
                    session_id: system_audio_session_id
                        .clone()
                        .expect("system audio session id should exist when source is enabled"),
                    started_at_unix_ms: started,
                }),
            };
            persist_capture_session_started(
                &app_handle,
                capture_id,
                started,
                &sources,
                &source_sessions,
                settings.segment_duration_seconds,
            );

            runtime.is_running = true;
            runtime.inactivity = initial_inactivity;
            runtime.microphone_vad = initial_microphone_vad;
            runtime.source_sessions = Some(source_sessions);
            runtime.requested_sources = Some(sources.clone());
            runtime.current_segment_sources = Some(sources);
            runtime.output_files = Some(output_files);
            runtime.current_segment_output_files = Some(segment_outputs);
            runtime.current_segment_index = segment_index;
            runtime.screen_frame_rate = settings.screen_frame_rate;
            runtime.screen_resolution = settings.screen_resolution.clone();
            runtime.effective_screen_bitrate_bps = effective_screen_bitrate_bps;
            runtime.microphone_device_id_for_capture = microphone_device_id_for_capture;
            runtime.segment_loop_control = Some(segment_loop_control);
            runtime.capture_clock = Some(capture_clock);
            runtime.segment_schedule = Some(segment_schedule);
            runtime.segment_planner = Some(segment_planner);
            runtime.microphone_planner = microphone_planner;
            runtime.system_audio_planner = system_audio_planner;
            runtime.frame_artifact_tx = frame_artifact_tx;
            runtime.recording_file = recording_file;
            runtime.microphone_recording_file = microphone_recording_file;
            runtime.system_audio_recording_file = system_audio_recording_file;
            runtime.active_screen_session = active_screen_session;
            runtime.active_microphone_session = active_microphone_session;
            runtime.capture_suspension = None;
            apply_runtime_signal(runtime, RuntimeSignal::SourcesReady)?;
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = sources;
            let _ = microphone_device_id_for_capture;
            let _ = app_handle;
            Err(CaptureErrorResponse {
                code: "unsupported_platform".to_string(),
                message: "Native capture is currently supported only on macOS".to_string(),
            })
        }
    };

    if let Err(error) = start_result {
        reset_runtime_after_start_error(runtime);
        return Err(error);
    }

    Ok(())
}

pub(super) fn stop_capture_runtime(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    #[cfg(target_os = "macos")]
    {
        request_runtime_stop_transition_if_needed(runtime)?;

        let mut current_segment_output_files = runtime.current_segment_output_files.clone();
        let recording_file = runtime.recording_file.clone().or_else(|| {
            current_screen_output_file(current_segment_output_files.as_ref()).map(str::to_owned)
        });
        let microphone_recording_file = runtime.microphone_recording_file.clone();
        let system_audio_recording_file =
            runtime.system_audio_recording_file.clone().or_else(|| {
                current_system_audio_output_file(current_segment_output_files.as_ref())
                    .map(str::to_owned)
            });
        let requested_sources = runtime.requested_sources.clone();

        let mut first_error: Option<CaptureErrorResponse> = None;

        if let Some(session) = runtime.active_microphone_session.as_mut() {
            match session.stop_returning_finalization() {
                Ok(finalization) => apply_microphone_output_finalization(
                    current_segment_output_files.as_mut(),
                    &finalization,
                    runtime.source_sessions.as_ref(),
                    runtime.segment_schedule.as_ref(),
                    runtime.current_segment_index,
                ),
                Err(error) => {
                    first_error = Some(error);
                }
            }
            runtime.active_microphone_session = None;
        }

        if let Err(error) =
            capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                active_session: &mut runtime.active_screen_session,
                inactivity_tail_trim_seconds: 0,
            })
        {
            if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                return Err(error);
            }

            if first_error.is_none() {
                first_error = Some(error);
            }
        }

        if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
            flush_frame_artifacts(tx);
        }

        if let Err(error) = finalize_capture_outputs(
            current_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        ) {
            cleanup_unusable_segment_artifacts(
                current_segment_output_files.as_ref(),
                recording_file.as_deref(),
                microphone_recording_file.as_deref(),
                system_audio_recording_file.as_deref(),
            );
            if let Some(previous_error) = first_error.take() {
                first_error = Some(CaptureErrorResponse {
                    code: previous_error.code,
                    message: format!(
                        "{}; additionally failed to finalize capture outputs: [{}] {}",
                        previous_error.message, error.code, error.message
                    ),
                });
            } else {
                first_error = Some(error);
            }
        }

        if first_error.is_none() {
            if let (Some(committed), Some(segment)) = (
                runtime.output_files.as_mut(),
                current_segment_output_files.as_ref(),
            ) {
                append_committed_segment_output_files(committed, segment);
            }
            persist_committed_audio_segments(
                app_handle,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                current_segment_output_files.as_ref(),
            );
            warm_scrub_previews_for_committed_screen_outputs(
                app_handle,
                current_segment_output_files.as_ref(),
            );
            close_frame_batches_for_stopped_screen_session(
                app_handle,
                runtime.source_sessions.as_ref(),
            )?;
        }

        if let Some(error) = first_error {
            Err(error)
        } else {
            if runtime.runtime_state == RuntimeState::Stopping {
                apply_runtime_signal(runtime, RuntimeSignal::SourcesStopped)?;
            }
            Ok(())
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = runtime;
        let _ = app_handle;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn request_runtime_stop_transition_if_needed(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    match runtime.runtime_controller.state() {
        RuntimeState::Idle => {
            runtime.runtime_state = RuntimeState::Idle;
            Ok(())
        }
        RuntimeState::Stopping => {
            runtime.runtime_state = RuntimeState::Stopping;
            Ok(())
        }
        _ if runtime.is_running => apply_runtime_signal(runtime, RuntimeSignal::StopRequested),
        _ => Ok(()),
    }
}
