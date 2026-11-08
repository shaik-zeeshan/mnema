//! Opening a segment with the current privacy filter.

use super::*;

#[cfg(target_os = "macos")]
pub(crate) fn start_segment_with_current_privacy_filter(
    core: &Arc<AppCore>,
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    sources: &CaptureSources,
    screen_frame_rate: f64,
    screen_resolution: &capture_types::ScreenResolution,
    effective_screen_bitrate_bps: Option<u32>,
    microphone_device_id: Option<&str>,
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
    microphone_output_path: Option<&Path>,
) -> Result<StartedSegmentState, CaptureErrorResponse> {
    let initial_privacy_filter = privacy::collect_initial_privacy_filter(core);
    let started_segment = start_segment_with_inactivity_tail_trim_seconds(
        session_dir,
        screen_output_file,
        sources,
        screen_frame_rate,
        screen_resolution,
        effective_screen_bitrate_bps,
        microphone_device_id,
        frame_artifact_tx,
        Some(metadata::frame_metadata_snapshot_provider(core)),
        microphone_output_path,
        0,
        0.0,
        microphone_capture::MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        initial_privacy_filter.screen_capture_filter(),
    )?;
    initial_privacy_filter.mark_applied(core);
    if let Some(settings) = core
        .try_state::<crate::recording_settings::RecordingSettingsState>()
        .map(|state| {
            state
                .lock()
                .expect("recording settings state poisoned")
                .settings
                .clone()
        })
    {
        privacy::record_initial_privacy_filter_outcome(core, &settings, started_segment.5);
    }
    Ok((
        started_segment.0,
        started_segment.1,
        started_segment.2,
        started_segment.3,
        started_segment.4,
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn start_segment_with_inactivity_tail_trim_seconds(
    session_dir: &Path,
    screen_output_file: Option<&Path>,
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
        Option<capture_screen::ActiveCaptureSession>,
        Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
        Option<capture_screen::PrivacyFilterApplyOutcome>,
    ),
    CaptureErrorResponse,
> {
    let _ = &frame_artifact_tx;
    cleanup_failed_audio_outputs(microphone_output_path, None);
    let microphone_audio_dir = microphone_output_path.and_then(|p| p.parent());
    create_segment_output_dirs(session_dir, microphone_audio_dir, None, sources)?;

    let mut output_files = empty_output_files();
    let mut recording_file: Option<String> = None;
    let mut microphone_recording_file: Option<String> = None;
    let mut active_screen_session: Option<capture_screen::ActiveCaptureSession> = None;
    let mut active_microphone_session: Option<
        microphone_capture::AvFoundationMicrophoneCaptureSession,
    > = None;
    let mut initial_privacy_filter_outcome = None;

    // Screen only: system audio starts on its own tap, so it is never a reason to
    // bring a ScreenCaptureKit stream up (ADR 0052).
    if sources.screen {
        let screen_sources = capture_screen::ScreenCaptureSources {
            screen: sources.screen,
        };
        let screen_options = capture_session_options(
            frame_artifact_tx,
            metadata_snapshot_provider,
            initial_privacy_filter,
        );
        let screen_capture = match capture_screen::start_capture_session_with_options(
            session_dir,
            screen_output_file,
            &screen_sources,
            screen_frame_rate,
            screen_resolution,
            effective_screen_bitrate_bps,
            screen_options,
        ) {
            Ok(screen_capture) => screen_capture,
            Err(error) => {
                if error.code != "capture_start_rollback_incomplete" {
                    cleanup_failed_segment_dirs(session_dir, microphone_audio_dir, None);
                }
                return Err(error);
            }
        };

        if let Some(screen_file) = screen_capture.output_files.screen_file {
            set_current_screen_output_file(&mut output_files, screen_file);
        }

        recording_file = Some(screen_capture.recording_file);
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

                cleanup_failed_segment_dirs(session_dir, microphone_audio_dir, None);
                return Err(error);
            }
        }
    }

    Ok((
        output_files,
        recording_file,
        microphone_recording_file,
        active_screen_session,
        active_microphone_session,
        initial_privacy_filter_outcome,
    ))
}
