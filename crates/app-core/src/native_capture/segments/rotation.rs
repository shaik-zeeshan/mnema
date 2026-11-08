//! Segment timing and rotation planning: reanchoring, contiguous numbering,
//! the loop's sleep, and the capture options a new segment opens with.

use super::*;

pub(crate) fn reanchor_active_segment_timing(
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
        crate::native_capture::runtime::now_unix_ms(),
    );

    Ok(())
}

pub(crate) fn reanchored_source_session_started_at_unix_ms(
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

pub(crate) fn reanchor_source_session_timing(
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

pub(crate) fn next_emitted_segment_index(current_segment_index: u64) -> u64 {
    current_segment_index + 1
}

pub(crate) fn segment_loop_sleep_duration(
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
pub(crate) struct PlannedSegmentRotation {
    pub(crate) next_index: u64,
    pub(crate) segment_dir: PathBuf,
    pub(crate) screen_output_file: PathBuf,
    pub(crate) microphone_output_path: Option<PathBuf>,
    pub(crate) system_audio_output_path: Option<PathBuf>,
}

#[cfg(target_os = "macos")]
pub(crate) fn plan_live_rotation_segment(
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
pub(crate) fn recover_from_segment_finalize_error(
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
    crate::debug_log::log_warn(format!(
        "recovered native capture segment finalization failure while {context}: [{}] {}",
        error.code, error.message
    ));

    true
}

pub(crate) fn capture_session_options(
    frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
    metadata_snapshot_provider: Option<metadata::FrameMetadataSnapshotProvider>,
    initial_privacy_filter: Option<capture_screen::PrivacyContentFilter>,
) -> capture_screen::ScreenCaptureSessionOptions {
    let Some(frame_artifact_tx) = frame_artifact_tx else {
        return capture_screen::ScreenCaptureSessionOptions {
            initial_privacy_filter,
            ..Default::default()
        };
    };

    if !capture_screen::supports_frame_export() {
        return capture_screen::ScreenCaptureSessionOptions {
            initial_privacy_filter,
            ..Default::default()
        };
    }

    let channel_closed_logged = std::sync::atomic::AtomicBool::new(false);
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
                        // The worker is gone for this session; say so once, not
                        // once per exported frame.
                        if !channel_closed_logged.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            crate::debug_log::log_error(
                                "failed to forward native frame artifact for persistence: worker channel closed",
                            );
                        }
                    }
                }
            }),
        }),
        initial_privacy_filter,
    }
}
