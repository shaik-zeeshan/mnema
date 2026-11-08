//! Low-disk safety (ADR 0040): the boundary check, the mid-segment write
//! failure, the suspension and its recovery.

use super::*;

// ---------------------------------------------------------------------------
// Low-disk safety (ADR 0040): preflight + segment-open boundary check.
//
// Free space is checked exactly when a new segment file is about to be opened —
// the preflight is this check applied to the first segment, and each rotation is
// the same check applied to the next segment. There is no continuous poll. All
// three of these helpers are pure logic over a measured free-space reading, so
// the suspend/resume behavior is exercisable with a scripted probe (see the
// `free_space_probe` runtime seam) without a real full disk.
// ---------------------------------------------------------------------------

/// Best-effort low-disk decision at a segment-open boundary using the runtime's
/// recordings root and injected probe. `None` means the free space could not be
/// measured (no existing ancestor to stat, or the probe errored) — best-effort
/// semantics, so the caller must not act on `None`. A `Some(decision)` reflects
/// the measured reading against the next-segment estimate.
#[cfg(target_os = "macos")]
pub(crate) fn low_disk_decision_at_boundary(
    runtime: &NativeCaptureRuntime,
) -> Option<disk_space::LowDiskDecision> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    let free = disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())?;
    let estimate = next_segment_estimate_for_runtime(runtime);
    Some(disk_space::classify_free_space(free, estimate))
}

/// Preflight free-space check for `start_capture_runtime`, applied to the first
/// segment before any file is opened. Best-effort: an unmeasurable reading
/// (`None`) never blocks the start; only a measured shortfall below the pause
/// threshold refuses with `insufficient_disk_space`.
#[cfg(target_os = "macos")]
pub(crate) fn preflight_disk_space_check(
    recordings_root: &Path,
    effective_screen_bitrate_bps: Option<u32>,
    sources: &CaptureSources,
    segment_duration_seconds: u64,
    probe: disk_space::FreeSpaceProbe,
) -> Result<(), CaptureErrorResponse> {
    let bitrate_bps = effective_screen_bitrate_bps.unwrap_or(0) as u64;
    let audio_bytes_per_sec =
        disk_space::audio_bytes_per_sec_for_sources(sources.microphone, sources.system_audio);
    let estimate = disk_space::next_segment_estimate_bytes(
        bitrate_bps,
        audio_bytes_per_sec,
        segment_duration_seconds,
    );

    // Best-effort: if we cannot measure, do not block the start.
    let Some(free) = disk_space::measure_free_space(recordings_root, probe) else {
        return Ok(());
    };

    let pause_threshold = disk_space::pause_threshold_bytes(estimate);
    if free < pause_threshold {
        return Err(CaptureErrorResponse {
            code: "insufficient_disk_space".to_string(),
            message: format!(
                "Only {} free; Mnema needs ~{} to record.",
                disk_space::human_bytes(free),
                disk_space::human_bytes(pause_threshold)
            ),
        });
    }

    Ok(())
}

/// The outcome of a boundary low-disk check: whether the caller should skip
/// opening the next segment, and if so whether the session has been ended (a
/// graceful stop, so the loop must stop) or merely suspended (recovery resumes).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LowDiskBoundaryOutcome {
    /// Free space is sufficient (or unmeasurable): open the next segment.
    Proceed,
    /// Below the pause threshold but at/above the floor: entered a Low-Disk
    /// Suspension across all sources; skip the rotation, recovery will resume.
    Suspended,
    /// Below the critical floor: stopped the session gracefully; the segment loop
    /// must stop.
    Stopped,
}

/// Boundary low-disk check for the rotation path. Called when a new segment is
/// about to be opened (a rotation is due) and the runtime is not already
/// suspended. Best-effort: an unmeasurable reading returns `Proceed` and lets the
/// rotation proceed.
///
/// - `Pause` (free below the pause threshold but at/above the reserve floor):
///   enter the Low-Disk Suspension via the shared suspend entry and return
///   `Suspended` so the caller skips opening the next segment.
/// - `Critical` (free below the reserve floor): the app's own storage is at risk,
///   so stop gracefully (commit the healthy current segment, end the session) and
///   return `Stopped`.
#[cfg(target_os = "macos")]
pub(crate) fn maybe_suspend_for_low_disk_at_boundary(
    core: &Arc<AppCore>,
    runtime: &mut NativeCaptureRuntime,
) -> LowDiskBoundaryOutcome {
    // Recovery owns an already-suspended session; never re-enter here.
    if runtime.capture_suspension.is_some() {
        return LowDiskBoundaryOutcome::Proceed;
    }

    let Some(decision) = low_disk_decision_at_boundary(runtime) else {
        // Best-effort: cannot measure -> let the rotation proceed.
        return LowDiskBoundaryOutcome::Proceed;
    };

    match decision {
        disk_space::LowDiskDecision::Sufficient => LowDiskBoundaryOutcome::Proceed,
        // Below the critical reserve floor: stop gracefully rather than suspend.
        // The current segment is still healthy (only the *next* file can't be
        // opened), so commit it before ending.
        disk_space::LowDiskDecision::Critical => {
            let free = low_disk_free_at_boundary(runtime)
                .unwrap_or_else(disk_space::critical_threshold_bytes);
            graceful_stop_for_low_disk(Some(core), runtime, free, true);
            LowDiskBoundaryOutcome::Stopped
        }
        disk_space::LowDiskDecision::Pause => {
            let estimate = next_segment_estimate_for_runtime(runtime);
            let pause_threshold = disk_space::pause_threshold_bytes(estimate);
            let error = CaptureErrorResponse {
                code: LOW_DISK_NOTIFICATION_ID.to_string(),
                message: format!(
                    "Capture paused: recordings volume low on free space (need ~{} to open the next segment).",
                    disk_space::human_bytes(pause_threshold)
                ),
            };
            if let Err(stop_error) =
                suspend_screen_capture(Some(core), runtime, &error, CaptureSuspensionKind::LowDisk)
            {
                crate::debug_log::log_warn(format!(
                    "low-disk suspension could not stop capture; preserving runtime state: [{}] {}",
                    stop_error.code, stop_error.message
                ));
                return LowDiskBoundaryOutcome::Proceed;
            }
            crate::debug_log::log_info(
                "capture paused: recordings volume low on disk; suspending all sources until free space recovers",
            );
            LowDiskBoundaryOutcome::Suspended
        }
    }
}

/// Re-measure the raw free bytes at the recordings root for the boundary stop
/// log. `None` when unmeasurable (the caller substitutes the floor for the log).
#[cfg(target_os = "macos")]
pub(crate) fn low_disk_free_at_boundary(runtime: &NativeCaptureRuntime) -> Option<u64> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())
}

/// Best-effort discard of a mid-segment partial that failed to write because the
/// disk filled (ADR 0040 "Mid-segment disk-full → no corrupt segment"). There is
/// no temp-file-then-atomic-rename, so the writer leaves a corrupt/partial file at
/// its *final* path; this deletes each such path (`remove_file`, NotFound ignored)
/// and commits NO Capture Segment row — the caller drops the partial entirely and
/// then suspends (Pause) or stops (Critical).
///
/// Returns `false` to signal "no segment row committed" so a caller can treat the
/// rotation's `previous_segment_committed` flag uniformly. Pure I/O over the paths
/// it is given, so it is unit-testable without a real full disk.
#[cfg(target_os = "macos")]
pub(crate) fn discard_partial_segment_on_disk_full(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) -> bool {
    // The in-flight recording-file handles are the live final paths; the
    // committed output_files list may also carry the partial. Delete every
    // distinct final path we know about so no corrupt file survives.
    let mut paths: Vec<String> = Vec::new();
    let mut push = |candidate: Option<&str>| {
        if let Some(path) = candidate {
            if !path.is_empty() && !paths.iter().any(|existing| existing == path) {
                paths.push(path.to_string());
            }
        }
    };
    push(recording_file);
    push(microphone_recording_file);
    push(system_audio_recording_file);
    if let Some(output_files) = output_files {
        push(output_files.screen_file.as_deref());
        push(output_files.microphone_file.as_deref());
        push(output_files.system_audio_file.as_deref());
        for path in output_files
            .screen_files
            .iter()
            .chain(output_files.microphone_files.iter())
            .chain(output_files.system_audio_files.iter())
        {
            push(Some(path.as_str()));
        }
    }

    for path in &paths {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::debug_log::log_warn(format!(
                    "failed removing disk-full partial capture file {path}: {error}"
                ));
            }
        }
    }

    // No Capture Segment row is committed for the discarded partial.
    false
}

/// Re-probe free space at the recordings root after a writer append/finalize
/// failure to decide whether the failure coincides with low free space (i.e. it
/// is a disk-full failure). `None` means the failure did NOT coincide with low
/// free space (healthy disk, or unmeasurable — keep the existing failure
/// behavior); `Some((decision, free))` means low disk and the caller should
/// discard the partial and then suspend (`Pause`) or stop (`Critical`).
///
/// Best-effort: an unmeasurable probe returns `None` so an unrelated write
/// failure on a healthy-but-unstatable volume still falls back to existing
/// behavior and is never forced into a stop.
#[cfg(target_os = "macos")]
pub(crate) fn classify_write_failure_disk_full(
    runtime: &NativeCaptureRuntime,
) -> Option<(disk_space::LowDiskDecision, u64)> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    let free = disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())?;
    let estimate = next_segment_estimate_for_runtime(runtime);
    match disk_space::classify_free_space(free, estimate) {
        // A failure while free space is healthy is unrelated to disk space: keep
        // the existing failure behavior (return None).
        disk_space::LowDiskDecision::Sufficient => None,
        decision @ (disk_space::LowDiskDecision::Pause | disk_space::LowDiskDecision::Critical) => {
            Some((decision, free))
        }
    }
}

/// What a mid-segment writer failure resolved to once free space was re-probed.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteFailureDiskFullOutcome {
    /// The failure did not coincide with low free space (healthy or unmeasurable
    /// disk): keep the EXISTING failure behavior unchanged.
    NotDiskFull,
    /// Disk-full at/above the floor: the partial was discarded and a Low-Disk
    /// Suspension entered; the loop should skip the rotation and let recovery
    /// resume once free space returns.
    Suspended,
    /// Disk-full below the floor: the partial was discarded and the session was
    /// stopped gracefully; the loop should stop.
    Stopped,
}

/// Reactive mid-segment disk-full handling (ADR 0040 "Mid-segment disk-full → no
/// corrupt segment"). Called from the segment loop's existing writer
/// append/finalize-failure handling. Re-probes free space at the recordings root;
/// if the failure coincides with low free space it is treated as disk-full:
/// best-effort delete the partial file at its FINAL path (committing NO Capture
/// Segment row for it) and then either enter a Low-Disk Suspension (`Pause`) or
/// stop gracefully (`Critical`).
///
/// When free space is healthy (or unmeasurable), returns `NotDiskFull` and does
/// nothing — the caller keeps its existing failure behavior. This is purely
/// additive for the disk-full case.
#[cfg(target_os = "macos")]
pub(crate) fn handle_mid_segment_write_failure_for_low_disk(
    core: &Arc<AppCore>,
    runtime: &mut NativeCaptureRuntime,
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) -> WriteFailureDiskFullOutcome {
    let Some((decision, free)) = classify_write_failure_disk_full(runtime) else {
        // Healthy or unmeasurable disk: not a disk-full failure — leave the
        // existing failure behavior intact.
        return WriteFailureDiskFullOutcome::NotDiskFull;
    };

    // Disk-full: discard the partial at its final path(s) and commit no row. The
    // discard is the single guarantee against a corrupt/locked file surviving, run
    // before either the suspend or the stop branch.
    let _committed = discard_partial_segment_on_disk_full(
        output_files,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
    );

    // Stop every live source so no broken writer keeps a file open.
    stop_active_sessions_after_failure(runtime);

    match decision {
        // Below the reserve floor: stop gracefully. The partial was already
        // discarded, so do not re-commit a current segment.
        disk_space::LowDiskDecision::Critical => {
            graceful_stop_for_low_disk(Some(core), runtime, free, false);
            crate::debug_log::log_error(format!(
                "mid-segment disk-full below the reserve floor; discarded the partial and stopped gracefully ({} free)",
                disk_space::human_bytes(free)
            ));
            WriteFailureDiskFullOutcome::Stopped
        }
        // At/above the floor: discard the partial and enter the Low-Disk
        // Suspension so recovery resumes once free space returns.
        disk_space::LowDiskDecision::Pause => {
            enter_low_disk_suspension_after_partial_discard(core, runtime);
            crate::debug_log::log_warn(format!(
                "mid-segment disk-full; discarded the partial and suspended all sources until free space recovers ({} free)",
                disk_space::human_bytes(free)
            ));
            WriteFailureDiskFullOutcome::Suspended
        }
        disk_space::LowDiskDecision::Sufficient => {
            // Unreachable: classify_write_failure_disk_full returns None for
            // Sufficient. Kept total for safety — fall back to existing behavior.
            WriteFailureDiskFullOutcome::NotDiskFull
        }
    }
}

/// Enter a Low-Disk Suspension after a mid-segment partial has already been
/// discarded. Unlike [`suspend_screen_capture`], this does NOT try
/// to commit the in-flight (failed) segment — its writers are broken and its
/// partial was just deleted. It only clears the in-flight segment refs, sets the
/// suspension slot across all sources, and pushes the low-disk warning. Recovery
/// (`resume_all_sources_after_low_disk`) starts a fresh segment from
/// `requested_sources` + `current_segment_index`, so no live segment state is
/// needed to resume.
#[cfg(target_os = "macos")]
pub(crate) fn enter_low_disk_suspension_after_partial_discard(
    core: &Arc<AppCore>,
    runtime: &mut NativeCaptureRuntime,
) {
    runtime.active_microphone_session = None;
    runtime.microphone_recording_file = None;
    runtime.recording_file = None;
    runtime.system_audio_recording_file = None;
    runtime.current_segment_output_files = None;
    runtime.current_segment_sources = Some(explicit_privacy_suspension_sources(runtime));

    let estimate = next_segment_estimate_for_runtime(runtime);
    let pause_threshold = disk_space::pause_threshold_bytes(estimate);
    let error = CaptureErrorResponse {
        code: LOW_DISK_NOTIFICATION_ID.to_string(),
        message: format!(
            "Capture paused: recordings volume filled mid-segment (need ~{} to open the next segment).",
            disk_space::human_bytes(pause_threshold)
        ),
    };
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(
        CaptureSuspensionKind::LowDisk,
        &error,
    ));

    push_warning_app_notification(
        core,
        LOW_DISK_NOTIFICATION_ID,
        "Capture paused — low disk space",
        "Capture paused — low disk space. Free up space and recording resumes automatically.",
        None,
        now_unix_ms(),
    );
}

/// Whether a Low-Disk Suspension may resume right now, re-probing free space at
/// the recordings root. `None` means free space is currently unmeasurable (stay
/// suspended, keep retrying); `Some(true)` means it has climbed to the resume
/// threshold (hysteresis) and `Some(false)` means it is still short. Pure over
/// the injected probe — the testable resume-decision seam mirroring the gate in
/// [`attempt_low_disk_recovery`] (which inlines the same `disk_space` primitives
/// because it also needs the measured free bytes for classify + logging).
#[cfg(all(target_os = "macos", test))]
pub(crate) fn low_disk_can_resume(runtime: &NativeCaptureRuntime) -> Option<bool> {
    let recordings_root = recordings_root_for_runtime(runtime)?;
    let free = disk_space::measure_free_space(&recordings_root, runtime.free_space_probe())?;
    let estimate = next_segment_estimate_for_runtime(runtime);
    Some(disk_space::can_resume(free, estimate))
}

/// Recovery path for a [`CaptureSuspensionKind::LowDisk`] suspension. Re-probes
/// free space and, once it has recovered to the resume threshold, restarts a
/// fresh segment across all suspended sources — including the microphone, which
/// only LowDisk stops. Otherwise it stays suspended and keeps retrying.
#[cfg(target_os = "macos")]
pub(crate) fn attempt_low_disk_recovery(
    core: &Arc<AppCore>,
    runtime: &mut NativeCaptureRuntime,
) -> PrivacySuspensionRecoveryOutcome {
    let recordings_root = match recordings_root_for_runtime(runtime) {
        Some(root) => root,
        // No planner to derive a recordings root from: cannot probe, so wait.
        None => return PrivacySuspensionRecoveryOutcome::RetryPending,
    };
    let estimate = next_segment_estimate_for_runtime(runtime);
    let free = match disk_space::measure_free_space(&recordings_root, runtime.free_space_probe()) {
        Some(free) => free,
        // Best-effort: cannot measure right now -> stay suspended, retry later.
        None => return PrivacySuspensionRecoveryOutcome::RetryPending,
    };

    match disk_space::classify_free_space(free, estimate) {
        // Free space kept falling while suspended and crossed the critical reserve
        // floor: the app's own storage is now at risk, so stop gracefully rather
        // than keep waiting. The suspended session has no live current segment to
        // commit (it was committed at suspend time), so do not re-commit here.
        disk_space::LowDiskDecision::Critical => {
            graceful_stop_for_low_disk(Some(core), runtime, free, false);
            return PrivacySuspensionRecoveryOutcome::NotSuspended;
        }
        // Still below the pause threshold but above the floor: stay suspended.
        disk_space::LowDiskDecision::Pause => {
            return PrivacySuspensionRecoveryOutcome::RetryPending;
        }
        // Sufficient (cleared the pause threshold) is necessary but not sufficient
        // to resume: hysteresis requires the higher resume threshold. Fall through
        // to the resume gate below.
        disk_space::LowDiskDecision::Sufficient => {}
    }

    if !disk_space::can_resume(free, estimate) {
        // In the hysteresis band: cleared pause but not yet resume. Stay suspended
        // so we don't flap back into a pause as soon as one more segment is opened.
        return PrivacySuspensionRecoveryOutcome::RetryPending;
    }

    match resume_all_sources_after_low_disk(core, runtime) {
        Ok(()) => {
            runtime.capture_suspension = None;
            clear_app_notification_by_id(core, LOW_DISK_NOTIFICATION_ID);
            crate::debug_log::log_info(format!(
                "recovered: recordings volume free space back above resume threshold; restarted all sources ({} free)",
                disk_space::human_bytes(free)
            ));
            PrivacySuspensionRecoveryOutcome::Recovered
        }
        Err(error) => {
            if let Some(suspension) = runtime.capture_suspension.as_mut() {
                suspension.record_recovery_failure(&error);
            }
            crate::debug_log::log_warn(format!(
                "low-disk recovery restart failed; sources remain suspended: [{}] {}",
                error.code, error.message
            ));
            PrivacySuspensionRecoveryOutcome::RetryPending
        }
    }
}

/// The sources low-disk recovery restarts: each requested one inactivity has not
/// paused. Not a microphone lent to voice enrollment (restarted when handed
/// back) or held while the Mac is away (lock is away) — live away too: recovery
/// runs before the tick that first sees the Mac away sets the hold. A
/// microphone left down here is the tick's to reopen once nothing holds it.
#[cfg(target_os = "macos")]
pub(crate) fn low_disk_recovery_sources(
    runtime: &NativeCaptureRuntime,
    requested_sources: &CaptureSources,
) -> CaptureSources {
    CaptureSources {
        screen: requested_sources.screen && !runtime.inactivity.is_screen_paused(),
        microphone: requested_sources.microphone
            && !runtime.inactivity.is_microphone_paused()
            && !runtime.microphone_lent_out
            && !runtime.microphone_away_hold
            && !crate::native_capture::microphone_away::mac_is_away(runtime),
        system_audio: requested_sources.system_audio
            && !runtime.inactivity.is_screen_paused()
            && !runtime.inactivity.is_system_audio_paused(),
    }
}

/// Restart a fresh segment across every source a Low-Disk Suspension stopped:
/// screen + system audio (via the shared privacy-filter segment start) and the
/// microphone (its own native session). Mutates the runtime to point at the new
/// segment on success; leaves it suspended on error so the caller can retry.
#[cfg(target_os = "macos")]
pub(crate) fn resume_all_sources_after_low_disk(
    core: &Arc<AppCore>,
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    let Some(requested_sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Requested sources missing while recovering low-disk suspension".to_string(),
        });
    };

    let recover_sources = low_disk_recovery_sources(runtime, &requested_sources);

    let next_index = next_emitted_segment_index(runtime.current_segment_index);

    // --- Screen ---
    let mut next_segment_outputs = empty_output_files();
    let mut next_recording_file: Option<String> = None;

    if recover_sources.screen {
        let Some(screen_planner) = screen_planner_for_runtime(runtime).cloned() else {
            return Err(CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: "Capture screen planner missing while recovering low-disk suspension"
                    .to_string(),
            });
        };
        let segment_dir = screen_planner.segment_dir(next_index);
        let screen_output_file = screen_planner.segment_screen_output(next_index);
        let screen_only_sources = CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        };

        let (
            segment_outputs,
            recording_file,
            _microphone_recording_file,
            active_screen_session,
            _active_microphone_session,
        ) = start_segment_with_current_privacy_filter(
            core,
            &segment_dir,
            Some(&screen_output_file),
            &screen_only_sources,
            runtime.screen_frame_rate,
            &runtime.screen_resolution,
            runtime.effective_screen_bitrate_bps,
            None,
            runtime.frame_artifact_tx.clone(),
            None,
        )?;

        next_segment_outputs = segment_outputs;
        next_recording_file = recording_file;
        runtime.active_screen_session = active_screen_session;
    }

    // --- System audio (its own tap) ---
    // Low disk is the one suspension that stops the tap, so it is the one that
    // restarts it. A failure here only costs system audio; the other sources
    // recover regardless.
    let next_system_audio_recording_file = match start_system_audio_family_for_runtime(
        runtime,
        privacy::collect_initial_privacy_filter(core).excluded_bundle_ids(),
        next_index,
        "recovering low-disk suspension",
    ) {
        Ok(output_file) => output_file,
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to restart system audio after low-disk recovery; continuing without it: [{}] {}",
                error.code, error.message
            ));
            None
        }
    };
    if let Some(system_audio_output_file) = next_system_audio_recording_file.as_ref() {
        set_current_system_audio_output_file(
            &mut next_segment_outputs,
            system_audio_output_file.clone(),
        );
    }

    // --- Microphone (separate native session) ---
    // The screen session above is now live on `runtime`. If any
    // microphone-restart step fails we must stop the live sessions before
    // returning Err: the caller (`attempt_low_disk_recovery`) records the failure
    // but leaves the runtime suspended and retries every ~10s, so a leftover live
    // screen session is an orphaned writer the next attempt would overwrite —
    // leaking it (there is no `Drop` that stops a screen session) and opening a
    // second writer over overlapping segment files. Mirrors the rollback the
    // combined start path already does on a mic-start failure.
    if recover_sources.microphone {
        let restart_result = (|| -> Result<(), CaptureErrorResponse> {
            ensure_microphone_planner_for_runtime(runtime, "recovering low-disk suspension")?;
            refresh_runtime_planner_dates(runtime);
            let microphone_output_file =
                crate::native_capture::microphone::next_microphone_output_file_for_runtime(
                    runtime,
                )?;
            let session =
                microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
                    &microphone_output_file,
                    runtime.microphone_device_id_for_capture.as_deref(),
                )?;
            runtime.active_microphone_session = Some(session);
            runtime.microphone_recording_file = Some(microphone_output_file.clone());
            set_current_microphone_output_file(&mut next_segment_outputs, microphone_output_file);
            Ok(())
        })();
        if let Err(error) = restart_result {
            stop_active_sessions_after_failure(runtime);
            return Err(error);
        }
    }

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(next_segment_outputs);
    runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
        &requested_sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    );
    runtime.recording_file = next_recording_file;
    runtime.system_audio_recording_file = next_system_audio_recording_file;

    if let Err(error) = reanchor_active_segment_timing(runtime, "recovering low-disk suspension") {
        crate::debug_log::log_warn(format!(
            "failed to reanchor segment timing after low-disk recovery: [{}] {}",
            error.code, error.message
        ));
    }

    Ok(())
}
