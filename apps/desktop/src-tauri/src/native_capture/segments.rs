use super::output::{
    append_committed_segment_output_files, set_current_microphone_output_file,
    set_current_screen_output_file, set_current_system_audio_output_file,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::output::finalize_capture_outputs;
use super::settings::compute_effective_screen_bitrate_bps;
use super::metadata;
#[cfg(target_os = "macos")]
use super::privacy;
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
#[cfg(target_os = "macos")]
use capture_vad::MicrophoneVadRuntime;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;
use tauri::Manager;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc;

use super::emit_audio_segments_changed;
use super::lifecycle::TickOutcome;
use super::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal, has_any_capture_sources,
    now_monotonic_marker_ms, now_unix_ms, prefixed_capture_id, refresh_runtime_planner_dates,
    reset_runtime_after_start_error, NativeCaptureRuntime, SegmentLoopControl,
};
#[cfg(any(target_os = "macos", test))]
use super::runtime::mark_runtime_session_failed;
#[cfg(target_os = "macos")]
use super::runtime::{
    ensure_microphone_planner_for_runtime, ensure_system_audio_planner_for_runtime,
    screen_planner_for_runtime, should_recover_from_segment_finalize_error,
};
#[cfg(target_os = "macos")]
use super::runtime::{
    privacy_suspended_sources_for_runtime_state, CaptureSuspensionKind, PrivacyCaptureSuspension,
    PrivacyCaptureSuspensionStatus,
};
use super::NativeCaptureState;

// Keep frame artifact persistence off the capture callback thread while bounding
// in-memory buffering. Backpressure is applied on a dedicated worker thread so
// exported frame artifacts are not dropped and the synchronous callback stays
// non-blocking.
const FRAME_ARTIFACT_BUFFER_CAPACITY: usize = 64;
const SEGMENT_LOOP_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(target_os = "macos")]
const PRIVACY_FILTER_POLL_INTERVAL: Duration = Duration::from_secs(1);
// While capture is suspended because the display is unavailable (display sleep,
// screen lock, lid close, monitor disconnect), throttle recovery attempts to
// this cadence so we don't churn ScreenCaptureKit restarts on every 1s poll.
#[cfg(target_os = "macos")]
const DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL: Duration = Duration::from_secs(2);

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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn stop_active_sessions_after_failure(runtime: &mut NativeCaptureRuntime) {
    #[cfg(target_os = "macos")]
    {
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            let _ = session.stop();
        }
        runtime.active_microphone_session = None;
    }
    #[cfg(target_os = "windows")]
    {
        // Dropping the boxed session tears the WASAPI/MF capture thread down, but
        // ask it to finalize first so a partially-written `.m4a` is still openable.
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            let _ = session.stop_returning_finalization();
        }
        runtime.active_microphone_session = None;
        if let Some(session) = runtime.active_system_audio_session.as_mut() {
            let _ = session.stop_returning_finalization();
        }
        runtime.active_system_audio_session = None;
    }

    let _ = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
        inactivity_tail_trim_seconds: 0,
    });
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(target_os = "macos")]
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
                    .and_then(|provider| provider());
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
    if runtime.privacy_capture_suspension.is_some() {
        return privacy_suspended_sources_for_runtime_state(runtime, microphone_paused);
    }

    active_sources_for_inactivity_paused_state(
        requested_sources,
        screen_paused,
        microphone_paused,
        system_audio_paused,
    )
}

#[cfg(target_os = "macos")]
fn suspend_screen_system_audio_capture(
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
    runtime.current_segment_sources = Some(explicit_privacy_suspension_sources(runtime));
    runtime.privacy_capture_suspension = Some(PrivacyCaptureSuspension::with_kind(kind, error));
    // A display-unavailable suspension means macOS already tore the screen stream
    // down, so the in-flight segment's `.mov` is incomplete/unopenable. Trying to
    // finalize it only emits a spurious "screen output missing" error every time
    // the display sleeps; the prior segments are already committed and recovery
    // starts a fresh segment, so skip the doomed commit for that kind.
    if kind != CaptureSuspensionKind::DisplayUnavailable {
        commit_suspended_screen_system_outputs(app_handle, runtime);
    }
    runtime.recording_file = None;
    runtime.system_audio_recording_file = None;
    preserve_live_microphone_continuation_outputs(runtime);
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

#[cfg(target_os = "macos")]
fn attempt_privacy_suspension_recovery(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
) -> PrivacySuspensionRecoveryOutcome {
    let suspension_kind = match runtime.privacy_capture_suspension.as_ref() {
        Some(suspension) => suspension.kind,
        None => return PrivacySuspensionRecoveryOutcome::NotSuspended,
    };

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
        .privacy_capture_suspension
        .as_ref()
        .is_some_and(PrivacyCaptureSuspension::can_retry);
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
        runtime.privacy_capture_suspension = None;
        return PrivacySuspensionRecoveryOutcome::NotSuspended;
    }

    let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
        if let Some(suspension) = runtime.privacy_capture_suspension.as_mut() {
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
                if let Some(suspension) = runtime.privacy_capture_suspension.as_mut() {
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
            if let Some(suspension) = runtime.privacy_capture_suspension.as_mut() {
                suspension.record_recovery_failure(&error);
                if suspension.status == PrivacyCaptureSuspensionStatus::RestartRequired {
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
    runtime.privacy_capture_suspension = None;
    if let Err(error) = reanchor_active_segment_timing(runtime, "recovering privacy suspension") {
        super::debug_log::log(format!(
            "failed to reanchor segment timing after privacy recovery: [{}] {}",
            error.code, error.message
        ));
    }

    PrivacySuspensionRecoveryOutcome::Recovered
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn rfc3339_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Windows has no AVFoundation duration probe; the segment window falls back to
/// the scheduled segment duration when `None` is returned. The `.m4a` is still
/// validated for a positive duration separately via the MF Source Reader probe.
#[cfg(target_os = "windows")]
fn audio_file_duration_ms(_file_path: &str) -> Option<u64> {
    None
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

/// Map a Windows audio finalization onto committed output bookkeeping.
///
/// Windows writes the final `.m4a` directly (source == output), so this only
/// confirms the closed segment's file or — when the segment captured no audio —
/// drops it from bookkeeping so an empty `.m4a` is never committed.
#[cfg(target_os = "windows")]
fn apply_windows_audio_output_finalization(
    output_files: Option<&mut CaptureOutputFiles>,
    finalization: &microphone_capture::MicrophoneOutputFinalization,
    is_system_audio: bool,
) {
    let Some(output_files) = output_files else {
        return;
    };

    match finalization.output_file.as_deref() {
        Some(output_file) => {
            let (current_file, files) = if is_system_audio {
                (
                    &mut output_files.system_audio_file,
                    &mut output_files.system_audio_files,
                )
            } else {
                (
                    &mut output_files.microphone_file,
                    &mut output_files.microphone_files,
                )
            };
            if files.iter().any(|file| file == output_file) {
                *current_file = Some(output_file.to_string());
            } else if is_system_audio {
                set_current_system_audio_output_file(output_files, output_file.to_string());
            } else {
                set_current_microphone_output_file(output_files, output_file.to_string());
            }
        }
        None => {
            if let Some(source_file) = finalization.source_file.as_deref() {
                if is_system_audio {
                    output_files.system_audio_files.retain(|file| file != source_file);
                } else {
                    output_files.microphone_files.retain(|file| file != source_file);
                }
            }
            if is_system_audio {
                output_files.system_audio_file = output_files.system_audio_files.last().cloned();
            } else {
                output_files.microphone_file = output_files.microphone_files.last().cloned();
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn apply_windows_microphone_output_finalization(
    output_files: Option<&mut CaptureOutputFiles>,
    finalization: &microphone_capture::MicrophoneOutputFinalization,
) {
    apply_windows_audio_output_finalization(output_files, finalization, false);
}

#[cfg(target_os = "windows")]
pub(super) fn apply_windows_system_audio_output_finalization(
    output_files: Option<&mut CaptureOutputFiles>,
    finalization: &microphone_capture::MicrophoneOutputFinalization,
) {
    apply_windows_audio_output_finalization(output_files, finalization, true);
}

#[cfg(target_os = "windows")]
fn clear_current_screen_output(output_files: Option<&mut CaptureOutputFiles>) {
    if let Some(output_files) = output_files {
        output_files.screen_file = None;
        output_files.screen_files.clear();
    }
}

#[cfg(target_os = "windows")]
fn clear_current_microphone_output(output_files: Option<&mut CaptureOutputFiles>) {
    if let Some(output_files) = output_files {
        output_files.microphone_file = None;
        output_files.microphone_files.clear();
    }
}

#[cfg(target_os = "windows")]
fn clear_current_system_audio_output(output_files: Option<&mut CaptureOutputFiles>) {
    if let Some(output_files) = output_files {
        output_files.system_audio_file = None;
        output_files.system_audio_files.clear();
    }
}

#[cfg(target_os = "windows")]
fn append_committed_outputs(runtime: &mut NativeCaptureRuntime, output_files: Option<&CaptureOutputFiles>) {
    if let (Some(committed), Some(output_files)) = (runtime.output_files.as_mut(), output_files) {
        append_committed_segment_output_files(committed, output_files);
    }
}

#[cfg(target_os = "windows")]
fn refresh_windows_current_segment_sources(runtime: &mut NativeCaptureRuntime) {
    let Some(requested_sources) = runtime.requested_sources.as_ref() else {
        runtime.current_segment_sources = None;
        return;
    };

    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        requested_sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
}

/// Set only the audio (microphone / system-audio) family pause flags, leaving the
/// screen pause flag, reason, and pause-start timestamp untouched (ADR 0023). An
/// audio inactivity pause/resume must not disturb a concurrent `TransientLiveness`
/// screen pause: clobbering its reason to `Inactivity` (the old behavior of
/// routing the audio paths through `set_family_paused_states`) would stop the
/// display probe from watching the screen and let the activity resume-all path
/// churn the screen back on against a display that may still be gone.
#[cfg(target_os = "windows")]
fn mark_windows_audio_family_paused(
    runtime: &mut NativeCaptureRuntime,
    microphone_paused: bool,
    system_audio_paused: bool,
) {
    runtime
        .inactivity
        .set_audio_family_paused_states(microphone_paused, system_audio_paused);
    refresh_windows_current_segment_sources(runtime);
}

/// Like [`mark_windows_family_paused`] but records an explicit screen-pause reason
/// (ADR 0023). The transient-liveness recovery path passes
/// `ScreenPauseReason::TransientLiveness { .. }` so the resume side knows the screen
/// must wait for a display/session-present probe rather than user activity.
#[cfg(target_os = "windows")]
fn mark_windows_family_paused_with_screen_reason(
    runtime: &mut NativeCaptureRuntime,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
    screen_pause_reason: super::inactivity::ScreenPauseReason,
) {
    runtime.inactivity.set_family_paused_states_with_reason(
        screen_paused,
        microphone_paused,
        system_audio_paused,
        screen_pause_reason,
    );
    if screen_paused {
        runtime
            .inactivity
            .mark_screen_pause_started_with_reason(now_monotonic_marker_ms(), screen_pause_reason);
    }
    refresh_windows_current_segment_sources(runtime);
}

#[cfg(target_os = "windows")]
pub(super) fn pause_screen_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    _app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    pause_windows_screen_with_reason(
        runtime,
        super::inactivity::ScreenPauseReason::Inactivity,
        "inactivity",
        false,
    )
}

/// Enter a screen-only transient-liveness suspension (ADR 0023). Reuses the same
/// stop/finalize-segment screen pause path as the inactivity slice, but records
/// the reason `TransientLiveness { trigger }` so the resume side waits for a
/// display/session-present probe instead of user activity. The WGC screen session
/// is already dead in this path (`GraphicsCaptureItem.Closed`/not-live), so the
/// stop is expected to merely finalize the partially-written segment — any stop
/// error is logged and tolerated rather than failing the session.
#[cfg(target_os = "windows")]
pub(super) fn pause_screen_for_transient_liveness(
    runtime: &mut NativeCaptureRuntime,
    trigger: super::inactivity::TransientLivenessTrigger,
) -> Result<(), CaptureErrorResponse> {
    pause_windows_screen_with_reason(
        runtime,
        super::inactivity::ScreenPauseReason::TransientLiveness { trigger },
        "transient liveness",
        true,
    )
}

/// Shared body for the Windows screen pause used by both the inactivity slice and
/// the transient-liveness recovery slice (ADR 0023). The two differ only in the
/// recorded `screen_pause_reason` and whether a backend stop error fails the
/// caller (`tolerate_stop_error`): the transient-liveness path's session is
/// already dead, so the stop just finalizes the partial segment.
#[cfg(target_os = "windows")]
fn pause_windows_screen_with_reason(
    runtime: &mut NativeCaptureRuntime,
    screen_pause_reason: super::inactivity::ScreenPauseReason,
    context: &str,
    tolerate_stop_error: bool,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_screen_paused()
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.screen)
    {
        return Ok(());
    }

    let mut screen_outputs = runtime.current_segment_output_files.clone().map(|mut outputs| {
        outputs.microphone_file = None;
        outputs.microphone_files.clear();
        outputs.system_audio_file = None;
        outputs.system_audio_files.clear();
        outputs
    });
    let recording_file = runtime.recording_file.clone();

    if let Err(error) = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
        inactivity_tail_trim_seconds: runtime.inactivity.idle_timeout_seconds,
    }) {
        if tolerate_stop_error {
            super::debug_log::log(format!(
                "Windows screen capture stop reported an issue while pausing for {context} (tolerated; the partial segment is still finalized): [{}] {}",
                error.code, error.message
            ));
        } else {
            return Err(error);
        }
    }

    if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
        flush_frame_artifacts(tx);
    }

    if let Err(error) = finalize_capture_outputs(
        screen_outputs.as_mut(),
        recording_file.as_deref(),
        None,
        None,
        Some(&CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
    ) {
        super::debug_log::log(format!(
            "Windows screen output finalization reported an issue while pausing for {context}: [{}] {}",
            error.code, error.message
        ));
    }

    append_committed_outputs(runtime, screen_outputs.as_ref());
    runtime.recording_file = None;
    clear_current_screen_output(runtime.current_segment_output_files.as_mut());
    mark_windows_family_paused_with_screen_reason(
        runtime,
        true,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
        screen_pause_reason,
    );

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn pause_microphone_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_microphone_paused()
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
    {
        return Ok(());
    }

    let mut microphone_outputs = runtime.current_segment_output_files.clone().map(|mut outputs| {
        outputs.screen_file = None;
        outputs.screen_files.clear();
        outputs.system_audio_file = None;
        outputs.system_audio_files.clear();
        outputs
    });
    let microphone_recording_file = runtime.microphone_recording_file.clone();

    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let finalization = session.stop_returning_finalization()?;
        apply_windows_microphone_output_finalization(microphone_outputs.as_mut(), &finalization);
    }
    runtime.active_microphone_session = None;

    if let Err(error) = finalize_capture_outputs(
        microphone_outputs.as_mut(),
        None,
        microphone_recording_file.as_deref(),
        None,
        Some(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
    ) {
        super::debug_log::log(format!(
            "Windows microphone output finalization reported an issue while pausing for inactivity: [{}] {}",
            error.code, error.message
        ));
    }

    append_committed_outputs(runtime, microphone_outputs.as_ref());
    persist_committed_audio_segments(
        app_handle,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
        microphone_outputs.as_ref(),
    );
    runtime.microphone_recording_file = None;
    clear_current_microphone_output(runtime.current_segment_output_files.as_mut());
    mark_windows_audio_family_paused(runtime, true, runtime.inactivity.system_audio_paused);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn pause_system_audio_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_system_audio_paused()
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.system_audio)
    {
        return Ok(());
    }

    let mut system_audio_outputs = runtime.current_segment_output_files.clone().map(|mut outputs| {
        outputs.screen_file = None;
        outputs.screen_files.clear();
        outputs.microphone_file = None;
        outputs.microphone_files.clear();
        outputs
    });
    let system_audio_recording_file = runtime.system_audio_recording_file.clone();

    if let Some(session) = runtime.active_system_audio_session.as_mut() {
        let finalization = session.stop_returning_finalization()?;
        apply_windows_system_audio_output_finalization(system_audio_outputs.as_mut(), &finalization);
    }
    runtime.active_system_audio_session = None;

    if let Err(error) = finalize_capture_outputs(
        system_audio_outputs.as_mut(),
        None,
        None,
        system_audio_recording_file.as_deref(),
        Some(&CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
    ) {
        super::debug_log::log(format!(
            "Windows system-audio output finalization reported an issue while pausing for inactivity: [{}] {}",
            error.code, error.message
        ));
    }

    append_committed_outputs(runtime, system_audio_outputs.as_ref());
    persist_committed_audio_segments(
        app_handle,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
        system_audio_outputs.as_ref(),
    );
    runtime.system_audio_recording_file = None;
    clear_current_system_audio_output(runtime.current_segment_output_files.as_mut());
    mark_windows_audio_family_paused(runtime, runtime.inactivity.microphone_paused, true);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn resume_screen_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_screen_paused() {
        return Ok(());
    }
    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming Windows screen from inactivity".to_string(),
        });
    };
    if !requested_sources.screen {
        return Ok(());
    }

    let resume_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    )
    .unwrap_or(CaptureSources {
        screen: false,
        microphone: false,
        system_audio: false,
    });
    start_windows_active_segment(
        app_handle,
        runtime,
        &resume_sources,
        "resuming Windows screen from inactivity",
    )?;
    runtime.inactivity.set_family_paused_states(
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    refresh_windows_current_segment_sources(runtime);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn resume_microphone_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_microphone_paused() {
        return Ok(());
    }
    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming Windows microphone from inactivity".to_string(),
        });
    };
    if !requested_sources.microphone {
        return Ok(());
    }

    let resume_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        runtime.inactivity.screen_paused,
        false,
        runtime.inactivity.system_audio_paused,
    )
    .unwrap_or(CaptureSources {
        screen: false,
        microphone: false,
        system_audio: false,
    });
    start_windows_active_segment(
        app_handle,
        runtime,
        &resume_sources,
        "resuming Windows microphone from inactivity",
    )?;
    runtime
        .inactivity
        .set_audio_family_paused_states(false, runtime.inactivity.system_audio_paused);
    refresh_windows_current_segment_sources(runtime);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn resume_system_audio_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_system_audio_paused() {
        return Ok(());
    }
    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming Windows system audio from inactivity".to_string(),
        });
    };
    if !requested_sources.system_audio {
        return Ok(());
    }

    let resume_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        false,
    )
    .unwrap_or(CaptureSources {
        screen: false,
        microphone: false,
        system_audio: false,
    });
    start_windows_active_segment(
        app_handle,
        runtime,
        &resume_sources,
        "resuming Windows system audio from inactivity",
    )?;
    runtime
        .inactivity
        .set_audio_family_paused_states(runtime.inactivity.microphone_paused, false);
    refresh_windows_current_segment_sources(runtime);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn pause_runtime_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_paused {
        return Ok(());
    }

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

    runtime.inactivity.is_paused = true;
    refresh_windows_current_segment_sources(runtime);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn resume_runtime_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_paused {
        return Ok(());
    }

    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming Windows inactivity".to_string(),
        });
    };
    let resume_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        false,
        false,
        false,
    )
    .unwrap_or(CaptureSources {
        screen: false,
        microphone: false,
        system_audio: false,
    });
    start_windows_active_segment(
        app_handle,
        runtime,
        &resume_sources,
        "resuming Windows native capture from inactivity",
    )?;
    runtime.inactivity.set_family_paused_states(false, false, false);
    runtime.current_segment_sources = Some(resume_sources);
    Ok(())
}


/// Windows audio-segment persistence.
///
/// Commits produced microphone Audio Segments to the index but enqueues **no**
/// audio processing jobs yet — capture-and-store only. Passing disabled
/// admissions guarantees `upsert_audio_segment_and_maybe_enqueue_processing`
/// upserts the segment without enqueuing transcription/speaker/system-audio
/// work; a future audio-decode slice will backfill those jobs.
#[cfg(target_os = "windows")]
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

    let infra = std::sync::Arc::clone(&*app_handle.state::<crate::app_infra::AppInfraState>());
    let app_handle = app_handle.clone();
    let persistence = run_native_capture_async("audio-segment-persistence", async move {
        let transcription_admission = ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
        let speaker_admission = ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
        let system_audio_speech_admission =
            ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
        let mut persisted_any = false;
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

    if !runtime.inactivity.is_screen_paused() {
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
    if sources.system_audio && sources.screen && !runtime.inactivity.is_screen_paused() {
        if !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
            // No active screen session — system audio cannot resume without one.
            // Keep pause/source state unchanged so the inactivity system does not
            // lose track of the paused writer.
            return Ok(());
        }

        refresh_runtime_planner_dates(runtime);
        // Always try to seed the planner for real writer resumes so future
        // resumes/rotations preserve the dedicated system-audio session.
        let system_audio_planner = ensure_system_audio_planner_for_runtime(
            runtime,
            "resuming system audio from inactivity",
        )?;

        let planner = system_audio_planner.ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture system-audio planner missing while resuming system audio".to_string(),
        })?;
        let audio_dir = planner.audio_dir();
        std::fs::create_dir_all(&audio_dir).map_err(|error| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to create capture audio directory: {error}"),
        })?;
        let new_system_audio_file = planner
            .system_audio_resume_file(runtime.current_segment_index, super::runtime::now_unix_ms())
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
pub(super) fn resume_screen_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
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
        u32,
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
            let (_, privacy_filter_update) = privacy::collect_privacy_filter_update(
                app_handle,
                privacy::PrivacyRefreshReason::FallbackPoll,
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
    Option<Box<dyn capture_screen::ScreenCaptureSession>>,
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
        u32,
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
    runtime.current_segment_sources = if runtime.privacy_capture_suspension.is_some() {
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
    screen_frame_rate: u32,
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
    screen_frame_rate: u32,
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
        Option<Box<dyn capture_screen::ScreenCaptureSession>>,
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
    let mut active_screen_session: Option<Box<dyn capture_screen::ScreenCaptureSession>> = None;
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
        active_screen_session = Some(Box::new(screen_capture.session));
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

#[cfg(all(test, target_os = "windows"))]
type WindowsMicrophoneStartHook = Box<
    dyn FnMut(
        String,
        Option<String>,
    ) -> Result<Box<dyn microphone_capture::AudioCaptureSession>, CaptureErrorResponse>,
>;

#[cfg(all(test, target_os = "windows"))]
type WindowsSystemAudioStartHook =
    Box<dyn FnMut(String) -> Result<Box<dyn microphone_capture::AudioCaptureSession>, CaptureErrorResponse>>;

#[cfg(all(test, target_os = "windows"))]
type WindowsScreenStartHook = Box<
    dyn FnMut(
        std::path::PathBuf,
        std::path::PathBuf,
    ) -> Result<
        (
            Box<dyn capture_screen::ScreenCaptureSession>,
            String,
            CaptureOutputFiles,
        ),
        CaptureErrorResponse,
    >,
>;

#[cfg(all(test, target_os = "windows"))]
thread_local! {
    static WINDOWS_MICROPHONE_START_HOOK: std::cell::RefCell<Option<WindowsMicrophoneStartHook>> =
        std::cell::RefCell::new(None);
    static WINDOWS_SYSTEM_AUDIO_START_HOOK: std::cell::RefCell<Option<WindowsSystemAudioStartHook>> =
        std::cell::RefCell::new(None);
    static WINDOWS_SCREEN_START_HOOK: std::cell::RefCell<Option<WindowsScreenStartHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(all(test, target_os = "windows"))]
pub(super) struct WindowsStartHookGuard {
    source: WindowsStartHookSource,
}

#[cfg(all(test, target_os = "windows"))]
#[derive(Clone, Copy)]
enum WindowsStartHookSource {
    Microphone,
    SystemAudio,
    Screen,
}

#[cfg(all(test, target_os = "windows"))]
impl Drop for WindowsStartHookGuard {
    fn drop(&mut self) {
        match self.source {
            WindowsStartHookSource::Microphone => {
                WINDOWS_MICROPHONE_START_HOOK.with(|hook| hook.borrow_mut().take());
            }
            WindowsStartHookSource::SystemAudio => {
                WINDOWS_SYSTEM_AUDIO_START_HOOK.with(|hook| hook.borrow_mut().take());
            }
            WindowsStartHookSource::Screen => {
                WINDOWS_SCREEN_START_HOOK.with(|hook| hook.borrow_mut().take());
            }
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
pub(super) fn set_windows_microphone_start_hook_for_test(
    hook: impl FnMut(
            String,
            Option<String>,
        ) -> Result<Box<dyn microphone_capture::AudioCaptureSession>, CaptureErrorResponse>
        + 'static,
) -> WindowsStartHookGuard {
    WINDOWS_MICROPHONE_START_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    WindowsStartHookGuard {
        source: WindowsStartHookSource::Microphone,
    }
}

#[cfg(all(test, target_os = "windows"))]
pub(super) fn set_windows_system_audio_start_hook_for_test(
    hook: impl FnMut(
            String,
        ) -> Result<Box<dyn microphone_capture::AudioCaptureSession>, CaptureErrorResponse>
        + 'static,
) -> WindowsStartHookGuard {
    WINDOWS_SYSTEM_AUDIO_START_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    WindowsStartHookGuard {
        source: WindowsStartHookSource::SystemAudio,
    }
}

#[cfg(all(test, target_os = "windows"))]
pub(super) fn set_windows_screen_start_hook_for_test(
    hook: impl FnMut(
            std::path::PathBuf,
            std::path::PathBuf,
        ) -> Result<
            (
                Box<dyn capture_screen::ScreenCaptureSession>,
                String,
                CaptureOutputFiles,
            ),
            CaptureErrorResponse,
        > + 'static,
) -> WindowsStartHookGuard {
    WINDOWS_SCREEN_START_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    WindowsStartHookGuard {
        source: WindowsStartHookSource::Screen,
    }
}

#[cfg(target_os = "windows")]
fn start_windows_microphone_session_for_file(
    output_file: &str,
    device_id: Option<&str>,
) -> Result<Box<dyn microphone_capture::AudioCaptureSession>, CaptureErrorResponse> {
    #[cfg(test)]
    {
        let hooked = WINDOWS_MICROPHONE_START_HOOK.with(|hook| {
            hook.borrow_mut()
                .as_mut()
                .map(|hook| hook(output_file.to_string(), device_id.map(str::to_string)))
        });
        if let Some(result) = hooked {
            return result;
        }
    }

    microphone_capture::start_wasapi_microphone_capture_session_for_file(output_file, device_id)
        .map(|session| Box::new(session) as Box<dyn microphone_capture::AudioCaptureSession>)
}

#[cfg(target_os = "windows")]
fn start_windows_system_audio_session_for_file(
    output_file: &str,
) -> Result<Box<dyn microphone_capture::AudioCaptureSession>, CaptureErrorResponse> {
    #[cfg(test)]
    {
        let hooked = WINDOWS_SYSTEM_AUDIO_START_HOOK.with(|hook| {
            hook.borrow_mut()
                .as_mut()
                .map(|hook| hook(output_file.to_string()))
        });
        if let Some(result) = hooked {
            return result;
        }
    }

    microphone_capture::start_wasapi_system_audio_capture_session_for_file(output_file)
        .map(|session| Box::new(session) as Box<dyn microphone_capture::AudioCaptureSession>)
}

#[cfg(target_os = "windows")]
#[allow(clippy::too_many_arguments)]
fn start_windows_screen_session_for_segment(
    segment_dir: &Path,
    screen_output_file: &Path,
    sources: &capture_screen::ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &capture_types::ScreenResolution,
    video_bitrate_bps: Option<u32>,
    options: capture_screen::ScreenCaptureSessionOptions,
) -> Result<
    (
        Box<dyn capture_screen::ScreenCaptureSession>,
        String,
        CaptureOutputFiles,
    ),
    CaptureErrorResponse,
> {
    #[cfg(test)]
    {
        let hooked = WINDOWS_SCREEN_START_HOOK.with(|hook| {
            hook.borrow_mut()
                .as_mut()
                .map(|hook| hook(segment_dir.to_path_buf(), screen_output_file.to_path_buf()))
        });
        if let Some(result) = hooked {
            return result;
        }
    }

    let screen_capture = capture_screen::start_capture_session_with_options(
        segment_dir,
        Some(screen_output_file),
        None,
        sources,
        screen_frame_rate,
        screen_resolution,
        video_bitrate_bps,
        options,
    )?;
    Ok((
        Box::new(screen_capture.session),
        screen_capture.recording_file,
        screen_capture.output_files,
    ))
}

#[cfg(target_os = "windows")]
fn rollback_started_windows_active_segment(
    active_screen_session: &mut Option<Box<dyn capture_screen::ScreenCaptureSession>>,
    active_microphone_session: &mut Option<Box<dyn microphone_capture::AudioCaptureSession>>,
    active_system_audio_session: &mut Option<Box<dyn microphone_capture::AudioCaptureSession>>,
    segment_dir: Option<&Path>,
    screen_output_path: Option<&Path>,
    microphone_output_path: Option<&Path>,
    system_audio_output_path: Option<&Path>,
) {
    if let Some(session) = active_microphone_session.as_mut() {
        let _ = session.stop_returning_finalization();
    }
    *active_microphone_session = None;

    if let Some(session) = active_system_audio_session.as_mut() {
        let _ = session.stop_returning_finalization();
    }
    *active_system_audio_session = None;

    if let Err(error) = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: active_screen_session,
        inactivity_tail_trim_seconds: 0,
    }) {
        super::debug_log::log(format!(
            "failed to rollback Windows screen capture session after active segment start failure: [{}] {}",
            error.code, error.message
        ));
    }

    if let Some(path) = screen_output_path {
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
    cleanup_failed_audio_outputs(microphone_output_path, system_audio_output_path);
    if let Some(segment_dir) = segment_dir {
        cleanup_failed_segment_dirs(
            segment_dir,
            microphone_output_path.and_then(Path::parent),
            system_audio_output_path.and_then(Path::parent),
        );
    }
}

#[cfg(target_os = "windows")]
fn windows_audio_family_output_files(
    output_files: Option<&CaptureOutputFiles>,
    microphone: bool,
    system_audio: bool,
) -> Option<CaptureOutputFiles> {
    output_files.map(|outputs| CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: microphone.then(|| outputs.microphone_file.clone()).flatten(),
        microphone_files: microphone
            .then(|| outputs.microphone_files.clone())
            .unwrap_or_default(),
        system_audio_file: system_audio.then(|| outputs.system_audio_file.clone()).flatten(),
        system_audio_files: system_audio
            .then(|| outputs.system_audio_files.clone())
            .unwrap_or_default(),
    })
}

#[cfg(target_os = "windows")]
fn live_windows_microphone_rotation_path(
    runtime: &NativeCaptureRuntime,
    target_index: u64,
    context: &str,
) -> Result<Option<std::path::PathBuf>, CaptureErrorResponse> {
    if runtime.inactivity.is_microphone_paused()
        || runtime.microphone_recording_file.is_none()
        || !runtime
            .active_microphone_session
            .as_ref()
            .is_some_and(|session| session.is_live())
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
    {
        return Ok(None);
    }

    let planner = runtime.microphone_planner.as_ref().ok_or_else(|| CaptureErrorResponse {
        code: "invalid_runtime_state".to_string(),
        message: format!("Capture microphone planner missing while {context}"),
    })?;
    Ok(Some(planner.microphone_file(target_index)))
}

#[cfg(target_os = "windows")]
fn live_windows_system_audio_rotation_path(
    runtime: &NativeCaptureRuntime,
    target_index: u64,
    context: &str,
) -> Result<Option<std::path::PathBuf>, CaptureErrorResponse> {
    if runtime.inactivity.is_system_audio_paused()
        || runtime.system_audio_recording_file.is_none()
        || !runtime
            .active_system_audio_session
            .as_ref()
            .is_some_and(|session| session.is_live())
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.system_audio)
    {
        return Ok(None);
    }

    let planner = runtime.system_audio_planner.as_ref().ok_or_else(|| CaptureErrorResponse {
        code: "invalid_runtime_state".to_string(),
        message: format!("Capture system-audio planner missing while {context}"),
    })?;
    Ok(Some(planner.system_audio_file(target_index)))
}

pub(super) fn start_windows_active_segment(
    app_handle: Option<&tauri::AppHandle>,
    runtime: &mut NativeCaptureRuntime,
    active_sources: &CaptureSources,
    context: &str,
) -> Result<(), CaptureErrorResponse> {
    refresh_runtime_planner_dates(runtime);

    if !has_any_capture_sources(active_sources) {
        runtime.current_segment_sources = Some(active_sources.clone());
        return Ok(());
    }

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let previous_segment_outputs = runtime.current_segment_output_files.clone();
    let mut recording_file = runtime.recording_file.clone();
    let mut microphone_recording_file = runtime.microphone_recording_file.clone();
    let mut system_audio_recording_file = runtime.system_audio_recording_file.clone();
    let mut active_screen_session: Option<Box<dyn capture_screen::ScreenCaptureSession>> = None;
    let mut active_microphone_session: Option<Box<dyn microphone_capture::AudioCaptureSession>> =
        None;
    let mut active_system_audio_session: Option<Box<dyn microphone_capture::AudioCaptureSession>> =
        None;
    let screen_session_live =
        capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref());
    let microphone_session_live = runtime
        .active_microphone_session
        .as_ref()
        .is_some_and(|session| session.is_live());
    let system_audio_session_live = runtime
        .active_system_audio_session
        .as_ref()
        .is_some_and(|session| session.is_live());
    let start_screen = active_sources.screen
        && (!screen_session_live || runtime.recording_file.is_none());
    let start_microphone = active_sources.microphone
        && (!microphone_session_live || runtime.microphone_recording_file.is_none());
    let start_system_audio = active_sources.system_audio
        && (!system_audio_session_live || runtime.system_audio_recording_file.is_none());

    if !start_screen && !start_microphone && !start_system_audio {
        runtime.current_segment_sources = Some(active_sources.clone());
        return Ok(());
    }

    let starts_new_emitted_segment = runtime.current_segment_sources.is_none()
        || (!screen_session_live && !microphone_session_live && !system_audio_session_live);
    let target_index = if starts_new_emitted_segment {
        next_index
    } else {
        runtime.current_segment_index
    };
    if starts_new_emitted_segment {
        if runtime.segment_schedule.is_none() {
            return Err(CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: format!("Capture schedule missing while {context}"),
            });
        }
        if runtime.capture_clock.is_none() {
            return Err(CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: format!("Capture clock missing while {context}"),
            });
        }
    }
    let mut segment_outputs = if starts_new_emitted_segment {
        empty_output_files()
    } else {
        previous_segment_outputs
            .clone()
            .unwrap_or_else(empty_output_files)
    };

    let live_microphone_rotation_path = if starts_new_emitted_segment && !start_microphone {
        live_windows_microphone_rotation_path(runtime, target_index, context)?
    } else {
        None
    };
    let live_system_audio_rotation_path = if starts_new_emitted_segment && !start_system_audio {
        live_windows_system_audio_rotation_path(runtime, target_index, context)?
    } else {
        None
    };
    let screen_plan = if start_screen {
        let planner = runtime.segment_planner.as_ref().ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture screen planner missing while {context}"),
        })?;
        let screen_output_file = if starts_new_emitted_segment {
            planner.segment_screen_output(target_index)
        } else {
            planner.screen_resume_file(target_index, now_unix_ms())
        };
        Some((planner.segment_dir(target_index), screen_output_file))
    } else {
        None
    };
    let microphone_output_path = if start_microphone {
        let planner = runtime
            .microphone_planner
            .as_ref()
            .ok_or_else(|| CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: format!("Capture microphone planner missing while {context}"),
            })?;
        Some(if starts_new_emitted_segment {
            planner.microphone_file(target_index)
        } else {
            planner.microphone_reconnect_file(target_index, now_unix_ms())
        })
    } else {
        None
    };
    let system_audio_output_path = if start_system_audio {
        let planner = runtime
            .system_audio_planner
            .as_ref()
            .ok_or_else(|| CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: format!("Capture system-audio planner missing while {context}"),
            })?;
        Some(if starts_new_emitted_segment {
            planner.system_audio_file(target_index)
        } else {
            planner.system_audio_resume_file(target_index, now_unix_ms())
        })
    } else {
        None
    };

    if let Some((segment_dir, _)) = screen_plan.as_ref() {
        create_segment_output_dirs(
            segment_dir,
            microphone_output_path
                .as_deref()
                .or(live_microphone_rotation_path.as_deref())
                .and_then(Path::parent),
            system_audio_output_path
                .as_deref()
                .or(live_system_audio_rotation_path.as_deref())
                .and_then(Path::parent),
            active_sources,
        )?;
    } else {
        for audio_dir in [
            microphone_output_path
                .as_deref()
                .or(live_microphone_rotation_path.as_deref())
                .and_then(Path::parent),
            system_audio_output_path
                .as_deref()
                .or(live_system_audio_rotation_path.as_deref())
                .and_then(Path::parent),
        ]
        .into_iter()
        .flatten()
        {
            std::fs::create_dir_all(audio_dir).map_err(|error| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!("Failed to create capture audio directory: {error}"),
            })?;
        }
    }

    if start_screen {
        let Some((segment_dir, screen_output_file)) = screen_plan.as_ref() else {
            unreachable!("screen plan exists when screen source is active");
        };
        let screen_sources = capture_screen::ScreenCaptureSources {
            screen: true,
            system_audio: false,
        };
        let metadata_snapshot_provider =
            app_handle.map(metadata::frame_metadata_snapshot_provider);
        let (session, started_recording_file, started_output_files) =
            match start_windows_screen_session_for_segment(
                segment_dir,
                screen_output_file,
                &screen_sources,
                runtime.screen_frame_rate,
                &runtime.screen_resolution,
                runtime.effective_screen_bitrate_bps,
                capture_session_options(
                    runtime.frame_artifact_tx.clone(),
                    metadata_snapshot_provider,
                    0,
                    None,
                ),
            ) {
                Ok(screen_capture) => screen_capture,
                Err(error) => {
                    if error.code != "capture_start_rollback_incomplete" {
                        rollback_started_windows_active_segment(
                            &mut active_screen_session,
                            &mut active_microphone_session,
                            &mut active_system_audio_session,
                            screen_plan.as_ref().map(|(segment_dir, _)| segment_dir.as_path()),
                            screen_plan
                                .as_ref()
                                .map(|(_, screen_output_file)| screen_output_file.as_path()),
                            microphone_output_path.as_deref(),
                            system_audio_output_path.as_deref(),
                        );
                    }
                    return Err(error);
                }
            };
        if let Some(screen_file) = started_output_files.screen_file {
            set_current_screen_output_file(&mut segment_outputs, screen_file);
        }
        recording_file = Some(started_recording_file);
        active_screen_session = Some(session);
    }

    if let Some(path) = live_system_audio_rotation_path.as_ref() {
        let output_file = path.to_string_lossy().to_string();
        let mut previous_system_audio_outputs =
            windows_audio_family_output_files(previous_segment_outputs.as_ref(), false, true);
        let finalization = match runtime
            .active_system_audio_session
            .as_mut()
            .expect("live system-audio rotation path requires an active session")
            .rotate_output_file_returning_finalization(&output_file)
        {
            Ok(finalization) => finalization,
            Err(error) => {
                rollback_started_windows_active_segment(
                    &mut active_screen_session,
                    &mut active_microphone_session,
                    &mut active_system_audio_session,
                    screen_plan.as_ref().map(|(segment_dir, _)| segment_dir.as_path()),
                    screen_plan
                        .as_ref()
                        .map(|(_, screen_output_file)| screen_output_file.as_path()),
                    live_microphone_rotation_path.as_deref(),
                    Some(path.as_path()),
                );
                return Err(error);
            }
        };
        apply_windows_system_audio_output_finalization(
            previous_system_audio_outputs.as_mut(),
            &finalization,
        );
        append_committed_outputs(runtime, previous_system_audio_outputs.as_ref());
        persist_committed_audio_segments(
            app_handle,
            runtime.source_sessions.as_ref(),
            runtime.segment_schedule.as_ref(),
            runtime.current_segment_index,
            previous_system_audio_outputs.as_ref(),
        );
        set_current_system_audio_output_file(&mut segment_outputs, output_file.clone());
        system_audio_recording_file = Some(output_file);
    }

    if let Some(path) = live_microphone_rotation_path.as_ref() {
        let output_file = path.to_string_lossy().to_string();
        let mut previous_microphone_outputs =
            windows_audio_family_output_files(previous_segment_outputs.as_ref(), true, false);
        let finalization = match runtime
            .active_microphone_session
            .as_mut()
            .expect("live microphone rotation path requires an active session")
            .rotate_output_file_returning_finalization(&output_file)
        {
            Ok(finalization) => finalization,
            Err(error) => {
                rollback_started_windows_active_segment(
                    &mut active_screen_session,
                    &mut active_microphone_session,
                    &mut active_system_audio_session,
                    screen_plan.as_ref().map(|(segment_dir, _)| segment_dir.as_path()),
                    screen_plan
                        .as_ref()
                        .map(|(_, screen_output_file)| screen_output_file.as_path()),
                    Some(path.as_path()),
                    live_system_audio_rotation_path.as_deref(),
                );
                return Err(error);
            }
        };
        apply_windows_microphone_output_finalization(
            previous_microphone_outputs.as_mut(),
            &finalization,
        );
        append_committed_outputs(runtime, previous_microphone_outputs.as_ref());
        persist_committed_audio_segments(
            app_handle,
            runtime.source_sessions.as_ref(),
            runtime.segment_schedule.as_ref(),
            runtime.current_segment_index,
            previous_microphone_outputs.as_ref(),
        );
        set_current_microphone_output_file(&mut segment_outputs, output_file.clone());
        microphone_recording_file = Some(output_file);
    }

    if let Some(path) = microphone_output_path.as_ref() {
        let output_file = path.to_string_lossy().to_string();
        let session = match start_windows_microphone_session_for_file(
            &output_file,
            runtime.microphone_device_id_for_capture.as_deref(),
        ) {
            Ok(session) => session,
            Err(error) => {
                rollback_started_windows_active_segment(
                    &mut active_screen_session,
                    &mut active_microphone_session,
                    &mut active_system_audio_session,
                    screen_plan.as_ref().map(|(segment_dir, _)| segment_dir.as_path()),
                    screen_plan.as_ref().map(|(_, screen_output_file)| screen_output_file.as_path()),
                    microphone_output_path.as_deref(),
                    system_audio_output_path.as_deref(),
                );
                return Err(error);
            }
        };
        set_current_microphone_output_file(&mut segment_outputs, output_file.clone());
        microphone_recording_file = Some(output_file);
        active_microphone_session = Some(session);
    }

    if let Some(path) = system_audio_output_path.as_ref() {
        let output_file = path.to_string_lossy().to_string();
        let session = match start_windows_system_audio_session_for_file(&output_file) {
            Ok(session) => session,
            Err(error) => {
                rollback_started_windows_active_segment(
                    &mut active_screen_session,
                    &mut active_microphone_session,
                    &mut active_system_audio_session,
                    screen_plan.as_ref().map(|(segment_dir, _)| segment_dir.as_path()),
                    screen_plan.as_ref().map(|(_, screen_output_file)| screen_output_file.as_path()),
                    microphone_output_path.as_deref(),
                    system_audio_output_path.as_deref(),
                );
                return Err(error);
            }
        };
        set_current_system_audio_output_file(&mut segment_outputs, output_file.clone());
        system_audio_recording_file = Some(output_file);
        active_system_audio_session = Some(session);
    }

    if starts_new_emitted_segment {
        runtime.current_segment_index = target_index;
    }
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.current_segment_sources = Some(active_sources.clone());
    runtime.recording_file = recording_file;
    runtime.microphone_recording_file = microphone_recording_file;
    runtime.system_audio_recording_file = system_audio_recording_file;
    if start_screen {
        runtime.active_screen_session = active_screen_session;
    }
    if start_microphone {
        runtime.active_microphone_session = active_microphone_session;
    }
    if start_system_audio {
        runtime.active_system_audio_session = active_system_audio_session;
    }
    if starts_new_emitted_segment {
        reanchor_active_segment_timing(runtime, context)?;
    }

    Ok(())
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

            if let Some(privacy_filter_update) = privacy_filter_update {
                if runtime.runtime().privacy_capture_suspension.is_some() {
                    // Throttle display-unavailable recovery so we probe for a
                    // returning display at a calm cadence instead of on every 1s
                    // poll. Privacy-filter recovery is left to retry promptly
                    // (it's capped at a few attempts).
                    let suspension_kind = runtime
                        .runtime()
                        .privacy_capture_suspension
                        .as_ref()
                        .map(|suspension| suspension.kind);
                    let throttle_display_recovery = suspension_kind
                        == Some(CaptureSuspensionKind::DisplayUnavailable)
                        && last_suspension_recovery_attempt.elapsed()
                            < DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL;
                    if !throttle_display_recovery {
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

#[cfg(target_os = "windows")]
fn spawn_segment_loop(app_handle: tauri::AppHandle) -> SegmentLoopControl {
    let control = SegmentLoopControl::new();
    let worker_control = control.clone();

    thread::spawn(move || {
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

            let capture_state = app_handle.state::<NativeCaptureState>();
            let mut runtime = match capture_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => break,
            };

            if !runtime.runtime().is_running || worker_control.stop.load(Ordering::Relaxed) {
                break;
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
    fn display_unavailable_suspension_skips_dead_segment_commit_and_stays_running() {
        // A display-unavailable suspension means macOS already tore the screen
        // stream down, so the in-flight segment is unrecoverable. Even with an
        // (otherwise openable) current segment, the suspend path must skip the
        // commit — and must not fail the session — so a screen-only recording can
        // resume automatically when the display returns.
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
                .privacy_capture_suspension
                .as_ref()
                .map(|suspension| suspension.kind),
            Some(CaptureSuspensionKind::DisplayUnavailable)
        );
        // The dead in-flight segment is dropped, not committed.
        assert!(runtime.current_segment_output_files.is_none());
        assert!(runtime.recording_file.is_none());
        let output_files = runtime
            .output_files
            .expect("output files collection should be preserved");
        assert!(
            output_files.screen_file.is_none() && output_files.screen_files.is_empty(),
            "the unrecoverable in-flight segment must not be committed"
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
    let requires_screen_permission = settings.capture_screen
        || (cfg!(target_os = "macos") && settings.capture_system_audio);
    if requires_screen_permission {
        let screen_ok = capture_screen::ensure_screen_permission();
        if !screen_ok {
            return Err(CaptureErrorResponse {
                code: "screen_permission_denied".to_string(),
                message: if cfg!(target_os = "macos") && settings.capture_system_audio {
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
            runtime.privacy_capture_suspension = None;
            apply_runtime_signal(runtime, RuntimeSignal::SourcesReady)?;
            Ok(())
        }

        #[cfg(target_os = "windows")]
        {

            let started = now_unix_ms();
            let recordings_root =
                crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
                    &settings.save_directory,
                )
                .recordings_root();
            std::fs::create_dir_all(&recordings_root).map_err(|error| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!("Failed to create capture recordings directory: {error}"),
            })?;

            let segment_schedule =
                SegmentSchedule::new(Duration::from_secs(settings.segment_duration_seconds));
            let capture_clock = CaptureClock::start_now();
            let effective_screen_bitrate_bps = compute_effective_screen_bitrate_bps(settings);
            let initial_inactivity = super::inactivity::InactivityState::from_recording_settings(
                settings,
                now_monotonic_marker_ms(),
            );

            let screen_session_id = if sources.screen {
                Some(prefixed_capture_id("screen")?)
            } else {
                None
            };
            let microphone_session_id = if sources.microphone {
                Some(prefixed_capture_id("mic")?)
            } else {
                None
            };
            let system_audio_session_id = if sources.system_audio {
                Some(prefixed_capture_id("sysaudio_session")?)
            } else {
                None
            };

            let segment_planner = screen_session_id.as_deref().map(|session_id| {
                SegmentPlanner::new(recordings_root.to_string_lossy().to_string(), session_id)
            });
            let microphone_planner = microphone_session_id.as_deref().map(|session_id| {
                SegmentPlanner::new(recordings_root.to_string_lossy().to_string(), session_id)
            });
            let system_audio_planner = system_audio_session_id.as_deref().map(|session_id| {
                SegmentPlanner::new(recordings_root.to_string_lossy().to_string(), session_id)
            });
            let frame_artifact_tx = screen_session_id
                .as_ref()
                .map(|session_id| spawn_frame_artifact_worker(&app_handle, session_id.clone()));

            let source_sessions = SourceSessions {
                screen: screen_session_id.map(|session_id| SourceSessionMeta {
                    session_id,
                    started_at_unix_ms: started,
                }),
                microphone: microphone_session_id.map(|session_id| SourceSessionMeta {
                    session_id,
                    started_at_unix_ms: started,
                }),
                system_audio: system_audio_session_id.map(|session_id| SourceSessionMeta {
                    session_id,
                    started_at_unix_ms: started,
                }),
            };
            let segment_loop_control = spawn_segment_loop(app_handle.clone());

            runtime.is_running = true;
            runtime.source_sessions = Some(source_sessions);
            runtime.requested_sources = Some(sources.clone());
            runtime.current_segment_sources = None;
            runtime.output_files = Some(empty_output_files());
            runtime.current_segment_output_files = None;
            runtime.current_segment_index = 0;
            runtime.screen_frame_rate = settings.screen_frame_rate;
            runtime.screen_resolution = settings.screen_resolution.clone();
            runtime.effective_screen_bitrate_bps = effective_screen_bitrate_bps;
            runtime.inactivity = initial_inactivity;
            runtime.microphone_device_id_for_capture = microphone_device_id_for_capture;
            runtime.segment_loop_control = Some(segment_loop_control);
            runtime.capture_clock = Some(capture_clock);
            runtime.segment_schedule = Some(segment_schedule);
            runtime.segment_planner = segment_planner;
            runtime.microphone_planner = microphone_planner;
            runtime.system_audio_planner = system_audio_planner;
            runtime.frame_artifact_tx = frame_artifact_tx;
            runtime.recording_file = None;
            runtime.microphone_recording_file = None;
            runtime.system_audio_recording_file = None;
            runtime.active_screen_session = None;
            runtime.active_microphone_session = None;
            runtime.active_system_audio_session = None;
            start_windows_active_segment(
                Some(&app_handle),
                runtime,
                &sources,
                "starting capture runtime",
            )?;
            apply_runtime_signal(runtime, RuntimeSignal::SourcesReady)?;
            Ok(())
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
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

    #[cfg(target_os = "windows")]
    {
        // Tolerate an already-Idle/Stopping controller (e.g. a user pause drove
        // the controller to Idle via StopRequested -> Stopping -> SourcesStopped
        // while keeping `is_running == true`), mirroring the macOS branch. Issuing
        // a bare StopRequested here would hit the invalid `(Idle, StopRequested)`
        // transition and fail the first Stop click while paused.
        request_runtime_stop_transition_if_needed(runtime)?;

        let mut current_segment_output_files = runtime.current_segment_output_files.clone();
        let recording_file = runtime.recording_file.clone();
        let microphone_recording_file = runtime.microphone_recording_file.clone();
        let system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let requested_sources = runtime.requested_sources.clone();

        // Finalize the in-flight microphone segment so its `.m4a` is openable
        // before the WASAPI/MF capture thread is torn down.
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            match session.stop_returning_finalization() {
                Ok(finalization) => apply_windows_microphone_output_finalization(
                    current_segment_output_files.as_mut(),
                    &finalization,
                ),
                Err(error) => {
                    super::debug_log::log(format!(
                        "failed to finalize Windows microphone capture on stop: [{}] {}",
                        error.code, error.message
                    ));
                }
            }
        }
        runtime.active_microphone_session = None;

        if let Some(session) = runtime.active_system_audio_session.as_mut() {
            match session.stop_returning_finalization() {
                Ok(finalization) => apply_windows_system_audio_output_finalization(
                    current_segment_output_files.as_mut(),
                    &finalization,
                ),
                Err(error) => {
                    super::debug_log::log(format!(
                        "failed to finalize Windows system-audio capture on stop: [{}] {}",
                        error.code, error.message
                    ));
                }
            }
        }
        runtime.active_system_audio_session = None;

        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut runtime.active_screen_session,
            inactivity_tail_trim_seconds: 0,
        })?;

        if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
            flush_frame_artifacts(tx);
        }

        // Validate produced audio outputs (openable `.m4a` with positive duration)
        // through the shared injectable validator seam and drop any unusable ones.
        if let Err(error) = finalize_capture_outputs(
            current_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        ) {
            super::debug_log::log(format!(
                "Windows capture output finalization reported an issue on stop: [{}] {}",
                error.code, error.message
            ));
        }

        if let (Some(committed), Some(segment)) = (
            runtime.output_files.as_mut(),
            current_segment_output_files.as_ref(),
        ) {
            append_committed_segment_output_files(committed, segment);
        }

        // Commit produced Audio Segments but do not enqueue processing jobs on
        // Windows yet (capture-and-store only).
        persist_committed_audio_segments(
            app_handle,
            runtime.source_sessions.as_ref(),
            runtime.segment_schedule.as_ref(),
            runtime.current_segment_index,
            current_segment_output_files.as_ref(),
        );

        if runtime.runtime_state == RuntimeState::Stopping {
            apply_runtime_signal(runtime, RuntimeSignal::SourcesStopped)?;
        }
        Ok(())
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = runtime;
        let _ = app_handle;
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

/// Platform-neutral segment scheduling tests.
///
/// These exercise pure scheduling logic (rotation boundaries, contiguous
/// segment numbering, idle sleep cadence) with no OS capture APIs, so they run
/// on every target that has a capture CI lane — currently macOS and Windows.
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod scheduling_tests {
    use super::{next_emitted_segment_index, segment_loop_sleep_duration};
    use crate::native_capture::runtime::should_rotate_segment;
    use capture_runtime::{CaptureClock, SegmentSchedule};
    use std::time::Duration;

    #[test]
    fn should_rotate_segment_only_after_boundary_crossing() {
        assert!(!should_rotate_segment(1, 1));
        assert!(should_rotate_segment(1, 2));
        assert!(should_rotate_segment(3, 5));
    }

    #[test]
    fn rotation_keeps_emitted_segment_numbering_contiguous_when_schedule_jumps_ahead() {
        let scheduled_index = 10;

        assert!(should_rotate_segment(4, scheduled_index));
        assert_eq!(next_emitted_segment_index(4), 5);
    }

    #[test]
    fn segment_loop_sleep_duration_uses_idle_poll_interval_for_zero_duration_schedule() {
        let schedule = SegmentSchedule::new(Duration::ZERO);
        let clock = CaptureClock::start_now();

        assert_eq!(
            segment_loop_sleep_duration(&schedule, &clock),
            Duration::from_secs(1)
        );
    }
}

/// Cross-platform tests for the [`capture_screen::ScreenCaptureSession`] seam.
///
/// They drive the lifecycle helpers (`stop`, `rotate`, liveness, stop-error
/// draining) through a fully in-memory fake session, so the orchestration
/// contract is verified on every platform without a real capture backend.
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod screen_capture_session_seam_tests {
    use super::empty_output_files;
    use capture_screen::{
        rotate_screen_capture_session, screen_capture_session_is_live, stop_screen_capture_session,
        take_screen_capture_session_stop_error, RotateScreenCaptureSessionArgs,
        RotatedCaptureOutputs, ScreenCaptureSession, StopScreenCaptureSessionArgs,
    };
    use capture_types::CaptureErrorResponse;
    use std::path::Path;

    /// In-memory [`ScreenCaptureSession`] for orchestration tests.
    #[derive(Debug)]
    struct FakeScreenCaptureSession {
        live: bool,
        stop_result: Result<(), CaptureErrorResponse>,
        pending_stop_error: Option<CaptureErrorResponse>,
        rotate_recording_file: String,
        stop_calls: u32,
        rotate_calls: u32,
        last_stop_tail_trim_seconds: u64,
    }

    // `Result` has no `Default`, so spell the defaults out instead of deriving.
    impl Default for FakeScreenCaptureSession {
        fn default() -> Self {
            Self {
                live: false,
                stop_result: Ok(()),
                pending_stop_error: None,
                rotate_recording_file: String::new(),
                stop_calls: 0,
                rotate_calls: 0,
                last_stop_tail_trim_seconds: 0,
            }
        }
    }

    impl FakeScreenCaptureSession {
        fn live() -> Self {
            Self {
                live: true,
                rotate_recording_file: "fake-rotated.mov".to_string(),
                ..Self::default()
            }
        }

        fn failing_stop(code: &str) -> Self {
            Self {
                live: true,
                stop_result: Err(CaptureErrorResponse {
                    code: code.to_string(),
                    message: "fake stop failure".to_string(),
                }),
                ..Self::default()
            }
        }

        fn with_pending_stop_error(code: &str) -> Self {
            Self {
                live: false,
                pending_stop_error: Some(CaptureErrorResponse {
                    code: code.to_string(),
                    message: "fake delegate stop error".to_string(),
                }),
                ..Self::default()
            }
        }
    }

    impl ScreenCaptureSession for FakeScreenCaptureSession {
        fn rotate(
            &mut self,
            _segment_dir: &Path,
            _screen_output_file: Option<&Path>,
            _system_audio_output_path: Option<&Path>,
        ) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
            self.rotate_calls += 1;
            Ok(RotatedCaptureOutputs {
                recording_file: self.rotate_recording_file.clone(),
                system_audio_recording_file: None,
                output_files: empty_output_files(),
            })
        }

        fn stop(&mut self, inactivity_tail_trim_seconds: u64) -> Result<(), CaptureErrorResponse> {
            self.stop_calls += 1;
            self.last_stop_tail_trim_seconds = inactivity_tail_trim_seconds;
            self.live = false;
            self.stop_result.clone()
        }

        fn is_live(&self) -> bool {
            self.live
        }

        fn take_stop_error(&mut self) -> Option<CaptureErrorResponse> {
            self.pending_stop_error.take()
        }

        fn supports_frame_export(&self) -> bool {
            false
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn boxed(session: FakeScreenCaptureSession) -> Option<Box<dyn ScreenCaptureSession>> {
        Some(Box::new(session))
    }

    #[test]
    fn liveness_reflects_underlying_session() {
        assert!(screen_capture_session_is_live(
            boxed(FakeScreenCaptureSession::live()).as_ref()
        ));
        assert!(!screen_capture_session_is_live(
            boxed(FakeScreenCaptureSession::default()).as_ref()
        ));
        assert!(!screen_capture_session_is_live(None));
    }

    #[test]
    fn successful_stop_clears_the_session() {
        let mut session = boxed(FakeScreenCaptureSession::live());

        let result = stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut session,
            inactivity_tail_trim_seconds: 7,
        });

        assert!(result.is_ok());
        assert!(session.is_none(), "a clean stop detaches the session");
    }

    #[test]
    fn failed_stop_still_clears_the_session_and_surfaces_the_error() {
        let mut session = boxed(FakeScreenCaptureSession::failing_stop("fake_stop_failed"));

        let result = stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut session,
            inactivity_tail_trim_seconds: 0,
        });

        assert_eq!(
            result.err().map(|error| error.code),
            Some("fake_stop_failed".to_string())
        );
        assert!(session.is_none());
    }

    #[test]
    fn stop_error_is_drained_once() {
        let mut session = boxed(FakeScreenCaptureSession::with_pending_stop_error(
            "fake_delegate",
        ));

        let first = take_screen_capture_session_stop_error(session.as_mut());
        assert_eq!(
            first.map(|error| error.code),
            Some("fake_delegate".to_string())
        );

        let second = take_screen_capture_session_stop_error(session.as_mut());
        assert!(second.is_none(), "the stop error is cleared after draining");
    }

    #[test]
    fn rotate_returns_backend_outputs() {
        let mut session = boxed(FakeScreenCaptureSession::live());

        let outputs = rotate_screen_capture_session(RotateScreenCaptureSessionArgs {
            active_session: &mut session,
            segment_dir: Path::new("/tmp/segment"),
            screen_output_file: None,
            system_audio_output_path: None,
        })
        .expect("rotation should succeed");

        assert_eq!(outputs.recording_file, "fake-rotated.mov");
    }

    #[test]
    fn rotate_without_active_session_is_invalid_state() {
        let mut session: Option<Box<dyn ScreenCaptureSession>> = None;

        let result = rotate_screen_capture_session(RotateScreenCaptureSessionArgs {
            active_session: &mut session,
            segment_dir: Path::new("/tmp/segment"),
            screen_output_file: None,
            system_audio_output_path: None,
        });

        // `RotatedCaptureOutputs` is intentionally not `Debug`, so match instead
        // of `expect_err`.
        let error = match result {
            Ok(_) => panic!("rotation without a session should error"),
            Err(error) => error,
        };
        assert_eq!(error.code, "invalid_runtime_state");
    }
}
