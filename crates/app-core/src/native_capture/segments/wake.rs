//! Wake recovery, and the live audio continuation a screen restart carries.

use super::*;

#[cfg(target_os = "macos")]
pub(crate) fn refresh_current_segment_sources_for_pause_state(
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

/// Keeps the audio families' in-flight files tracked after screen capture stops.
///
/// Both are independent of the screen, so both can still be recording: the
/// microphone on its own session, system audio on its own tap. A file dropped
/// here is a file no rotation or stop ever commits.
///
/// The microphone's files closed this segment are carried too: the suspension
/// commits screen and system audio only, and a stop that closed one (a vanished
/// device, enrollment) leaves it for the next rotation to commit.
#[cfg(target_os = "macos")]
pub(crate) fn preserve_live_audio_continuation_outputs(runtime: &mut NativeCaptureRuntime) {
    let mut audio_continuation = empty_output_files();
    let mut has_live_audio =
        merge_live_microphone_continuation_into_segment_outputs(runtime, &mut audio_continuation);

    if let Some(system_audio_file) = live_system_audio_continuation_file(runtime) {
        has_live_audio = true;
        set_current_system_audio_output_file(&mut audio_continuation, system_audio_file);
    }

    runtime.current_segment_output_files = has_live_audio.then_some(audio_continuation);
}

#[cfg(target_os = "macos")]
pub(crate) fn live_microphone_continuation_file(runtime: &NativeCaptureRuntime) -> Option<String> {
    (!runtime.inactivity.microphone_paused)
        .then(|| runtime.microphone_recording_file.clone())
        .flatten()
}

/// The file the tap is still writing, if it is still writing one.
///
/// The single answer to "did system audio survive this?", which the suspend path
/// uses for both the bookkeeping it keeps and the file it declines to commit.
#[cfg(target_os = "macos")]
pub(crate) fn live_system_audio_continuation_file(
    runtime: &NativeCaptureRuntime,
) -> Option<String> {
    (runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
        && !runtime.inactivity.is_system_audio_paused()
        && !crate::native_capture::runtime::system_audio_stops_with_suspension(runtime))
    .then(|| runtime.system_audio_recording_file.clone())
    .flatten()
}

/// Carries the microphone into a rebuilt segment's outputs: the files its stops
/// closed this segment (a vanished device, enrollment), which no other path
/// commits, and the file it is still recording. Whether the microphone has
/// anything in the new segment.
#[cfg(target_os = "macos")]
pub(crate) fn merge_live_microphone_continuation_into_segment_outputs(
    runtime: &NativeCaptureRuntime,
    segment_outputs: &mut CaptureOutputFiles,
) -> bool {
    if let Some(output_files) = runtime.current_segment_output_files.as_ref() {
        segment_outputs.microphone_file = output_files.microphone_file.clone();
        segment_outputs.microphone_files = output_files.microphone_files.clone();
    }
    if let Some(mic_file) = live_microphone_continuation_file(runtime) {
        if segment_outputs.microphone_files.contains(&mic_file) {
            segment_outputs.microphone_file = Some(mic_file);
        } else {
            set_current_microphone_output_file(segment_outputs, mic_file);
        }
    }
    !segment_outputs.microphone_files.is_empty()
        || (!runtime.inactivity.microphone_paused && runtime.active_microphone_session.is_some())
}

/// The screen's pause/resume paths rebuild `current_segment_output_files` from
/// the screen alone; the tap's live file has to be carried across or the segment
/// it is still writing is orphaned.
#[cfg(target_os = "macos")]
pub(crate) fn merge_live_system_audio_continuation_into_segment_outputs(
    runtime: &NativeCaptureRuntime,
    segment_outputs: &mut CaptureOutputFiles,
) {
    if let Some(system_audio_file) = live_system_audio_continuation_file(runtime) {
        set_current_system_audio_output_file(segment_outputs, system_audio_file);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn current_screen_output_file(
    output_files: Option<&CaptureOutputFiles>,
) -> Option<&str> {
    let output_files = output_files?;

    output_files
        .screen_file
        .as_deref()
        .or_else(|| output_files.screen_files.last().map(String::as_str))
}

#[cfg(target_os = "macos")]
pub(crate) fn current_system_audio_output_file(
    output_files: Option<&CaptureOutputFiles>,
) -> Option<&str> {
    let output_files = output_files?;

    output_files
        .system_audio_file
        .as_deref()
        .or_else(|| output_files.system_audio_files.last().map(String::as_str))
}

#[cfg(target_os = "macos")]
pub(crate) fn recover_screen_capture_after_wake_with_start_segment<F>(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
    start_segment_fn: F,
) -> Result<bool, CaptureErrorResponse>
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

    // Screen only. The tap is not torn down by a wake, so its in-flight file must
    // not be finalized here — only the audio families' own paths ever close their
    // segments (ADR 0052).
    let screen_sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };

    // The tap is not torn down by a wake, so the file it is still writing must not
    // be finalized here — an open `.m4a` reads as zero-duration and the finalize
    // path would delete it out from under the running writer. But generations a
    // mid-segment tap rebuild already finalized belong to this segment and are
    // orphaned if dropped, so strip only the live file and let the earlier
    // generations commit with the previous segment — mirroring the suspend path
    // (`commit_suspended_screen_system_outputs`), which strips exactly the same way.
    let live_system_audio_file = live_system_audio_continuation_file(runtime);
    let mut previous_screen_outputs =
        runtime
            .current_segment_output_files
            .clone()
            .map(|mut outputs| {
                outputs.microphone_file = None;
                outputs.microphone_files.clear();
                if let Some(live_file) = live_system_audio_file.as_deref() {
                    strip_live_system_audio_output_file(&mut outputs, live_file);
                }
                outputs
            });
    let recording_file = runtime.recording_file.clone().or_else(|| {
        current_screen_output_file(previous_screen_outputs.as_ref()).map(str::to_owned)
    });

    if let Err(error) = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
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
        None,
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
                None,
            ) =>
        {
            false
        }
        Err(error) => {
            cleanup_unusable_segment_artifacts(
                previous_screen_outputs.as_ref(),
                recording_file.as_deref(),
                None,
                None,
            );
            crate::debug_log::log_error(format!(
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
        warm_scrub_previews_for_committed_screen_outputs(core, previous_screen_outputs.as_ref());
    }

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);

    let started_segment = match start_segment_fn(
        &segment_dir,
        Some(&screen_output_file),
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
            runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
                &requested_sources,
                true,
                runtime.inactivity.microphone_paused,
                runtime.inactivity.system_audio_paused,
            );
            preserve_live_audio_continuation_outputs(runtime);
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
        mut active_screen_session,
        _active_microphone_session,
    ) = started_segment;

    let next_microphone_recording_file = next_reanchored_microphone_output_file(
        runtime,
        next_index,
        "recovering after system wake",
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
    // The tap recorded straight through the sleep (ADR 0052), so `segment_outputs`
    // — rebuilt from the fresh screen start — has to carry its file back or the
    // next rotation finalizes a segment that never mentions it and deletes it.
    merge_live_system_audio_continuation_into_segment_outputs(runtime, &mut segment_outputs);

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
    runtime.active_screen_session = active_screen_session;
    reanchor_active_segment_timing(runtime, "recovering after system wake")?;

    Ok(true)
}

#[cfg(target_os = "macos")]
pub(crate) fn recover_screen_capture_after_wake(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
) -> Result<bool, CaptureErrorResponse> {
    let metadata_snapshot_provider =
        core.map(|core| metadata::frame_metadata_snapshot_provider(core));
    let initial_privacy_filter = core.map(|core| privacy::collect_initial_privacy_filter(core));
    recover_screen_capture_after_wake_with_start_segment(
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
                0,
                0.0,
                microphone_capture::MicrophoneInactivityTailTrimActivityMode::PeakLevel,
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
