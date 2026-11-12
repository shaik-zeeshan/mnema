//! Privacy and display-unavailable suspension: stopping the screen, committing
//! the suspended outputs, and the bounded recovery that restarts it (ADR 0021).

use super::*;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrivacySuspensionRecoveryOutcome {
    NotSuspended,
    RetryPending,
    RestartRequired,
    Recovered,
}

#[cfg(target_os = "macos")]
pub(crate) fn screen_system_output_files(
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
pub(crate) fn explicit_privacy_suspension_sources(
    runtime: &NativeCaptureRuntime,
) -> CaptureSources {
    privacy_suspended_sources_for_runtime_state(runtime, runtime.inactivity.is_microphone_paused())
        .unwrap_or(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        })
}

#[cfg(target_os = "macos")]
pub(crate) fn active_sources_for_runtime_pause_state(
    runtime: &NativeCaptureRuntime,
    requested_sources: &CaptureSources,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
) -> Option<CaptureSources> {
    if runtime.capture_suspension.is_some() {
        return privacy_suspended_sources_for_runtime_state(runtime, microphone_paused);
    }

    active_sources_for_inactivity_paused_state(
        requested_sources,
        screen_paused,
        microphone_paused,
        system_audio_paused,
    )
}

/// Suspends screen capture. Despite the name, system audio no longer rides
/// along: it runs on a process tap with no display dependency, so this leaves it
/// alone exactly as it leaves the microphone alone (ADR 0021, amended by ADR
/// 0052). Lock and display sleep stop both through their away holds instead.
///
/// Low disk is the exception, and it is not about the screen: every source writes
/// to the same volume, so [`CaptureSuspensionKind::LowDisk`] still stops the
/// microphone and the tap along with it (ADR 0040).
#[cfg(target_os = "macos")]
pub(crate) fn suspend_screen_capture(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
    error: &CaptureErrorResponse,
    kind: CaptureSuspensionKind,
) -> Result<(), CaptureErrorResponse> {
    if let Err(stop_error) =
        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
            active_session: &mut runtime.active_screen_session,
        })
    {
        if capture_screen::should_preserve_runtime_on_stop_error(&stop_error) {
            return Err(stop_error);
        }
    }
    if kind == CaptureSuspensionKind::LowDisk {
        let _ = stop_microphone_session_for_runtime(runtime);
        runtime.active_microphone_session = None;
        // The stop above finalized the current segment's mic `.m4a` on
        // disk. Commit it before dropping the handle: LowDisk uniquely stops the
        // mic, and `commit_suspended_screen_system_outputs` below only commits
        // screen/system audio (microphone: false), so without this the finalized
        // mic file is orphaned — a real file on disk with no audio_segment row,
        // never transcribed and untracked by retention. That is up to ~5 min of
        // microphone audio lost on every low-disk pause; the normal rotation and
        // inactivity-pause paths both commit this file (ADR 0040 "the current
        // segment is still healthy ... so commit it before ending").
        commit_suspended_microphone_outputs(core, runtime, "entering low-disk suspension");
        if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
            crate::native_capture_output::clear_current_microphone_output_file(output_files);
        }
        runtime.microphone_recording_file = None;
        // The tap writes to the same full volume, so it stops here too — and its
        // in-flight segment is committed by `commit_suspended_screen_system_outputs`
        // below, which still carries system audio for exactly this case.
        crate::native_capture::runtime::stop_system_audio_session(runtime);
    }
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(kind, error));
    runtime.current_segment_sources = Some(explicit_privacy_suspension_sources(runtime));
    // Commit the in-flight segment for every suspension kind, including
    // DisplayUnavailable: the stop above finalizes the writers even when the
    // delegate already reported the stream dead (terminated streams skip the
    // doomed second stop but still finish their writers), so the tail `.mov` is
    // openable and committing it preserves the last partial segment instead of
    // orphaning it. A finalize failure is logged and skipped inside.
    commit_suspended_screen_system_outputs(
        core,
        runtime,
        crate::native_capture::runtime::system_audio_stops_with_suspension(runtime),
    );
    runtime.recording_file = None;
    // Kept only while the tap is genuinely still writing it — a stale path from a
    // source that is not recording would otherwise be re-committed later.
    runtime.system_audio_recording_file = live_system_audio_continuation_file(runtime);
    preserve_live_audio_continuation_outputs(runtime);

    // Notify on entering a Low-Disk Suspension (unlike the silent
    // display-unavailable case): low disk only heals if the user frees space.
    // Emitting it from the shared suspend entry means Slice 5's mid-segment fill
    // path, which reuses this entry, gets the warning for free.
    if kind == CaptureSuspensionKind::LowDisk {
        if let Some(core) = core {
            push_warning_app_notification(
                core,
                LOW_DISK_NOTIFICATION_ID,
                "Capture paused — low disk space",
                "Capture paused — low disk space. Free up space and recording resumes automatically.",
                None,
                now_unix_ms(),
            );
        }
    }

    Ok(())
}

/// Commits the in-flight segment when screen capture stops or suspends.
///
/// `system_audio_stopped` must say whether the caller has already torn the tap
/// down. Committing a file the tap is still writing closes a segment mid-write;
/// skipping one it has finished orphans it. Only the caller knows which it did,
/// so it is passed rather than inferred.
#[cfg(target_os = "macos")]
pub(crate) fn commit_suspended_screen_system_outputs(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
    system_audio_stopped: bool,
) {
    let Some(requested_sources) = runtime.requested_sources.as_ref() else {
        return;
    };
    let system_audio_stopped = requested_sources.system_audio && system_audio_stopped;
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
    if !system_audio_stopped {
        // The tap outlives every suspension except LowDisk (ADR 0052), so the file
        // it is still writing stays out of this commit — but generations a
        // mid-segment rebuild already finalized belong to this segment and are
        // orphaned if dropped here.
        if let Some(live_file) = runtime.system_audio_recording_file.as_deref() {
            strip_live_system_audio_output_file(&mut output_files, live_file);
        }
        if output_files.screen_file.is_none()
            && output_files.screen_files.is_empty()
            && output_files.system_audio_file.is_none()
            && output_files.system_audio_files.is_empty()
        {
            return;
        }
    }

    match finalize_capture_outputs(
        Some(&mut output_files),
        runtime.recording_file.as_deref(),
        None,
        system_audio_stopped
            .then(|| runtime.system_audio_recording_file.as_deref())
            .flatten(),
        Some(&commit_sources),
    ) {
        Ok(()) => {
            if let Some(committed) = runtime.output_files.as_mut() {
                append_committed_segment_output_files(committed, &output_files);
            }
            persist_committed_audio_segments(
                core,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                Some(&output_files),
            );
            warm_scrub_previews_for_committed_screen_outputs(core, Some(&output_files));
        }
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to finalize suspended screen outputs; continuing privacy handling: [{}] {}",
                error.code, error.message
            ));
        }
    }
}

/// Commit the microphone files a stop already closed and booked in
/// `current_segment_output_files`: on entering a Low-Disk Suspension (the only
/// kind that stops the mic), and on an idle pause with no session left to
/// finalize. Mirrors [`commit_suspended_screen_system_outputs`]
/// for the mic source: validate + finalize the already-closed file, append it to
/// the committed `output_files`, and persist its audio segment row. Must run
/// before `preserve_live_audio_continuation_outputs` clears
/// `current_segment_output_files` and before `microphone_recording_file` is
/// nulled. Best-effort: a finalize failure is logged and skipped; a missing mic
/// output is a no-op.
#[cfg(target_os = "macos")]
pub(crate) fn commit_suspended_microphone_outputs(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
    context: &str,
) {
    let Some(output_files) = runtime.current_segment_output_files.as_ref() else {
        return;
    };
    if output_files.microphone_file.is_none() && output_files.microphone_files.is_empty() {
        return;
    }
    let mut microphone_outputs = empty_output_files();
    microphone_outputs.microphone_file = output_files.microphone_file.clone();
    microphone_outputs.microphone_files = output_files.microphone_files.clone();

    match finalize_capture_outputs(
        Some(&mut microphone_outputs),
        None,
        runtime.microphone_recording_file.as_deref(),
        None,
        Some(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
    ) {
        Ok(()) => {
            if let Some(committed) = runtime.output_files.as_mut() {
                append_committed_segment_output_files(committed, &microphone_outputs);
            }
            persist_committed_audio_segments(
                core,
                runtime.source_sessions.as_ref(),
                runtime.segment_schedule.as_ref(),
                runtime.current_segment_index,
                Some(&microphone_outputs),
            );
        }
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to commit microphone outputs while {context}; continuing: [{}] {}",
                error.code, error.message
            ));
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn attempt_privacy_suspension_recovery(
    core: &Arc<AppCore>,
    runtime: &mut NativeCaptureRuntime,
) -> PrivacySuspensionRecoveryOutcome {
    let suspension_kind = match runtime.capture_suspension.as_ref() {
        Some(suspension) => suspension.kind,
        None => return PrivacySuspensionRecoveryOutcome::NotSuspended,
    };

    // Low disk recovers on a different axis (free space, not a returning display
    // or a re-applicable privacy filter) and uniquely must restart the mic too, so
    // it owns a dedicated recovery path.
    if suspension_kind == CaptureSuspensionKind::LowDisk {
        return attempt_low_disk_recovery(core, runtime);
    }

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
        .capture_suspension
        .as_ref()
        .is_some_and(CaptureSuspension::can_retry);
    if !can_retry {
        return PrivacySuspensionRecoveryOutcome::RestartRequired;
    }

    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return PrivacySuspensionRecoveryOutcome::RestartRequired;
    };
    // Screen-scoped recovery: only the screen was suspended, so only the screen
    // restarts here (ADR 0021, amended). System audio kept its tap running
    // throughout and needs no recovery.
    let recover_sources = CaptureSources {
        screen: requested_sources.screen && !runtime.inactivity.is_screen_paused(),
        microphone: false,
        system_audio: false,
    };
    if !recover_sources.screen {
        runtime.capture_suspension = None;
        return PrivacySuspensionRecoveryOutcome::NotSuspended;
    }

    let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
        if let Some(suspension) = runtime.capture_suspension.as_mut() {
            suspension.record_recovery_failure(&CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: "Capture screen planner missing while recovering privacy suspension"
                    .to_string(),
            });
        }
        return PrivacySuspensionRecoveryOutcome::RetryPending;
    };

    let next_index = next_emitted_segment_index(runtime.current_segment_index);
    let segment_dir = screen_planner.segment_dir(next_index);
    let screen_output_file = screen_planner.segment_screen_output(next_index);

    let started_segment = start_segment_with_current_privacy_filter(
        core,
        &segment_dir,
        Some(&screen_output_file),
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
        active_screen_session,
        _active_microphone_session,
    ) = match started_segment {
        Ok(value) => value,
        Err(error) => {
            if let Some(suspension) = runtime.capture_suspension.as_mut() {
                suspension.record_recovery_failure(&error);
                if suspension.status == CaptureSuspensionStatus::RestartRequired {
                    crate::debug_log::log_error(format!(
                        "privacy recovery restart attempts exhausted; screen capture requires manual stop/start: [{}] {}",
                        error.code, error.message
                    ));
                    push_privacy_recovery_restart_required_notification(core);
                    return PrivacySuspensionRecoveryOutcome::RestartRequired;
                }
            }
            crate::debug_log::log_warn(format!(
                "privacy recovery restart failed; screen capture remains suspended: [{}] {}",
                error.code, error.message
            ));
            return PrivacySuspensionRecoveryOutcome::RetryPending;
        }
    };

    commit_suspended_screen_system_outputs(Some(core), runtime, false);
    merge_live_microphone_continuation_into_segment_outputs(runtime, &mut segment_outputs);
    merge_live_system_audio_continuation_into_segment_outputs(runtime, &mut segment_outputs);
    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        false,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.recording_file = recording_file;
    runtime.active_screen_session = active_screen_session;
    runtime.capture_suspension = None;
    if let Err(error) = reanchor_active_segment_timing(runtime, "recovering privacy suspension") {
        crate::debug_log::log_warn(format!(
            "failed to reanchor segment timing after privacy recovery: [{}] {}",
            error.code, error.message
        ));
    }

    PrivacySuspensionRecoveryOutcome::Recovered
}
