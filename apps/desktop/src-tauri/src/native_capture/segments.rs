use super::output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
use super::settings::compute_effective_screen_bitrate_bps;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::Manager;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc;

use super::emit_audio_segments_changed;
use super::lifecycle::TickOutcome;
use super::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal,
    ensure_microphone_planner_for_runtime, ensure_system_audio_planner_for_runtime,
    mark_runtime_session_failed, now_monotonic_marker_ms, now_unix_ms, prefixed_capture_id,
    refresh_runtime_planner_dates, reset_runtime_after_start_error, screen_planner_for_runtime,
    should_recover_from_segment_finalize_error, NativeCaptureRuntime, SegmentLoopControl,
};
use super::NativeCaptureState;

// Keep frame artifact persistence off the capture callback thread while bounding
// in-memory buffering. Backpressure is applied on a dedicated worker thread so
// exported frame artifacts are not dropped and the synchronous callback stays
// non-blocking.
const FRAME_ARTIFACT_BUFFER_CAPACITY: usize = 64;
const SEGMENT_LOOP_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const PRIVACY_FILTER_POLL_INTERVAL: Duration = Duration::from_secs(1);

type FrameMetadataSnapshotProvider =
    Arc<dyn Fn() -> Option<capture_metadata::FrameMetadataSnapshot> + Send + Sync + 'static>;

fn frame_metadata_snapshot_provider(
    app_handle: &tauri::AppHandle,
) -> FrameMetadataSnapshotProvider {
    let app_handle = app_handle.clone();
    Arc::new(move || {
        crate::native_capture::metadata::latest_frame_metadata_snapshot(
            app_handle
                .state::<crate::native_capture::CaptureMetadataState>()
                .inner(),
        )
    })
}

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
    metadata_snapshot_provider: Option<FrameMetadataSnapshotProvider>,
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

fn privacy_filter_from_decision(
    decision: capture_metadata::PrivacyFilterDecision,
) -> Option<capture_screen::PrivacyContentFilter> {
    decision
        .privacy_filter_applied
        .then_some(capture_screen::PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: decision.excluded_bundle_ids,
            excluded_window_ids: decision.excluded_window_ids,
        })
}

#[cfg(target_os = "macos")]
struct PrivacyFilterUpdate {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: capture_screen::PrivacyContentFilter,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct InitialPrivacyFilter {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
}

#[cfg(target_os = "macos")]
fn empty_privacy_filter() -> capture_screen::PrivacyContentFilter {
    capture_screen::PrivacyContentFilter {
        display_id: 0,
        excluded_bundle_ids: Vec::new(),
        excluded_window_ids: Vec::new(),
    }
}

#[cfg(target_os = "macos")]
fn collect_initial_privacy_filter(app_handle: &tauri::AppHandle) -> InitialPrivacyFilter {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = crate::native_capture::metadata::refresh_metadata_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.metadata,
        &settings.privacy,
    );
    let filter = privacy_filter_from_decision(decision.clone());
    InitialPrivacyFilter { decision, filter }
}

#[cfg(target_os = "macos")]
fn mark_privacy_decision_applied(
    app_handle: &tauri::AppHandle,
    decision: capture_metadata::PrivacyFilterDecision,
) {
    crate::native_capture::metadata::mark_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        decision,
    );
}

#[cfg(target_os = "macos")]
fn collect_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
    let current = collect_initial_privacy_filter(app_handle);
    let filter = current.filter.unwrap_or_else(empty_privacy_filter);
    PrivacyFilterUpdate {
        decision: current.decision,
        filter,
    }
}

#[cfg(target_os = "macos")]
fn apply_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
    update: PrivacyFilterUpdate,
) -> Result<(), CaptureErrorResponse> {
    if !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        return Ok(());
    }
    capture_screen::update_active_privacy_filter(&mut runtime.active_screen_session, update.filter)
        .map_err(|error| CaptureErrorResponse {
            code: "privacy_filter_apply_failed".to_string(),
            message: error.message,
        })?;
    mark_privacy_decision_applied(app_handle, update.decision);
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

    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
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

    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
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

    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
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

    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
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
            active_sources_for_inactivity_paused_state(
                sources,
                true,
                runtime.inactivity.microphone_paused,
                runtime.inactivity.system_audio_paused,
            )
        });

        runtime.inactivity.set_family_paused_states(
            true,
            runtime.inactivity.microphone_paused,
            runtime.inactivity.system_audio_paused,
        );

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
                    active_sources_for_inactivity_paused_state(
                        sources,
                        true, // screen is now stopped
                        runtime.inactivity.microphone_paused,
                        runtime.inactivity.system_audio_paused,
                    )
                });
                runtime.inactivity.set_family_paused_states(
                    true,
                    runtime.inactivity.microphone_paused,
                    runtime.inactivity.system_audio_paused,
                );
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
                    active_sources_for_inactivity_paused_state(
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
                runtime.inactivity.set_family_paused_states(
                    true,
                    runtime.inactivity.microphone_paused,
                    runtime.inactivity.system_audio_paused,
                );
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
    }

    runtime.recording_file = None;
    runtime.system_audio_recording_file = None;

    // Recompute current_segment_sources: if audio is still active, the
    // audio-only subset becomes the active set; otherwise clear it.
    runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
        active_sources_for_inactivity_paused_state(
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

    runtime.inactivity.set_family_paused_states(
        true,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );

    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn resume_screen_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(), CaptureErrorResponse> {
    let tail_trim_seconds = runtime.inactivity.idle_timeout_seconds;
    let microphone_activity_threshold = runtime.inactivity.microphone_activity_threshold();
    let microphone_tail_activity_mode = microphone_tail_trim_activity_mode_for_runtime(runtime);
    let metadata_snapshot_provider = app_handle.map(frame_metadata_snapshot_provider);
    let initial_privacy_filter = app_handle.map(collect_initial_privacy_filter);
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
                    .and_then(|initial| initial.filter.clone()),
            )?;
            if let (Some(app_handle), Some(initial)) = (app_handle, initial_privacy_filter) {
                mark_privacy_decision_applied(app_handle, initial.decision);
            }
            Ok(started_segment)
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
            let privacy_filter_update = collect_privacy_filter_update(app_handle);
            apply_privacy_filter_update(app_handle, runtime, privacy_filter_update)?;
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
        active_sources_for_inactivity_paused_state(sources, true, true, sources.system_audio)
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
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
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
    let metadata_snapshot_provider = app_handle.map(frame_metadata_snapshot_provider);
    let initial_privacy_filter = app_handle.map(collect_initial_privacy_filter);
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
                    .and_then(|initial| initial.filter.clone()),
            )?;
            if let (Some(app_handle), Some(initial)) = (app_handle, initial_privacy_filter) {
                mark_privacy_decision_applied(app_handle, initial.decision);
            }
            Ok(started_segment)
        },
    )
}

#[cfg(target_os = "macos")]
pub(super) fn resume_runtime_from_inactivity_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
    _start_segment_fn: F,
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
    runtime.current_segment_sources = sources.clone().into();

    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn resume_runtime_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    let tail_trim_seconds = runtime.inactivity.idle_timeout_seconds;
    let microphone_activity_threshold = runtime.inactivity.microphone_activity_threshold();
    let microphone_tail_activity_mode = microphone_tail_trim_activity_mode_for_runtime(runtime);
    resume_runtime_from_inactivity_with_start_segment(
        runtime,
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
            start_segment_with_inactivity_tail_trim_seconds(
                segment_dir,
                screen_output_file,
                system_audio_output_path,
                sources,
                screen_frame_rate,
                screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id,
                frame_artifact_tx,
                None,
                microphone_output_path,
                tail_trim_seconds,
                microphone_activity_threshold,
                microphone_tail_activity_mode,
                None,
            )
        },
    )
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
    let initial_privacy_filter = collect_initial_privacy_filter(app_handle);
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
        Some(frame_metadata_snapshot_provider(app_handle)),
        microphone_output_path,
        0,
        0.0,
        microphone_capture::MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        initial_privacy_filter.filter,
    )?;
    mark_privacy_decision_applied(app_handle, initial_privacy_filter.decision);
    Ok(started_segment)
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
    metadata_snapshot_provider: Option<FrameMetadataSnapshotProvider>,
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
    ))
}

#[cfg(target_os = "macos")]
fn spawn_segment_loop(app_handle: tauri::AppHandle) -> SegmentLoopControl {
    let control = SegmentLoopControl {
        stop: Arc::new(AtomicBool::new(false)),
    };
    let stop = control.stop.clone();

    thread::spawn(move || {
        let mut last_privacy_filter_poll = std::time::Instant::now() - PRIVACY_FILTER_POLL_INTERVAL;
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
                thread::sleep(sleep_duration);
            }

            if stop.load(Ordering::Relaxed) {
                break;
            }

            let privacy_filter_update =
                if last_privacy_filter_poll.elapsed() >= PRIVACY_FILTER_POLL_INTERVAL {
                    last_privacy_filter_poll = std::time::Instant::now();
                    Some(collect_privacy_filter_update(&app_handle))
                } else {
                    None
                };

            let capture_state = app_handle.state::<NativeCaptureState>();
            let mut runtime = match capture_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => break,
            };

            if !runtime.runtime().is_running || stop.load(Ordering::Relaxed) {
                break;
            }

            if let Some(privacy_filter_update) = privacy_filter_update {
                if let Err(error) = apply_privacy_filter_update(
                    &app_handle,
                    runtime.runtime_mut(),
                    privacy_filter_update,
                ) {
                    super::debug_log::log(format!(
                "privacy filter update failed; disabling screen/system-audio capture: [{}] {}",
                error.code, error.message
            ));
                    let active_session = &mut runtime.runtime_mut().active_screen_session;
                    let _ =
                        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                            active_session,
                            inactivity_tail_trim_seconds: 0,
                        });
                    runtime.runtime_mut().recording_file = None;
                    runtime.runtime_mut().system_audio_recording_file = None;
                    if let Some(current) = runtime.runtime_mut().current_segment_sources.as_mut() {
                        current.screen = false;
                        current.system_audio = false;
                    }
                    if !runtime
                        .runtime()
                        .requested_sources
                        .as_ref()
                        .is_some_and(|sources| sources.microphone)
                    {
                        break;
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
    });

    control
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
            let initial_privacy_decision =
                crate::native_capture::metadata::initial_privacy_decision(
                    app_handle
                        .state::<crate::native_capture::CaptureMetadataState>()
                        .inner(),
                    &settings.metadata,
                    &settings.privacy,
                );
            let initial_privacy_filter =
                privacy_filter_from_decision(initial_privacy_decision.clone());

            let (
                segment_outputs,
                recording_file,
                microphone_recording_file,
                system_audio_recording_file,
                active_screen_session,
                active_microphone_session,
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
                Some(frame_metadata_snapshot_provider(&app_handle)),
                first_microphone_output_path.as_deref(),
                inactivity_tail_trim_seconds,
                microphone_activity_threshold,
                microphone_tail_activity_mode,
                initial_privacy_filter,
            )?;
            crate::native_capture::metadata::mark_applied_privacy_decision(
                app_handle
                    .state::<crate::native_capture::CaptureMetadataState>()
                    .inner(),
                initial_privacy_decision,
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
        if runtime.is_running {
            apply_runtime_signal(runtime, RuntimeSignal::StopRequested)?;
        }

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
