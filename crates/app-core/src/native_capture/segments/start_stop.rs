//! Starting and stopping the capture runtime, and the frame artifact worker a
//! start spawns.

use super::*;

pub(crate) fn spawn_frame_artifact_worker(
    core: &Arc<AppCore>,
    session_id: String,
) -> mpsc::Sender<FrameArtifactMessage> {
    let (tx, mut rx) = mpsc::channel(FRAME_ARTIFACT_BUFFER_CAPACITY);
    let worker_core = Arc::clone(core);
    let infra = Arc::clone(&*core.state::<crate::AppInfraState>());

    core.spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                FrameArtifactMessage::Artifact(envelope) => {
                    if let Err(error) = crate::app_infra::persist_screen_frame_artifact(
                        infra.as_ref(),
                        &worker_core.state::<crate::recording_settings::RecordingSettingsState>(),
                        envelope.metadata_snapshot,
                        &session_id,
                        envelope.artifact,
                    )
                    .await
                    {
                        crate::debug_log::log_error(format!(
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

pub(crate) fn start_capture_runtime(
    runtime: &mut NativeCaptureRuntime,
    core: Arc<AppCore>,
    settings: &RecordingSettings,
    sources: CaptureSources,
    microphone_device_id_for_capture: Option<String>,
) -> Result<(), CaptureErrorResponse> {
    // Screen permission gates screen capture only. A Core Audio process tap has
    // its own TCC category ("Screen & System Audio Recording") and prompts on its
    // first read, so requiring the screen grant here would refuse to start an
    // audio-only session that needs no screen at all (ADR 0052). Nothing gates
    // system audio here: there is no authorization to check, and a denied tap
    // starts and runs exactly like a granted one.
    if settings.capture_screen {
        let screen_ok = capture_screen::ensure_screen_permission();
        if !screen_ok {
            return Err(CaptureErrorResponse {
                code: "screen_permission_denied".to_string(),
                message: "Screen capture permission is required".to_string(),
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
                .then(|| spawn_frame_artifact_worker(&core, session_id.clone()));
            std::fs::create_dir_all(&recordings_root).map_err(|error| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!("Failed to create capture recordings directory: {error}"),
            })?;

            // Lock is away: a start while the Mac is away (a trigger, a
            // schedule) leaves the microphone down under the away hold, and the
            // tick opens it once the Mac is back.
            let microphone_held_away =
                sources.microphone && crate::native_capture::microphone_away::mac_is_away(runtime);
            let first_segment_sources = CaptureSources {
                microphone: sources.microphone && !microphone_held_away,
                ..sources.clone()
            };

            let segment_index = 1;
            let first_segment_dir = segment_planner.segment_dir(segment_index);
            let first_screen_output_file = segment_planner.segment_screen_output(segment_index);
            let first_microphone_output_path = microphone_planner
                .as_ref()
                .filter(|_| first_segment_sources.microphone)
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
            capture_system_audio::reset_system_audio_activity();
            let initial_inactivity =
                crate::native_capture_inactivity::InactivityState::from_recording_settings(
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
            privacy::reset_privacy_filter_refresh_state(&core);
            let initial_privacy_filter = privacy::collect_initial_privacy_filter(&core);
            // The same exclusions the screen filter applies, for the tap's exclude
            // list: privacy parity with what ScreenCaptureKit's filter used to do
            // for system audio (ADR 0052).
            let initial_privacy_filter_excluded_bundle_ids =
                initial_privacy_filter.excluded_bundle_ids();

            let (
                mut segment_outputs,
                recording_file,
                microphone_recording_file,
                active_screen_session,
                active_microphone_session,
                initial_privacy_filter_outcome,
            ) = start_segment_with_inactivity_tail_trim_seconds(
                &first_segment_dir,
                Some(&first_screen_output_file),
                &first_segment_sources,
                settings.screen_frame_rate,
                &settings.screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id_for_capture.as_deref(),
                frame_artifact_tx.clone(),
                Some(metadata::frame_metadata_snapshot_provider(&core)),
                first_microphone_output_path.as_deref(),
                inactivity_tail_trim_seconds,
                microphone_activity_threshold,
                microphone_tail_activity_mode,
                initial_privacy_filter.screen_capture_filter(),
            )?;
            initial_privacy_filter.mark_applied(&core);
            privacy::record_initial_privacy_filter_outcome(
                &core,
                &settings,
                initial_privacy_filter_outcome,
            );

            let output_files = empty_output_files();
            let segment_loop_control = spawn_segment_loop(core.clone());
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
                &core,
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
            runtime.recording_started_at_unix_ms = Some(started);
            runtime.requested_sources = Some(sources.clone());
            runtime.current_segment_sources = Some(sources);
            runtime.output_files = Some(output_files);
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
            runtime.active_screen_session = active_screen_session;
            runtime.active_microphone_session = active_microphone_session;
            runtime.microphone_away_hold = microphone_held_away;
            runtime.capture_suspension = None;

            // The tap starts last and on its own terms: a session with screen and
            // microphone both off is a valid system-audio-only recording, and a
            // tap that fails to start costs only system audio (ADR 0052).
            match start_system_audio_family_for_runtime(
                runtime,
                initial_privacy_filter_excluded_bundle_ids,
                segment_index,
                "starting capture",
            ) {
                Ok(system_audio_recording_file) => {
                    if let Some(system_audio_output_file) = system_audio_recording_file.as_ref() {
                        set_current_system_audio_output_file(
                            &mut segment_outputs,
                            system_audio_output_file.clone(),
                        );
                    }
                    runtime.system_audio_recording_file = system_audio_recording_file;
                }
                Err(error) => {
                    crate::debug_log::log_warn(format!(
                        "failed to start system audio capture; continuing without it: [{}] {}",
                        error.code, error.message
                    ));
                }
            }
            runtime.current_segment_output_files = Some(segment_outputs);

            apply_runtime_signal(runtime, RuntimeSignal::SourcesReady)?;
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = sources;
            let _ = microphone_device_id_for_capture;
            let _ = core;
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

pub(crate) fn stop_capture_runtime(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
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

        crate::native_capture::activity::process_pending_microphone_vad_frames(runtime);
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
        // An away hold belongs to the session just stopped.
        runtime.microphone_away_hold = false;
        // So do the inactivity pauses: every family stops here. One left
        // standing would keep the fresh file a user resume opens out of rotation
        // (past the 5-minute cap) until the next activity resume closed it and
        // dropped its finalization.
        runtime
            .inactivity
            .set_family_paused_states(false, false, false);

        // Before the finalize below, like every other source: it closes the tap's
        // asset writer, and `finalize_capture_outputs` drops an m4a that has no
        // readable duration — which is exactly what a still-open writer's file has.
        crate::native_capture::runtime::stop_system_audio_session(runtime);

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
            persist_committed_audio_segments(
                core,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                current_segment_output_files.as_ref(),
            );
            warm_scrub_previews_for_committed_screen_outputs(
                core,
                current_segment_output_files.as_ref(),
            );
            close_frame_batches_for_stopped_screen_session(core, runtime.source_sessions.as_ref())?;
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
        let _ = core;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn request_runtime_stop_transition_if_needed(
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
