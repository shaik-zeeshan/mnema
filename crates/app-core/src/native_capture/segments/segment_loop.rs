//! The segment loop thread: privacy refresh, suspension recovery, and the
//! lifecycle's inactivity and rotation ticks.

use super::*;

#[cfg(target_os = "macos")]
pub(crate) fn spawn_segment_loop(core: Arc<AppCore>) -> SegmentLoopControl {
    let control = SegmentLoopControl::new();
    let worker_control = control.clone();

    thread::spawn(move || {
        let mut last_privacy_filter_poll = Instant::now()
            .checked_sub(PRIVACY_FILTER_POLL_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut last_suspension_recovery_attempt = Instant::now()
            .checked_sub(DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut last_low_disk_recovery_attempt = Instant::now()
            .checked_sub(LOW_DISK_RECOVERY_INTERVAL)
            .unwrap_or_else(Instant::now);
        // Starts empty, so the first privacy poll of a session with exclusions
        // forwards them once more than it needs to. Harmless: the watcher diffs
        // the resulting process-object list and only rebuilds when it moves.
        let mut last_system_audio_excluded_bundle_ids: Vec<String> = Vec::new();
        loop {
            // A Rust-spawned thread has no autorelease pool, so every
            // autoreleased object this tick touches leaks for the life of the
            // process. The privacy-filter path alone enumerates ScreenCaptureKit
            // shareable content (one `SCRunningApplication` + bundle
            // `NSURL`/`CFString`s per running app) and had grown the heap to
            // ~5M live objects / 770MB after 40h of recording. Per-iteration so
            // the pool drains on every tick, including the `break` paths.
            let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
            let sleep_duration = {
                let capture_state = core.state::<NativeCaptureState>();
                let runtime = match capture_state.lock() {
                    Ok(runtime) => runtime,
                    Err(_) => break,
                };
                let runtime = runtime.runtime();

                if !runtime.is_running {
                    break;
                }

                let Some(schedule) = runtime.segment_schedule.as_ref() else {
                    break;
                };
                let Some(clock) = runtime.capture_clock.as_ref() else {
                    break;
                };

                segment_loop_sleep_duration(schedule, clock)
            };

            if !sleep_duration.is_zero() {
                worker_control.wait_timeout(sleep_duration);
            }

            if worker_control.stop.load(Ordering::Relaxed) {
                break;
            }

            if last_privacy_filter_poll.elapsed() >= PRIVACY_FILTER_POLL_INTERVAL {
                privacy::refresh::request_privacy_filter_refresh(
                    &core,
                    privacy::PrivacyRefreshReason::FallbackPoll,
                );
                last_privacy_filter_poll = Instant::now();
            }
            privacy::refresh::maybe_start_privacy_filter_collection(&core);
            let privacy_filter_update = privacy::take_completed_privacy_filter_update(&core);
            privacy::refresh::maybe_start_privacy_filter_collection(&core);

            // The privacy list reaches the tap here rather than through
            // `apply_privacy_filter_update`, which no-ops without a live screen
            // session — system audio has no screen session to require, and a
            // privacy edit must reach it in an audio-only recording too.
            //
            // Diffed off the lock because the update channel republishes the same
            // decision every fallback poll; only a real edit is worth waking the
            // tap for.
            let next_excluded_bundle_ids = privacy_filter_update
                .as_ref()
                .map(|update| update.update.excluded_bundle_ids().to_vec());
            let system_audio_excludes_moved = next_excluded_bundle_ids
                .as_ref()
                .is_some_and(|next| *next != last_system_audio_excluded_bundle_ids);

            let capture_state = core.state::<NativeCaptureState>();
            let mut runtime = match capture_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => break,
            };

            if !runtime.runtime().is_running || worker_control.stop.load(Ordering::Relaxed) {
                break;
            }

            if system_audio_excludes_moved {
                let next = next_excluded_bundle_ids.unwrap_or_default();
                // ponytail: this reconcile makes a synchronous Core Audio call
                // under the capture lock, which gates the tick and every Tauri
                // capture command. The ceiling is coreaudiod: while it is
                // restarting (`killall coreaudiod` is one of ADR 0052's own
                // drills) that call is not fast, and the stall surfaces as a UI
                // freeze. The diff above keeps it to real privacy edits, so the
                // window is narrow rather than closed. Closing it means handing
                // the exclude watcher out of the session behind an Arc and
                // reconciling off the lock — `poll()` is no escape, it runs on the
                // tick, which holds the same lock.
                if let Some(session) = runtime.runtime().active_system_audio_session.as_ref() {
                    session.set_excluded_bundle_ids(next.clone());
                }
                last_system_audio_excluded_bundle_ids = next;
            }

            // Low-disk recovery runs on its own ~10s throttle, independent of the
            // privacy-filter update channel: free space recovers without any
            // privacy-filter event arriving, so this re-probes on a calm cadence
            // and resumes all sources (incl. the mic) once free space climbs back
            // above the resume threshold. Disk recovers far more slowly than a
            // display waking, hence the longer interval vs display recovery.
            let low_disk_suspended = runtime
                .runtime()
                .capture_suspension
                .as_ref()
                .is_some_and(|suspension| suspension.kind == CaptureSuspensionKind::LowDisk);
            if low_disk_suspended
                && last_low_disk_recovery_attempt.elapsed() >= LOW_DISK_RECOVERY_INTERVAL
            {
                last_low_disk_recovery_attempt = Instant::now();
                match attempt_privacy_suspension_recovery(&core, runtime.runtime_mut()) {
                    PrivacySuspensionRecoveryOutcome::Recovered => {
                        crate::debug_log::log(
                            "low-disk capture recovered; restarted all sources after suspension",
                        );
                    }
                    PrivacySuspensionRecoveryOutcome::RestartRequired
                    | PrivacySuspensionRecoveryOutcome::RetryPending
                    | PrivacySuspensionRecoveryOutcome::NotSuspended => {}
                }
            }

            if let Some(privacy_filter_update) = privacy_filter_update {
                if runtime.runtime().capture_suspension.is_some() {
                    // Throttle display-unavailable recovery so we probe for a
                    // returning display at a calm cadence instead of on every 1s
                    // poll. Privacy-filter recovery is left to retry promptly
                    // (it's capped at a few attempts).
                    let suspension_kind = runtime
                        .runtime()
                        .capture_suspension
                        .as_ref()
                        .map(|suspension| suspension.kind);
                    let throttle_display_recovery = suspension_kind
                        == Some(CaptureSuspensionKind::DisplayUnavailable)
                        && last_suspension_recovery_attempt.elapsed()
                            < DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL;
                    // Low-disk recovery is owned by the dedicated ~10s-throttled
                    // block above; never drive it from the privacy-update channel
                    // (which would bypass that throttle on every privacy event).
                    let is_low_disk = suspension_kind == Some(CaptureSuspensionKind::LowDisk);
                    if !throttle_display_recovery && !is_low_disk {
                        last_suspension_recovery_attempt = Instant::now();
                        match attempt_privacy_suspension_recovery(&core, runtime.runtime_mut()) {
                            PrivacySuspensionRecoveryOutcome::Recovered => {
                                crate::debug_log::log_info(
                                    "screen capture recovered; restarted after suspension",
                                );
                            }
                            PrivacySuspensionRecoveryOutcome::RestartRequired => {}
                            PrivacySuspensionRecoveryOutcome::RetryPending
                            | PrivacySuspensionRecoveryOutcome::NotSuspended => {}
                        }
                    }
                } else {
                    let apply_result = privacy::apply_privacy_filter_update(
                        &core,
                        runtime.runtime_mut(),
                        privacy_filter_update.update,
                    );
                    match apply_result {
                        Ok(outcome) => {
                            if privacy::privacy_refresh_debug_log_enabled(
                                privacy_filter_update.reason,
                            ) {
                                crate::debug_log::log(format!(
                                    "privacy refresh apply succeeded (reason={:?}, generation={}, mode={:?}, request_satisfied={})",
                                    privacy_filter_update.reason,
                                    privacy_filter_update.generation,
                                    privacy_filter_update.mode,
                                    outcome.request_satisfied
                                ));
                            }
                            privacy::record_privacy_filter_apply_outcome(
                                &core,
                                privacy_filter_update.mode,
                                outcome,
                            );
                        }
                        Err(error) => {
                            let display_unavailable =
                                error.code == privacy::PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE;
                            let suspension_kind = if display_unavailable {
                                CaptureSuspensionKind::DisplayUnavailable
                            } else {
                                CaptureSuspensionKind::PrivacyFilter
                            };
                            if display_unavailable {
                                crate::debug_log::log_warn(format!(
                                    "capture display unavailable; suspending screen capture until the display returns: [{}] {}",
                                    error.code, error.message
                                ));
                            } else {
                                crate::debug_log::log_warn(format!(
                                    "privacy filter update failed; suspending screen capture: [{}] {}",
                                    error.code, error.message
                                ));
                            }
                            if let Err(stop_error) = suspend_screen_capture(
                                Some(&core),
                                runtime.runtime_mut(),
                                &error,
                                suspension_kind,
                            ) {
                                crate::debug_log::log_warn(format!(
                        "capture suspension could not stop screen capture; preserving runtime state: [{}] {}",
                        stop_error.code, stop_error.message
                    ));
                                if !capture_screen::should_preserve_runtime_on_stop_error(
                                    &stop_error,
                                ) {
                                    mark_runtime_session_failed(runtime.runtime_mut());
                                    break;
                                }
                                continue;
                            }
                            // A transient display loss keeps the session alive so
                            // recovery can resume the screen when the display
                            // returns, even with no other source. A genuine
                            // privacy-filter failure can't make progress, so it
                            // ends the session — but only if nothing else is
                            // recording. Both audio families are independent of
                            // the screen, so either one keeps it alive.
                            let has_live_audio_source =
                                runtime.runtime().requested_sources.as_ref().is_some_and(
                                    |sources| sources.microphone || sources.system_audio,
                                );
                            if !display_unavailable && !has_live_audio_source {
                                mark_runtime_session_failed(runtime.runtime_mut());
                                break;
                            }
                        }
                    }
                }
            }

            match runtime.tick_inactivity(&core) {
                TickOutcome::Continue => {}
                TickOutcome::SkipRotation => continue,
                TickOutcome::StopLoop => break,
            }

            match runtime.tick_rotation(&core) {
                TickOutcome::Continue => {}
                TickOutcome::SkipRotation => continue,
                TickOutcome::StopLoop => break,
            }
        }

        // The loop only falls out on its own when the session ended internally
        // (a fatal capture failure, a privacy/display suspension that could not
        // continue, lost sources, etc.). User- and command-initiated stops set
        // the worker stop flag and broadcast the new session state themselves, so
        // skip those to avoid racing their teardown. For an internal end nobody
        // else announces it, so the frontend and the native status-bar tray would
        // otherwise keep showing a running session (e.g. "Stop Recording") that no
        // longer exists. Broadcast the real state so both surfaces resync.
        if !worker_control.stop.load(Ordering::Relaxed) {
            let session = core
                .state::<NativeCaptureState>()
                .lock()
                .ok()
                .map(|runtime| runtime.session());
            if let Some(session) = session {
                if !session.is_running {
                    super::super::emit_native_capture_session_changed(&core, &session);
                    core.host().refresh_tray();
                }
            }
        }
    });

    control
}
