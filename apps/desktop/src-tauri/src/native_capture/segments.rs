use crate::native_capture_output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
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
use capture_types::{CaptureErrorResponse, CaptureOutputFiles, CaptureSources, RecordingSettings};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::Manager;
use tokio::sync::mpsc;

use super::activity::current_activity_snapshot;
use super::runtime::{
    apply_runtime_signal, mark_runtime_session_failed, now_monotonic_marker_ms, now_unix_ms,
    reset_runtime_after_start_error, should_rotate_segment, NativeCaptureRuntime,
    NativeCaptureState, SegmentLoopControl,
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
fn cleanup_failed_segment_dir(segment_dir: &Path) {
    if let Err(error) = std::fs::remove_dir_all(segment_dir) {
        if error.kind() != std::io::ErrorKind::NotFound {
            crate::native_capture_debug_log::log(format!(
                "failed removing unusable segment directory {}: {}",
                segment_dir.display(),
                error
            ));
        }
    }
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
fn pause_runtime_for_inactivity(
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

    if let Some(session) = runtime.active_microphone_session.as_mut() {
        session.stop()?;
    }
    runtime.active_microphone_session = None;

    capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
    })?;

    if let Some(tx) = runtime.frame_artifact_tx.as_ref() {
        flush_frame_artifacts(tx);
    }

    finalize_capture_outputs(
        current_segment_output_files.as_mut(),
        recording_file.as_deref(),
        microphone_recording_file.as_deref(),
        system_audio_recording_file.as_deref(),
        requested_sources.as_ref(),
    )?;

    if let (Some(committed), Some(segment)) = (
        runtime.output_files.as_mut(),
        current_segment_output_files.as_ref(),
    ) {
        append_committed_segment_output_files(committed, segment);
    }

    runtime.current_segment_output_files = None;
    runtime.recording_file = None;
    runtime.microphone_recording_file = None;
    runtime.system_audio_recording_file = None;
    runtime.inactivity.is_paused = true;

    Ok(())
}

#[cfg(target_os = "macos")]
fn resume_runtime_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_paused {
        return Ok(());
    }

    let Some(planner) = runtime.segment_planner.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture segment planner missing while resuming inactivity".to_string(),
        });
    };
    let Some(sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming inactivity".to_string(),
        });
    };
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

    let scheduled_index = schedule.current_segment_index(clock.elapsed());
    let next_index = (runtime.current_segment_index + 1).max(scheduled_index);
    let segment_dir = planner.segment_dir(next_index);

    let (
        segment_outputs,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        active_microphone_session,
    ) = start_segment(
        &segment_dir,
        &sources,
        runtime.screen_frame_rate,
        &runtime.screen_resolution,
        runtime.effective_screen_bitrate_bps,
        runtime.microphone_device_id_for_capture.as_deref(),
        runtime.frame_artifact_tx.clone(),
    )?;

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.recording_file = recording_file;
    runtime.microphone_recording_file = microphone_recording_file;
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    runtime.active_microphone_session = active_microphone_session;
    runtime.inactivity.is_paused = false;

    Ok(())
}

#[cfg(target_os = "macos")]
fn start_segment(
    session_dir: &Path,
    sources: &CaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &capture_types::ScreenResolution,
    effective_screen_bitrate_bps: Option<u32>,
    microphone_device_id: Option<&str>,
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
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
    std::fs::create_dir_all(session_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture segment directory: {error}"),
    })?;

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
        let screen_capture = capture_screen::start_capture_session_with_options(
            session_dir,
            &screen_sources,
            screen_frame_rate,
            screen_resolution,
            effective_screen_bitrate_bps,
            frame_export_options(frame_artifact_tx),
        )?;

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
        let microphone_output_file = session_dir
            .join("microphone.m4a")
            .to_string_lossy()
            .to_string();

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

        if runtime
            .inactivity
            .should_resume_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_runtime_from_inactivity(&mut runtime) {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    mark_runtime_session_failed(&mut runtime);
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

        let Some(planner) = runtime.segment_planner.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
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
        let segment_dir = planner.segment_dir(next_index);
        if std::fs::create_dir_all(&segment_dir).is_err() {
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
                    cleanup_failed_segment_dir(&segment_dir);
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
                cleanup_failed_segment_dir(&segment_dir);
                stop_active_sessions_after_failure(&mut runtime);
                mark_runtime_session_failed(&mut runtime);
                break;
            }

            let screen_only_sources = CaptureSources {
                screen: sources.screen,
                microphone: false,
                system_audio: sources.system_audio,
            };

            let started_segment = start_segment(
                &segment_dir,
                &screen_only_sources,
                runtime.screen_frame_rate,
                &runtime.screen_resolution,
                runtime.effective_screen_bitrate_bps,
                runtime.microphone_device_id_for_capture.as_deref(),
                runtime.frame_artifact_tx.clone(),
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
                    cleanup_failed_segment_dir(&segment_dir);
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
                    let microphone_output_file = planner
                        .microphone_file(next_index)
                        .to_string_lossy()
                        .to_string();
                    if session.rotate_output_file(&microphone_output_file).is_err() {
                        cleanup_failed_segment_dir(&segment_dir);
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
                let microphone_output_file = planner
                    .microphone_file(next_index)
                    .to_string_lossy()
                    .to_string();
                if session.rotate_output_file(&microphone_output_file).is_err() {
                    cleanup_failed_segment_dir(&segment_dir);
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

        if finalize_capture_outputs(
            previous_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        )
        .is_err()
        {
            cleanup_failed_segment_dir(&segment_dir);
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
            break;
        }

        if let (Some(committed), Some(segment)) = (
            runtime.output_files.as_mut(),
            previous_segment_output_files.as_ref(),
        ) {
            append_committed_segment_output_files(committed, segment);
        }

        runtime.current_segment_index = next_index;
        runtime.current_segment_output_files = Some(next_segment_outputs);
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
            let segment_planner =
                SegmentPlanner::new(settings.save_directory.clone(), session_id.clone());
            let segment_schedule =
                SegmentSchedule::new(Duration::from_secs(settings.segment_duration_seconds));
            let capture_clock = CaptureClock::start_now();
            let frame_artifact_tx = sources
                .screen
                .then(|| spawn_frame_artifact_worker(&app_handle, session_id.clone()));
            std::fs::create_dir_all(Path::new(&settings.save_directory)).map_err(|error| {
                CaptureErrorResponse {
                    code: "io_error".to_string(),
                    message: format!("Failed to create capture save directory: {error}"),
                }
            })?;

            let segment_index = 1;
            let first_segment_dir = segment_planner.segment_dir(segment_index);
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
                &sources,
                settings.screen_frame_rate,
                &settings.screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id_for_capture.as_deref(),
                frame_artifact_tx.clone(),
            )?;

            let output_files = empty_output_files();
            let segment_loop_control = spawn_segment_loop(app_handle);

            runtime.is_running = true;
            runtime.inactivity =
                crate::native_capture_inactivity::InactivityState::from_recording_settings(
                    settings,
                    started_monotonic,
                );
            runtime.started_at_unix_ms = Some(started);
            runtime.session_id = Some(session_id);
            runtime.requested_sources = Some(sources);
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
            let _ = frame_artifact_tx;
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
