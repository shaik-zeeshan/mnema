use crate::native_capture_output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    clear_current_microphone_output_file, finalize_capture_outputs,
    set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
use crate::native_capture_settings::compute_effective_screen_bitrate_bps;
use capture_microphone as microphone_capture;
use capture_runtime::{
    CaptureClock, RuntimeController, RuntimeSignal, RuntimeState, SegmentPlanner, SegmentSchedule,
};
#[cfg(target_os = "macos")]
use capture_screen::RotateScreenCaptureSessionArgs;
use capture_screen::StopScreenCaptureSessionArgs;
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CaptureSources, RecordingSettings, SourceSessionMeta,
    SourceSessions,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::Manager;
use tokio::sync::mpsc;

use super::activity::current_activity_snapshot;
use super::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal,
    ensure_microphone_planner_for_runtime, ensure_system_audio_planner_for_runtime,
    has_any_capture_sources, mark_runtime_session_failed, microphone_planner_for_runtime,
    now_monotonic_marker_ms, now_unix_ms, reset_runtime_after_start_error,
    screen_planner_for_runtime, should_recover_from_segment_finalize_error, should_rotate_segment,
    system_audio_planner_for_runtime, NativeCaptureRuntime, NativeCaptureState, SegmentLoopControl,
};

// Keep frame artifact persistence off the capture callback thread while bounding
// in-memory buffering. Backpressure is applied on a dedicated worker thread so
// exported frame artifacts are not dropped and the synchronous callback stays
// non-blocking.
const FRAME_ARTIFACT_BUFFER_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameArtifactForwardingResult {
    Enqueued,
    ReceiverClosed,
}

pub(crate) enum FrameArtifactMessage {
    Artifact(capture_screen::ScreenFrameArtifact),
    Flush(std::sync::mpsc::SyncSender<()>),
}

#[cfg(test)]
impl FrameArtifactMessage {
    pub(super) fn unwrap_artifact(self) -> capture_screen::ScreenFrameArtifact {
        match self {
            Self::Artifact(artifact) => artifact,
            Self::Flush(_) => panic!("expected Artifact, got Flush"),
        }
    }
}

pub(super) fn try_forward_frame_artifact(
    frame_artifact_tx: &mpsc::Sender<FrameArtifactMessage>,
    artifact: capture_screen::ScreenFrameArtifact,
) -> FrameArtifactForwardingResult {
    match frame_artifact_tx.blocking_send(FrameArtifactMessage::Artifact(artifact)) {
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
fn stop_active_sessions_after_failure(runtime: &mut NativeCaptureRuntime) {
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let _ = session.stop();
    }
    runtime.active_microphone_session = None;

    let _ = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
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
            crate::native_capture_debug_log::log(format!(
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
                crate::native_capture_debug_log::log(format!(
                    "failed removing unusable capture output file {}: {}",
                    path.display(),
                    error
                ));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn create_segment_output_dirs(
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

fn empty_output_files() -> CaptureOutputFiles {
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
fn recover_from_segment_finalize_error(
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
    crate::native_capture_debug_log::log(format!(
        "recovered native capture segment finalization failure while {context}: [{}] {}",
        error.code, error.message
    ));

    true
}

fn frame_export_options(
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
) -> capture_screen::ScreenCaptureSessionOptions {
    let Some(frame_artifact_tx) = frame_artifact_tx else {
        return capture_screen::ScreenCaptureSessionOptions::default();
    };

    if !capture_screen::supports_frame_export() {
        return capture_screen::ScreenCaptureSessionOptions::default();
    }

    capture_screen::ScreenCaptureSessionOptions {
        frame_export: Some(capture_screen::ScreenFrameExportConfig {
            on_frame_exported: Arc::new(move |artifact| {
                match try_forward_frame_artifact(&frame_artifact_tx, artifact) {
                    FrameArtifactForwardingResult::Enqueued => {}
                    FrameArtifactForwardingResult::ReceiverClosed => {
                        crate::native_capture_debug_log::log(
                            "failed to forward native frame artifact for persistence: worker channel closed",
                        );
                    }
                }
            }),
        }),
    }
}

fn spawn_frame_artifact_worker(
    app_handle: &tauri::AppHandle,
    session_id: String,
) -> mpsc::Sender<FrameArtifactMessage> {
    let (tx, mut rx) = mpsc::channel(FRAME_ARTIFACT_BUFFER_CAPACITY);
    let infra = Arc::clone(&*app_handle.state::<crate::app_infra::AppInfraState>());

    tauri::async_runtime::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                FrameArtifactMessage::Artifact(artifact) => {
                    if let Err(error) = crate::app_infra::persist_screen_frame_artifact(
                        infra.as_ref(),
                        &session_id,
                        artifact,
                    )
                    .await
                    {
                        crate::native_capture_debug_log::log(format!(
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
pub(super) fn pause_microphone_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
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

    if let Some(session) = runtime.active_microphone_session.as_mut() {
        if let Err(mic_err) = session.stop() {
            return Err(mic_err);
        }
    }
    runtime.active_microphone_session = None;

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

#[cfg(target_os = "macos")]
pub(super) fn pause_system_audio_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
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
        if runtime.active_screen_session.is_some() {
            // Soft-pause: tell the screen backend to finalize and detach its
            // system-audio writer without stopping/restarting the screen session.
            capture_screen::pause_system_audio_writer(&mut runtime.active_screen_session)?;

            // The finished system-audio file is already tracked in
            // system_audio_files via set_current_system_audio_output_file.
            // Clear only the "current" pointer so a future resume gets a
            // fresh path, but preserve the finished files list.
            if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                output_files.system_audio_file = None;
            }
            runtime.system_audio_recording_file = None;
        } else {
            // No active screen session backend (e.g. tests/headless) — just
            // reconcile bookkeeping without attempting a pause that would
            // fail in the native capture stack.
            if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                output_files.system_audio_file = None;
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

    ensure_microphone_planner_for_runtime(runtime, "resuming microphone from inactivity")?;

    if sources.microphone && runtime.microphone_planner.is_some() {
        let microphone_recording_file =
            super::microphone::next_microphone_output_file_for_runtime(runtime)?;
        let mic_start =
            microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
                &microphone_recording_file,
                runtime.microphone_device_id_for_capture.as_deref(),
            );

        match mic_start {
            Ok(session) => {
                runtime.active_microphone_session = Some(session);
                runtime.microphone_recording_file = Some(microphone_recording_file.clone());
                if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                    crate::native_capture_output::set_current_microphone_output_file(
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
        if runtime.active_screen_session.is_none() {
            // No active screen session — system audio cannot resume without one.
            // Keep pause/source state unchanged so the inactivity system does not
            // lose track of the paused writer.
            return Ok(());
        }

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
pub(super) fn pause_screen_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_screen_paused() {
        return Ok(());
    }

    let mut current_segment_output_files = runtime.current_segment_output_files.clone();
    let recording_file = runtime.recording_file.clone();
    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let system_audio_recording_file = runtime.system_audio_recording_file.clone();
    let requested_sources = runtime.requested_sources.clone();
    let mut segment_committed = false;

    let screen_finalize_recovered =
        match capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut runtime.active_screen_session,
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
) -> Result<(), CaptureErrorResponse> {
    resume_screen_from_inactivity_with_start_segment(runtime, start_segment)
}

#[cfg(target_os = "macos")]
pub(super) fn resume_screen_from_inactivity_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
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

    // Start only screen-family sources; audio sessions remain untouched.
    // When audio is paused, system_audio must also be suppressed since it
    // belongs to the audio family even though the capture backend shares the
    // screen session.
    let screen_only_sources = CaptureSources {
        screen: sources.screen,
        microphone: false,
        system_audio: sources.system_audio && !runtime.inactivity.is_system_audio_paused(),
    };

    let system_audio_planner = if screen_only_sources.system_audio {
        ensure_system_audio_planner_for_runtime(runtime, "resuming screen from inactivity")?
    } else {
        None
    };

    let scheduled_index = schedule.current_segment_index(clock.elapsed());
    let next_index = (runtime.current_segment_index + 1).max(scheduled_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);
    let system_audio_output_path = screen_only_sources
        .system_audio
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

    let (
        segment_outputs,
        recording_file,
        _microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        _active_microphone_session,
    ) = started_segment;

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
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    // Do not touch active_microphone_session or microphone_recording_file

    runtime.inactivity.set_family_paused_states(
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );

    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn pause_runtime_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_paused {
        return Ok(());
    }

    let mut current_segment_output_files = runtime.current_segment_output_files.clone();
    let recording_file = runtime.recording_file.clone();
    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let system_audio_recording_file = runtime.system_audio_recording_file.clone();
    let requested_sources = runtime.requested_sources.clone();
    let mut segment_committed = false;

    if let Some(session) = runtime.active_microphone_session.as_mut() {
        if let Err(error) = session.stop() {
            // Microphone stop failed but the screen session is still live.
            // Only clear microphone-related bookkeeping; preserve screen
            // segment state so the still-running screen capture remains
            // trackable by stop/rotation/finalization paths.
            runtime.active_microphone_session = None;
            runtime.microphone_recording_file = None;
            if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
                clear_current_microphone_output_file(output_files);
            }
            // Update current_segment_sources to reflect mic is gone but
            // screen (and possibly system_audio) are still live.
            if let Some(sources) = runtime.current_segment_sources.as_mut() {
                sources.microphone = false;
            }
            return Err(error);
        }
    }
    runtime.active_microphone_session = None;

    let screen_finalize_recovered =
        match capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut runtime.active_screen_session,
        }) {
            Ok(()) => false,
            Err(error)
                if recover_from_segment_finalize_error(
                    "pausing for inactivity",
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
                // Screen session is already stopped; reconcile bookkeeping.
                runtime.current_segment_output_files = None;
                runtime.current_segment_sources = None;
                runtime.recording_file = None;
                runtime.microphone_recording_file = None;
                runtime.system_audio_recording_file = None;
                runtime.inactivity.is_paused = true;
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
                    "pausing for inactivity",
                    &error,
                    current_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                ) => {}
            Err(error) => {
                // Finalization failed fatally; reconcile bookkeeping.
                runtime.current_segment_output_files = None;
                runtime.current_segment_sources = None;
                runtime.recording_file = None;
                runtime.microphone_recording_file = None;
                runtime.system_audio_recording_file = None;
                runtime.inactivity.is_paused = true;
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
    }

    runtime.current_segment_output_files = None;
    runtime.current_segment_sources = None;
    runtime.recording_file = None;
    runtime.microphone_recording_file = None;
    runtime.system_audio_recording_file = None;
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
fn apply_resumed_segment_state(
    runtime: &mut NativeCaptureRuntime,
    next_index: u64,
    resumed_sources: CaptureSources,
    started_segment: StartedSegmentState,
) {
    let (
        segment_outputs,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        active_microphone_session,
    ) = started_segment;

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.current_segment_sources =
        has_any_capture_sources(&resumed_sources).then_some(resumed_sources);
    runtime.recording_file = recording_file;
    runtime.microphone_recording_file = microphone_recording_file;
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    runtime.active_microphone_session = active_microphone_session;
    runtime.inactivity.is_paused = false;
}

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
pub(super) fn resume_runtime_from_inactivity_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
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
    if !runtime.inactivity.is_paused {
        return Ok(());
    }

    let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture screen planner missing while resuming inactivity".to_string(),
        });
    };
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

    let Some(schedule) = runtime.segment_schedule.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture schedule missing while resuming inactivity".to_string(),
        });
    };
    let Some(clock) = runtime.capture_clock.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture clock missing while resuming inactivity".to_string(),
        });
    };
    let resumed_sources = active_sources_for_inactivity_paused_state(&sources, false, false, false)
        .ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "No capture sources available while resuming inactivity".to_string(),
        })?;
    let microphone_planner = if resumed_sources.microphone {
        ensure_microphone_planner_for_runtime(runtime, "resuming inactivity")?
    } else {
        None
    };
    let system_audio_planner = if resumed_sources.system_audio {
        ensure_system_audio_planner_for_runtime(runtime, "resuming inactivity")?
    } else {
        None
    };

    let scheduled_index = schedule.current_segment_index(clock.elapsed());
    let next_index = (runtime.current_segment_index + 1).max(scheduled_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = resumed_sources
        .screen
        .then(|| screen_planner.segment_screen_output(next_index));
    let system_audio_output_path = resumed_sources
        .system_audio
        .then(|| {
            system_audio_planner
                .as_ref()
                .map(|planner| planner.system_audio_file(next_index))
        })
        .flatten();
    let microphone_output_path = resumed_sources
        .microphone
        .then(|| {
            microphone_planner
                .as_ref()
                .map(|planner| planner.microphone_file(next_index))
        })
        .flatten();

    let started_segment = start_segment_fn(
        &segment_dir,
        screen_output_file.as_deref(),
        system_audio_output_path.as_deref(),
        &resumed_sources,
        runtime.screen_frame_rate,
        &runtime.screen_resolution,
        runtime.effective_screen_bitrate_bps,
        runtime.microphone_device_id_for_capture.as_deref(),
        runtime.frame_artifact_tx.clone(),
        microphone_output_path.as_deref(),
    )?;

    apply_resumed_segment_state(runtime, next_index, resumed_sources, started_segment);

    Ok(())
}

#[cfg(target_os = "macos")]
fn resume_runtime_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    resume_runtime_from_inactivity_with_start_segment(runtime, start_segment)
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
        crate::native_capture_debug_log::log(format!(
            "fatal native capture inactivity resume failure: [{}] {}",
            error.code, error.message
        ));
        mark_runtime_session_failed(runtime);
        return true;
    }

    crate::native_capture_debug_log::log(format!(
        "failed to resume native capture after activity; keeping session paused for retry: [{}] {}",
        error.code, error.message
    ));

    false
}

#[cfg(target_os = "macos")]
fn start_segment(
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
        let screen_capture = match capture_screen::start_capture_session_with_options(
            session_dir,
            screen_output_file,
            system_audio_output_path,
            &screen_sources,
            screen_frame_rate,
            screen_resolution,
            effective_screen_bitrate_bps,
            frame_export_options(frame_artifact_tx),
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
            microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
                &microphone_output_file,
                microphone_device_id,
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

    thread::spawn(move || loop {
        let sleep_duration = {
            let capture_state = app_handle.state::<NativeCaptureState>();
            let runtime = match capture_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => break,
            };

            if !runtime.is_running {
                break;
            }

            let Some(schedule) = runtime.segment_schedule.as_ref() else {
                break;
            };
            let Some(clock) = runtime.capture_clock.as_ref() else {
                break;
            };

            let until_boundary = schedule.sleep_until_next_boundary(clock);
            until_boundary.min(Duration::from_secs(1))
        };

        if !sleep_duration.is_zero() {
            thread::sleep(sleep_duration);
        }

        if stop.load(Ordering::Relaxed) {
            break;
        }

        let capture_state = app_handle.state::<NativeCaptureState>();
        let mut runtime = match capture_state.lock() {
            Ok(runtime) => runtime,
            Err(_) => break,
        };

        if !runtime.is_running || stop.load(Ordering::Relaxed) {
            break;
        }

        let now = now_monotonic_marker_ms();
        let activity_snapshot = current_activity_snapshot(&runtime);
        let effective_idle = runtime
            .inactivity
            .effective_idle_for_snapshot(now, activity_snapshot);

        // --- Microphone inactivity: pause/resume microphone independently ---
        if runtime
            .inactivity
            .should_resume_microphone_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_microphone_from_inactivity(&mut runtime) {
                crate::native_capture_debug_log::log(format!(
                    "failed to resume microphone capture after activity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                crate::native_capture_debug_log::log(format!(
                    "resumed microphone capture after activity (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
                    mic_eval.effective_idle.idle_ms,
                    mic_eval.effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if runtime
            .inactivity
            .should_pause_microphone_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) = pause_microphone_for_inactivity(&mut runtime) {
                crate::native_capture_debug_log::log(format!(
                    "failed to pause microphone capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                crate::native_capture_debug_log::log(format!(
                    "paused microphone capture for inactivity threshold crossing (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
                    mic_eval.effective_idle.idle_ms,
                    mic_eval.effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        // --- System audio inactivity: pause/resume system audio independently ---
        if runtime
            .inactivity
            .should_resume_system_audio_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_system_audio_from_inactivity(&mut runtime) {
                crate::native_capture_debug_log::log(format!(
                    "failed to resume system audio capture after activity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                crate::native_capture_debug_log::log(format!(
                    "resumed system audio capture after activity (system_audio_effective_idle_ms={}, system_audio_effective_source={}, idle_timeout_seconds={})",
                    sa_eval.effective_idle.idle_ms,
                    sa_eval.effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if runtime
            .inactivity
            .should_pause_system_audio_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) = pause_system_audio_for_inactivity(&mut runtime) {
                crate::native_capture_debug_log::log(format!(
                    "failed to pause system audio capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                crate::native_capture_debug_log::log(format!(
                    "paused system audio capture for inactivity threshold crossing (system_audio_effective_idle_ms={}, system_audio_effective_source={}, idle_timeout_seconds={})",
                    sa_eval.effective_idle.idle_ms,
                    sa_eval.effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        // --- Screen-family inactivity: pause/resume only screen/system-audio ---
        if runtime
            .inactivity
            .should_resume_screen_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_screen_from_inactivity(&mut runtime) {
                if handle_inactivity_resume_error(&mut runtime, error) {
                    break;
                }
            } else {
                let screen_eval = runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                crate::native_capture_debug_log::log(format!(
                    "resumed screen capture after activity (screen_effective_idle_ms={}, screen_effective_source={}, idle_timeout_seconds={})",
                    screen_eval.effective_idle.idle_ms,
                    screen_eval.effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if runtime
            .inactivity
            .should_pause_screen_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) = pause_screen_for_inactivity(&mut runtime) {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            } else {
                let screen_eval = runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                crate::native_capture_debug_log::log(format!(
                    "paused screen capture for inactivity threshold crossing (screen_effective_idle_ms={}, screen_effective_source={}, idle_timeout_seconds={})",
                    screen_eval.effective_idle.idle_ms,
                    screen_eval.effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        // --- Legacy shared inactivity: pause/resume all sources ---
        if runtime
            .inactivity
            .should_resume_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_runtime_from_inactivity(&mut runtime) {
                if handle_inactivity_resume_error(&mut runtime, error) {
                    break;
                }
            } else {
                crate::native_capture_debug_log::log(format!(
                    "resumed native capture after activity (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                    effective_idle.idle_ms,
                    effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if runtime
            .inactivity
            .should_pause_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) = pause_runtime_for_inactivity(&mut runtime) {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            } else {
                crate::native_capture_debug_log::log(format!(
                    "paused native capture for inactivity threshold crossing (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                    effective_idle.idle_ms,
                    effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                ));
            }
            continue;
        }

        if runtime.inactivity.is_paused {
            continue;
        }

        let mut previous_segment_output_files = runtime.current_segment_output_files.clone();
        let recording_file = runtime.recording_file.clone();
        let microphone_recording_file = runtime.microphone_recording_file.clone();
        let system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let requested_sources = runtime.requested_sources.clone();

        let Some(screen_planner) = screen_planner_for_runtime(&runtime).cloned() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
        let microphone_planner = microphone_planner_for_runtime(&runtime).cloned();
        let system_audio_planner = system_audio_planner_for_runtime(&runtime).cloned();
        let Some(sources) = runtime.requested_sources.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
        let Some(schedule) = runtime.segment_schedule.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
        let Some(clock) = runtime.capture_clock.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };

        let scheduled_index = schedule.current_segment_index(clock.elapsed());
        if !should_rotate_segment(runtime.current_segment_index, scheduled_index) {
            continue;
        }

        if apply_runtime_signal(&mut runtime, RuntimeSignal::RotateRequested).is_err() {
            mark_runtime_session_failed(&mut runtime);
            break;
        }

        let next_index = (runtime.current_segment_index + 1).max(scheduled_index);
        let segment_dir = screen_planner.segment_dir(next_index);
        let screen_output_file = screen_planner.segment_screen_output(next_index);
        let system_audio_output_path = sources
            .system_audio
            .then(|| {
                system_audio_planner
                    .as_ref()
                    .map(|planner| planner.system_audio_file(next_index))
            })
            .flatten();
        let microphone_output_path = sources
            .microphone
            .then(|| {
                microphone_planner
                    .as_ref()
                    .map(|planner| planner.microphone_file(next_index))
            })
            .flatten();
        let microphone_audio_dir = microphone_output_path.as_deref().and_then(|p| p.parent());
        let system_audio_dir = system_audio_output_path.as_deref().and_then(|p| p.parent());
        if let Err(error) = create_segment_output_dirs(
            &segment_dir,
            microphone_audio_dir,
            system_audio_dir,
            &sources,
        ) {
            crate::native_capture_debug_log::log(format!(
                "failed to prepare capture segment output directories while rotating segments: [{}] {}",
                error.code, error.message
            ));
            mark_runtime_session_failed(&mut runtime);
            break;
        }

        let mut next_segment_outputs = empty_output_files();
        let mut next_recording_file = runtime.recording_file.clone();
        let mut next_microphone_recording_file = runtime.microphone_recording_file.clone();
        let mut next_system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let mut legacy_rotated = false;

        if sources.screen || sources.system_audio {
            let rotate_result =
                capture_screen::rotate_screen_capture_session(RotateScreenCaptureSessionArgs {
                    active_session: &mut runtime.active_screen_session,
                    segment_dir: &segment_dir,
                    screen_output_file: Some(&screen_output_file),
                    system_audio_output_path: system_audio_output_path.as_deref(),
                });

            match rotate_result {
                Ok(rotated) => {
                    if let Some(file) = rotated.output_files.screen_file {
                        set_current_screen_output_file(&mut next_segment_outputs, file);
                    }
                    if let Some(file) = rotated.output_files.system_audio_file {
                        set_current_system_audio_output_file(&mut next_segment_outputs, file);
                    }
                    next_recording_file = Some(rotated.recording_file);
                    next_system_audio_recording_file = rotated.system_audio_recording_file;
                }
                Err(error) if error.code == "capture_rotation_requires_restart" => {
                    legacy_rotated = true;
                }
                Err(_) => {
                    cleanup_failed_segment_dirs(
                        &segment_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                    stop_active_sessions_after_failure(&mut runtime);
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            }
        }

        if legacy_rotated {
            if capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                active_session: &mut runtime.active_screen_session,
            })
            .is_err()
            {
                cleanup_failed_segment_dirs(&segment_dir, microphone_audio_dir, system_audio_dir);
                stop_active_sessions_after_failure(&mut runtime);
                mark_runtime_session_failed(&mut runtime);
                break;
            }

            let screen_only_sources = CaptureSources {
                screen: sources.screen,
                microphone: false,
                system_audio: sources.system_audio && !runtime.inactivity.is_system_audio_paused(),
            };
            let legacy_system_audio_path = screen_only_sources
                .system_audio
                .then(|| system_audio_output_path.as_deref())
                .flatten();

            let started_segment = start_segment(
                &segment_dir,
                Some(&screen_output_file),
                legacy_system_audio_path,
                &screen_only_sources,
                runtime.screen_frame_rate,
                &runtime.screen_resolution,
                runtime.effective_screen_bitrate_bps,
                runtime.microphone_device_id_for_capture.as_deref(),
                runtime.frame_artifact_tx.clone(),
                None, // microphone not restarted in screen-only legacy rotate
            );

            let (
                started_outputs,
                started_recording_file,
                started_microphone_recording_file,
                started_system_audio_recording_file,
                active_screen_session,
                _,
            ) = match started_segment {
                Ok(value) => value,
                Err(_) => {
                    cleanup_failed_segment_dirs(
                        &segment_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                    stop_active_sessions_after_failure(&mut runtime);
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            };

            next_segment_outputs = started_outputs;
            next_recording_file = started_recording_file;
            next_microphone_recording_file = started_microphone_recording_file;
            next_system_audio_recording_file = started_system_audio_recording_file;
            runtime.active_screen_session = active_screen_session;

            if sources.microphone {
                if let Some(session) = runtime.active_microphone_session.as_mut() {
                    let microphone_output_file = microphone_planner
                        .as_ref()
                        .expect("microphone planner should exist when microphone source is enabled")
                        .microphone_file(next_index)
                        .to_string_lossy()
                        .to_string();
                    if session.rotate_output_file(&microphone_output_file).is_err() {
                        cleanup_failed_segment_dirs(
                            &segment_dir,
                            microphone_audio_dir,
                            system_audio_dir,
                        );
                        cleanup_unusable_segment_artifacts(
                            Some(&next_segment_outputs),
                            next_recording_file.as_deref(),
                            next_microphone_recording_file.as_deref(),
                            next_system_audio_recording_file.as_deref(),
                        );
                        stop_active_sessions_after_failure(&mut runtime);
                        mark_runtime_session_failed(&mut runtime);
                        break;
                    }
                    set_current_microphone_output_file(
                        &mut next_segment_outputs,
                        microphone_output_file.clone(),
                    );
                    next_microphone_recording_file = Some(microphone_output_file);
                }
            }
        } else if sources.microphone {
            if let Some(session) = runtime.active_microphone_session.as_mut() {
                let microphone_output_file = microphone_planner
                    .as_ref()
                    .expect("microphone planner should exist when microphone source is enabled")
                    .microphone_file(next_index)
                    .to_string_lossy()
                    .to_string();
                if session.rotate_output_file(&microphone_output_file).is_err() {
                    cleanup_failed_segment_dirs(
                        &segment_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                    cleanup_unusable_segment_artifacts(
                        Some(&next_segment_outputs),
                        next_recording_file.as_deref(),
                        next_microphone_recording_file.as_deref(),
                        next_system_audio_recording_file.as_deref(),
                    );
                    stop_active_sessions_after_failure(&mut runtime);
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
                set_current_microphone_output_file(
                    &mut next_segment_outputs,
                    microphone_output_file.clone(),
                );
                next_microphone_recording_file = Some(microphone_output_file);
            }
        }

        if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
            flush_frame_artifacts(tx);
        }

        let previous_segment_committed = match finalize_capture_outputs(
            previous_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        ) {
            Ok(()) => true,
            Err(error)
                if recover_from_segment_finalize_error(
                    "rotating segments",
                    &error,
                    previous_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                ) =>
            {
                false
            }
            Err(error) => {
                cleanup_failed_segment_dirs(&segment_dir, microphone_audio_dir, system_audio_dir);
                cleanup_unusable_segment_artifacts(
                    previous_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                );
                cleanup_unusable_segment_artifacts(
                    Some(&next_segment_outputs),
                    next_recording_file.as_deref(),
                    next_microphone_recording_file.as_deref(),
                    next_system_audio_recording_file.as_deref(),
                );
                stop_active_sessions_after_failure(&mut runtime);
                mark_runtime_session_failed(&mut runtime);
                crate::native_capture_debug_log::log(format!(
                    "fatal native capture segment finalization failure while rotating segments: [{}] {}",
                    error.code, error.message
                ));
                break;
            }
        };

        if previous_segment_committed {
            if let (Some(committed), Some(segment)) = (
                runtime.output_files.as_mut(),
                previous_segment_output_files.as_ref(),
            ) {
                append_committed_segment_output_files(committed, segment);
            }
        }

        runtime.current_segment_index = next_index;
        runtime.current_segment_output_files = Some(next_segment_outputs);
        runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
            &sources,
            runtime.inactivity.screen_paused,
            runtime.inactivity.microphone_paused,
            runtime.inactivity.system_audio_paused,
        );
        runtime.recording_file = next_recording_file;
        runtime.microphone_recording_file = next_microphone_recording_file;
        runtime.system_audio_recording_file = next_system_audio_recording_file;

        if apply_runtime_signal(&mut runtime, RuntimeSignal::SourcesReady).is_err() {
            stop_active_sessions_after_failure(&mut runtime);
            mark_runtime_session_failed(&mut runtime);
            break;
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
            let session_id = capture_screen::new_session_id()?;
            // Generate independent session IDs for microphone and system audio so each
            // source writes filenames tagged with its own logical source session.
            let microphone_session_id = if sources.microphone {
                Some(capture_screen::new_session_id()?)
            } else {
                None
            };
            let system_audio_session_id = if sources.system_audio {
                Some(capture_screen::new_session_id()?)
            } else {
                None
            };
            let recordings_root = crate::app_infra::recordings_root_dir(&settings.save_directory);
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

            let (
                segment_outputs,
                recording_file,
                microphone_recording_file,
                system_audio_recording_file,
                active_screen_session,
                active_microphone_session,
            ) = start_segment(
                &first_segment_dir,
                Some(&first_screen_output_file),
                first_system_audio_output_path.as_deref(),
                &sources,
                settings.screen_frame_rate,
                &settings.screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id_for_capture.as_deref(),
                frame_artifact_tx.clone(),
                first_microphone_output_path.as_deref(),
            )?;

            let output_files = empty_output_files();
            let segment_loop_control = spawn_segment_loop(app_handle);

            runtime.is_running = true;
            runtime.inactivity =
                crate::native_capture_inactivity::InactivityState::from_recording_settings(
                    settings,
                    started_monotonic,
                );
            runtime.source_sessions = Some(SourceSessions {
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
            });
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
) -> Result<(), CaptureErrorResponse> {
    #[cfg(target_os = "macos")]
    {
        if runtime.is_running {
            apply_runtime_signal(runtime, RuntimeSignal::StopRequested)?;
        }

        let mut current_segment_output_files = runtime.current_segment_output_files.clone();
        let recording_file = runtime.recording_file.clone();
        let microphone_recording_file = runtime.microphone_recording_file.clone();
        let system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let requested_sources = runtime.requested_sources.clone();

        let mut first_error: Option<CaptureErrorResponse> = None;

        if let Some(session) = runtime.active_microphone_session.as_mut() {
            if let Err(error) = session.stop() {
                first_error = Some(error);
            }
            runtime.active_microphone_session = None;
        }

        if let Err(error) =
            capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                active_session: &mut runtime.active_screen_session,
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
        Ok(())
    }
}
