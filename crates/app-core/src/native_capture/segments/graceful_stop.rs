//! Graceful stops at a segment boundary: low disk and the license block.

use super::*;

/// Shared boundary graceful-stop mechanics: commit the (healthy) current
/// segment when asked, stop every live source, and end the session via
/// [`mark_runtime_session_failed`] (which already clears the suspension and,
/// through the segment loop's fall-out broadcast, refreshes the frontend and
/// the native status bar). Callers surface their own notification + log
/// (low-disk vs trial-lapse).
///
/// `commit_current_segment` controls whether the in-flight segment is clean-
/// finalized before ending: a boundary/recovery stop has a healthy current
/// segment to commit (only the *next* file can't be opened), whereas a mid-
/// segment write-failure stop has already discarded its partial and passes
/// `false` so no half-written file is committed.
#[cfg(target_os = "macos")]
pub(crate) fn graceful_stop_at_boundary(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
    commit_current_segment: bool,
) {
    // Clean-finalize what is safely closable: a still-writing current segment.
    // The mid-segment-fill caller already discarded its partial and passes
    // `false`, so only a boundary/recovery stop (healthy current segment, only
    // opening the *next* file is refused) commits here.
    if commit_current_segment {
        // Stop the screen session *before* committing: a ScreenCaptureKit `.mov`
        // is only finalized (moov atom written, file readable/convertible) by
        // `stop()` -> `finish_writing()`. `commit_suspended_screen_system_outputs`
        // reads/converts the `.mov` via `finalize_capture_outputs`, so reading it
        // while the session is still live yields an un-finalized file and drops
        // the current segment. Every other commit site stops first (see
        // `suspend_screen_capture`); the later
        // `stop_active_sessions_after_failure` is then idempotent for the screen
        // session. The mic is finalized through its own live session next, so it
        // must NOT be stopped here.
        if let Err(stop_error) =
            capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                active_session: &mut runtime.active_screen_session,
            })
        {
            crate::debug_log::log_warn(format!(
                "failed stopping screen session before boundary graceful-stop commit; continuing: [{}] {}",
                stop_error.code, stop_error.message
            ));
        }
        // Same rule for the tap's m4a: its writer must be finished before the
        // commit reads the file, or the segment is dropped as zero-duration.
        crate::native_capture::runtime::stop_system_audio_session(runtime);
        commit_suspended_screen_system_outputs(core, runtime, true);
        finalize_live_microphone_continuation_on_stop(core, runtime);
    }

    // Stop every live source so no writer keeps a file open as we end. (The mic
    // is included; boundary graceful stops are the only paths that stop it.)
    stop_active_sessions_after_failure(runtime);

    // End the session: this broadcasts native_capture_session_changed + refreshes
    // the status bar via the segment loop's internal-end fall-out, and clears the
    // suspension slot.
    crate::native_capture::runtime::mark_runtime_session_failed(runtime);
}

/// Graceful stop when free space has fallen below the critical reserve floor
/// (ADR 0040 "Graceful stop is spatial"). At or below `CRITICAL_FLOOR_BYTES` the
/// app's own SQLite DB / OCR / OS storage is at risk, so the session ends rather
/// than waiting for recovery.
///
/// Spatial, not timed: the caller passes the measured `free_bytes` that crossed
/// the critical floor purely for the single-line stop log.
///
/// Clears the low-disk warning (if present) and pushes the `error`-severity
/// "Recording stopped — disk full." notification.
#[cfg(target_os = "macos")]
pub(crate) fn graceful_stop_for_low_disk(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
    free_bytes: u64,
    commit_current_segment: bool,
) {
    graceful_stop_at_boundary(core, runtime, commit_current_segment);

    if let Some(core) = core {
        // Replace the (transient, cleared-on-resume) low-disk warning with the
        // persistent graceful-stop error: capture has stopped, not paused.
        clear_app_notification_by_id(core, LOW_DISK_NOTIFICATION_ID);
        push_error_app_notification(
            core,
            DISK_FULL_STOPPED_NOTIFICATION_ID,
            DISK_FULL_STOPPED_TITLE,
            DISK_FULL_STOPPED_MESSAGE,
            None,
            now_unix_ms(),
        );
    }

    crate::debug_log::log_error(format!(
        "capture stopped: recordings volume free space fell below the critical reserve floor; ending session to protect app storage ({} free, floor {})",
        disk_space::human_bytes(free_bytes),
        disk_space::human_bytes(disk_space::critical_threshold_bytes()),
    ));
}

/// Graceful stop when licensing blocks capture mid-session (trial lapsed, or the
/// stored key was revoked), caught at the rotation boundary. The current segment
/// is healthy — commit it, then end the session and surface the persistent
/// notification. `revoked` picks honest copy: a revoked key reads as "revoked",
/// never as a lapsed trial.
#[cfg(target_os = "macos")]
pub(crate) fn graceful_stop_for_license_block(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
    revoked: bool,
) {
    graceful_stop_at_boundary(core, runtime, true);

    let (notification_id, title, message) = if revoked {
        (
            REVOKED_STOPPED_NOTIFICATION_ID,
            REVOKED_STOPPED_TITLE,
            REVOKED_STOPPED_MESSAGE,
        )
    } else {
        (
            TRIAL_ENDED_STOPPED_NOTIFICATION_ID,
            TRIAL_ENDED_STOPPED_TITLE,
            TRIAL_ENDED_STOPPED_MESSAGE,
        )
    };

    if let Some(core) = core {
        // Immediate + persistent surfacing: the native dialog interrupts (the
        // user may not be looking at the app when their recording dies), and
        // the bell notification keeps the reason around after it's dismissed.
        core.host().message(crate::MessageDialog {
            title: title.to_string(),
            message: message.to_string(),
            kind: crate::DialogKind::Error,
        });
        push_error_app_notification(core, notification_id, title, message, None, now_unix_ms());
    }

    crate::debug_log::log_warn(if revoked {
        "capture stopped: license revoked during the session, Read-Only Mode — committed the current segment and ended the session at the rotation boundary"
    } else {
        "capture stopped: trial ended during the session, Read-Only Mode — committed the current segment and ended the session at the rotation boundary"
    });
}

/// Finalize a still-live microphone continuation when stopping, mirroring the
/// commit path [`commit_suspended_screen_system_outputs`] does for screen/system
/// audio. Best-effort: a finalize failure is logged and skipped (we are stopping
/// regardless), and a missing session/output is a no-op.
#[cfg(target_os = "macos")]
pub(crate) fn finalize_live_microphone_continuation_on_stop(
    core: Option<&Arc<AppCore>>,
    runtime: &mut NativeCaptureRuntime,
) {
    crate::native_capture::activity::process_pending_microphone_vad_frames(runtime);
    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let Some(session) = runtime.active_microphone_session.as_mut() else {
        return;
    };
    let finalization = match session.pause_output_file_for_inactivity(0, 0.0) {
        Ok(finalization) => finalization,
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to finalize microphone continuation during low-disk graceful stop; continuing stop: [{}] {}",
                error.code, error.message
            ));
            return;
        }
    };

    let Some(output_files) = runtime.current_segment_output_files.as_ref() else {
        return;
    };
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
    if let Err(error) = finalize_capture_outputs(
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
    ) {
        crate::debug_log::log_warn(format!(
            "failed to commit microphone continuation during low-disk graceful stop; continuing stop: [{}] {}",
            error.code, error.message
        ));
        return;
    }

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
    runtime.microphone_recording_file = None;
}
