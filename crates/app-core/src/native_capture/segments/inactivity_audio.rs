//! Inactivity pause and resume for the two audio families.

use super::*;

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn pause_microphone_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_microphone_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(crate) fn pause_microphone_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_microphone_paused() {
        return Ok(());
    }
    // With no session to finalize, a pause would only drop the file the
    // enrollment stop already closed from the segment it belongs to.
    if runtime.microphone_lent_out {
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

    // Speech the VAD has not judged yet belongs to the file about to close.
    crate::native_capture::activity::process_pending_microphone_vad_frames(runtime);
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
    } else {
        // No session (its device went, or a dead one was dropped): the file its
        // stop closed is booked for the next commit, so commit it now rather
        // than clear it below.
        commit_suspended_microphone_outputs(core, runtime, "pausing for inactivity");
    }

    if let Some(finalized) = finalized_microphone_outputs.as_ref() {
        if let Some(committed) = runtime.output_files.as_mut() {
            append_committed_segment_output_files(committed, finalized);
        }
        persist_committed_microphone_segments(
            core,
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
    mark_microphone_family_paused(runtime);

    Ok(())
}

/// Books the microphone family as paused for inactivity and drops it from the
/// live segment's sources, so rotation leaves it alone until a resume.
#[cfg(target_os = "macos")]
pub(crate) fn mark_microphone_family_paused(runtime: &mut NativeCaptureRuntime) {
    runtime.inactivity.set_family_paused_states(
        runtime.inactivity.screen_paused,
        true,
        runtime.inactivity.system_audio_paused,
    );
    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return;
    };
    runtime.current_segment_sources = active_sources_for_runtime_pause_state(
        runtime,
        &requested_sources,
        runtime.inactivity.screen_paused,
        true, // microphone is now paused
        runtime.inactivity.system_audio_paused,
    );
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn process_inactivity_audio_transitions_for_snapshot(
    runtime: &mut NativeCaptureRuntime,
    now: u64,
    activity_snapshot: crate::native_capture_inactivity::ActivitySnapshot,
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
pub(crate) fn pause_system_audio_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    pause_system_audio_for_inactivity_with_app_handle(runtime, None)
}

#[cfg(target_os = "macos")]
pub(crate) fn pause_system_audio_for_inactivity_with_app_handle(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
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

    // Pause is the planner hook returning no path: the tap and its zero-watchdog
    // stay alive with nothing being written (ADR 0052). Stopping the tap instead
    // would be self-defeating — the resume trigger is "sound detected", and a
    // stopped tap can never deliver it.
    if runtime.active_system_audio_session.is_some() {
        let system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let planner = ensure_system_audio_planner_for_runtime(
            runtime,
            "pausing system audio for inactivity",
        )?;
        let segment_index = runtime.current_segment_index;
        if let (Some(planner), Some(session)) =
            (planner, runtime.active_system_audio_session.as_mut())
        {
            session.advance_segment(&planner, segment_index, None);
        }

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
                core,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                Some(finalized),
            );
        }
    }

    // The finished system-audio file has already been appended to the committed
    // output list and persisted. Remove it from the live segment bookkeeping so a
    // later pause/rotation/stop cannot upsert the same audio path again with the
    // later segment clock.
    if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
        output_files.system_audio_file = None;
        output_files.system_audio_files.clear();
    }
    runtime.system_audio_recording_file = None;

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
pub(crate) fn resume_microphone_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_microphone_paused() {
        return Ok(());
    }
    // Voice enrollment holds the device; its own speech must not wake a second
    // session. Restoring the microphone clears this pause.
    if runtime.microphone_lent_out {
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
            crate::native_capture::microphone::next_microphone_output_file_for_runtime(runtime)?;
        let microphone_tail_activity_mode = microphone_tail_trim_activity_mode_for_runtime(runtime);

        if let Some(session) = runtime.active_microphone_session.as_mut() {
            session.resume_output_file_with_inactivity_tail_trim_activity_mode(
                &microphone_recording_file,
                runtime.inactivity.idle_timeout_seconds,
                runtime.inactivity.microphone_activity_threshold(),
                microphone_tail_activity_mode,
            )?;
            runtime.microphone_recording_file = Some(microphone_recording_file.clone());
            crate::native_capture_output::set_current_microphone_output_file(
                current_segment_output_files_mut(runtime),
                microphone_recording_file,
            );
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
                    crate::native_capture_output::set_current_microphone_output_file(
                        current_segment_output_files_mut(runtime),
                        microphone_recording_file,
                    );
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
pub(crate) fn resume_system_audio_from_inactivity(
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

    // Nothing here asks about the screen. The tap owns its own liveness, so a
    // resume only needs a fresh output path and the family's own session — which
    // is why system audio can resume while the screen is paused, suspended, or
    // was never requested at all (ADR 0052).
    if runtime.active_system_audio_session.is_some() {
        refresh_runtime_planner_dates(runtime);
        // Always seed the planner on a real resume so future resumes/rotations
        // preserve the dedicated system-audio session.
        let planner = ensure_system_audio_planner_for_runtime(
            runtime,
            "resuming system audio from inactivity",
        )?
        .ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture system-audio planner missing while resuming system audio".to_string(),
        })?;
        let next_file = planner.system_audio_resume_file(
            runtime.current_segment_index,
            crate::native_capture::runtime::now_unix_ms(),
        );
        let segment_index = runtime.current_segment_index;

        let resumed = runtime
            .active_system_audio_session
            .as_mut()
            .expect("system audio session checked above")
            .advance_segment(&planner, segment_index, Some(next_file));

        if let Some(new_system_audio_file) = resumed {
            runtime.system_audio_recording_file = Some(new_system_audio_file.clone());
            set_current_system_audio_output_file(
                current_segment_output_files_mut(runtime),
                new_system_audio_file,
            );
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
