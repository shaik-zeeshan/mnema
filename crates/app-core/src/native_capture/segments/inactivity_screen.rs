//! Inactivity pause and resume for the screen and the whole runtime.

use super::*;

#[cfg(target_os = "macos")]
#[cfg(test)]
pub(crate) fn pause_screen_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_screen_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(crate) fn pause_screen_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
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
    // A screen pause is not the tap's business (ADR 0052): it keeps recording, so
    // the file it is still writing must be kept out of the commit below entirely.
    // An open `.m4a` has no readable duration, which the finalize path reads as
    // unusable and deletes — out from under the running writer — and this path
    // cannot stop the tap first the way every explicit stop path does. Only the
    // live file, though: generations a mid-segment tap rebuild already finalized
    // belong to this segment and are orphaned if dropped from the commit.
    let live_system_audio_file = live_system_audio_continuation_file(runtime);
    let system_audio_recording_file = live_system_audio_file
        .is_none()
        .then(|| runtime.system_audio_recording_file.clone())
        .flatten();
    if let Some(live_file) = live_system_audio_file.as_deref() {
        if let Some(output_files) = current_segment_output_files.as_mut() {
            strip_live_system_audio_output_file(output_files, live_file);
        }
    }
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
                runtime.system_audio_recording_file = live_system_audio_file.clone();
                preserve_live_audio_continuation_outputs(runtime);
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
                runtime.system_audio_recording_file = live_system_audio_file.clone();
                runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
                    active_sources_for_runtime_pause_state(
                        runtime,
                        sources,
                        true, // screen is now stopped
                        runtime.inactivity.microphone_paused,
                        runtime.inactivity.system_audio_paused,
                    )
                });
                // Whichever audio family is still live keeps its continuation
                // bookkeeping, so the capture it is still doing stays trackable.
                preserve_live_audio_continuation_outputs(runtime);
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
    }

    runtime.recording_file = None;
    // Kept exactly when the tap is still writing it: the commit above deliberately
    // left the live file alone, so nulling it here would orphan the segment it is
    // in the middle of recording.
    runtime.system_audio_recording_file = live_system_audio_file.clone();

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

    // Both audio families outlive the screen session, so whichever is still
    // recording keeps its continuation bookkeeping — including a system-audio-only
    // session, which the tap makes a first-class recording (ADR 0052).
    preserve_live_audio_continuation_outputs(runtime);

    mark_screen_paused_for_inactivity(runtime);

    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn mark_screen_paused_for_inactivity(runtime: &mut NativeCaptureRuntime) {
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
pub(crate) fn should_defer_screen_resume_for_missing_display(
    screen_stream_live: bool,
    display_available: bool,
) -> bool {
    !screen_stream_live && !display_available
}

#[cfg(target_os = "macos")]
pub(crate) fn resume_screen_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
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
    let metadata_snapshot_provider =
        core.map(|core| metadata::frame_metadata_snapshot_provider(core));
    let initial_privacy_filter = core.map(|core| privacy::collect_initial_privacy_filter(core));
    resume_screen_from_inactivity_with_start_segment(
        runtime,
        core,
        move |segment_dir,
              screen_output_file,
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
            if let (Some(core), Some(initial)) = (core, initial_privacy_filter) {
                initial.mark_applied(core);
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
                    privacy::record_initial_privacy_filter_outcome(
                        core,
                        &settings,
                        started_segment.5,
                    );
                }
            }
            Ok((
                started_segment.0,
                started_segment.1,
                started_segment.2,
                started_segment.3,
                started_segment.4,
            ))
        },
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn resume_screen_from_inactivity_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
    start_segment_fn: F,
) -> Result<(), CaptureErrorResponse>
where
    F: FnOnce(
        &Path,
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
        if let Some(core) = core {
            // This is the synchronous inactivity-resume path: it runs while the
            // `NativeCaptureState` mutex is held (segment loop -> tick_inactivity).
            // Use the cached browser URL so the bounded-but-slow Gecko AX read
            // cannot stall the held lock; the next off-lock metadata tick will
            // refresh the live URL.
            let (_, privacy_filter_update) = privacy::collect_privacy_filter_update(
                core,
                privacy::PrivacyRefreshReason::FallbackPoll,
                crate::native_capture::metadata::BrowserUrlReadMode::Cached,
            );
            let _ = privacy::apply_privacy_filter_update(core, runtime, privacy_filter_update)?;
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

        // Re-anchoring the segment clock re-anchors system audio's file with it,
        // purely so the two stay on the same segment index — the tap itself never
        // paused and does not care that the screen did.
        let next_system_audio_recording_file = next_reanchored_system_audio_output_file(
            runtime,
            next_index,
            "resuming screen outputs from inactivity",
        )?;
        if next_system_audio_recording_file.is_some() {
            append_and_persist_committed_audio_outputs(
                runtime,
                core,
                previous_system_audio_outputs.as_ref(),
            );
        }

        let next_microphone_recording_file = next_reanchored_microphone_output_file(
            runtime,
            next_index,
            "resuming screen outputs from inactivity",
        )?;
        if let Some(microphone_output_file) = next_microphone_recording_file.as_deref() {
            crate::native_capture::activity::process_pending_microphone_vad_frames(runtime);
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
                    core,
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

    // Start only the screen; the microphone session and the system-audio tap are
    // untouched by a screen pause and are both still running.
    let screen_only_sources = CaptureSources {
        screen: sources.screen,
        microphone: false,
        system_audio: false,
    };

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);

    let started_segment = start_segment_fn(
        &segment_dir,
        Some(&screen_output_file),
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
        mut active_screen_session,
        _active_microphone_session,
    ) = started_segment;

    let next_microphone_recording_file = next_reanchored_microphone_output_file(
        runtime,
        next_index,
        "resuming screen from inactivity",
    )?;
    if let Some(microphone_output_file) = next_microphone_recording_file.as_deref() {
        crate::native_capture::activity::process_pending_microphone_vad_frames(runtime);
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            let mic_finalization = match session
                .rotate_output_file_returning_finalization(microphone_output_file)
            {
                Ok(finalization) => finalization,
                Err(error) => {
                    let _ =
                        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                            active_session: &mut active_screen_session,
                        });
                    cleanup_unusable_segment_artifacts(
                        Some(&segment_outputs),
                        recording_file.as_deref(),
                        None,
                        None,
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
                core,
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
    merge_live_system_audio_continuation_into_segment_outputs(runtime, &mut segment_outputs);

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
pub(crate) fn pause_runtime_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_runtime_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(crate) fn pause_runtime_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
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
        pause_microphone_for_inactivity_with_app_handle(runtime, core)?;
    }

    if runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
    {
        pause_system_audio_for_inactivity_with_app_handle(runtime, core)?;
    }

    if runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.screen)
    {
        pause_screen_for_inactivity_with_app_handle(runtime, core)?;
    }

    runtime.current_segment_sources = requested_sources.as_ref().and_then(|sources| {
        active_sources_for_runtime_pause_state(runtime, sources, true, true, sources.system_audio)
    });
    runtime.inactivity.is_paused = true;

    Ok(())
}

#[cfg(target_os = "macos")]
/// `(segment outputs, screen file, microphone file, screen session, mic session)`.
/// System audio is absent by design: it starts and stops on its own tap.
pub(crate) type StartedSegmentState = (
    CaptureOutputFiles,
    Option<String>,
    Option<String>,
    Option<capture_screen::ActiveCaptureSession>,
    Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
);

#[cfg(target_os = "macos")]
pub(crate) fn resume_runtime_from_inactivity(
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
pub(crate) fn should_fail_runtime_on_inactivity_resume_error(error: &CaptureErrorResponse) -> bool {
    // Missing core runtime state indicates a corrupted paused session; retrying will not recover it.
    error.code == "invalid_runtime_state"
}

#[cfg(target_os = "macos")]
pub(crate) fn handle_inactivity_resume_error(
    runtime: &mut NativeCaptureRuntime,
    error: CaptureErrorResponse,
    retry_log_level: log::Level,
) -> bool {
    if should_fail_runtime_on_inactivity_resume_error(&error) {
        crate::debug_log::log_error(format!(
            "fatal native capture inactivity resume failure: [{}] {}",
            error.code, error.message
        ));
        mark_runtime_session_failed(runtime);
        return true;
    }

    crate::debug_log::log_at(
        retry_log_level,
        &format!(
            "failed to resume native capture after activity; keeping session paused for retry: [{}] {}",
            error.code, error.message
        ),
    );

    false
}
