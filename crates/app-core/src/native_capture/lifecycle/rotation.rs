//! The tick's rotation half: the low-disk and license boundary checks, then
//! finalizing the current segment and opening the next one.

use super::*;

impl RecordingLifecycle {
    #[cfg(target_os = "macos")]
    pub(crate) fn tick_rotation(&mut self, core: &Arc<AppCore>) -> TickOutcome {
        refresh_runtime_planner_dates(&mut self.runtime);

        let mut previous_segment_output_files = self.runtime.current_segment_output_files.clone();
        let recording_file = self.runtime.recording_file.clone();
        let microphone_recording_file = self.runtime.microphone_recording_file.clone();
        let system_audio_recording_file = self.runtime.system_audio_recording_file.clone();
        let requested_sources = self.runtime.requested_sources.clone();

        let Some(screen_planner) = screen_planner_for_runtime(&self.runtime).cloned() else {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        };
        let microphone_planner = microphone_planner_for_runtime(&self.runtime).cloned();
        let Some(sources) = self.runtime.requested_sources.clone() else {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        };
        let Some(schedule) = self.runtime.segment_schedule.clone() else {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        };
        let Some(clock) = self.runtime.capture_clock.clone() else {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        };
        let active_sources = if self.runtime.capture_suspension.is_some() {
            current_segment_sources_for_runtime(&self.runtime)
        } else {
            active_sources_for_inactivity_paused_state(
                &sources,
                self.runtime.inactivity.screen_paused,
                self.runtime.inactivity.microphone_paused,
                self.runtime.inactivity.system_audio_paused,
            )
        }
        .unwrap_or(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        });
        let system_audio_planner = if active_sources.system_audio {
            match ensure_system_audio_planner_for_runtime(&mut self.runtime, "rotating segments") {
                Ok(planner) => planner,
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to prepare system-audio planner while rotating segments: [{}] {}",
                        error.code, error.message
                    ));
                    mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            }
        } else {
            system_audio_planner_for_runtime(&self.runtime).cloned()
        };

        let Some(planned_rotation) = plan_live_rotation_segment(
            &self.runtime,
            &active_sources,
            &screen_planner,
            microphone_planner.as_ref(),
            system_audio_planner.as_ref(),
            &schedule,
            &clock,
        ) else {
            return TickOutcome::SkipRotation;
        };

        // Low-disk boundary check (ADR 0040): a rotation is due, so a new segment
        // file is about to be opened. If the recordings volume is too low on free
        // space, do not open the next segment — either enter a Low-Disk Suspension
        // across all sources (incl. mic) and let the recovery driver resume once
        // free space returns, or (below the critical reserve floor) stop the
        // session gracefully. Best-effort: an unmeasurable reading proceeds. The
        // graceful stop already marked the runtime failed and surfaced the error.
        match crate::native_capture::segments::maybe_suspend_for_low_disk_at_boundary(
            core,
            &mut self.runtime,
        ) {
            crate::native_capture::segments::LowDiskBoundaryOutcome::Proceed => {}
            crate::native_capture::segments::LowDiskBoundaryOutcome::Suspended => {
                return TickOutcome::SkipRotation;
            }
            crate::native_capture::segments::LowDiskBoundaryOutcome::Stopped => {
                return TickOutcome::StopLoop;
            }
        }

        // Trial-lapse boundary check: same shape as low-disk — the current
        // segment is healthy, only opening the *next* one is refused. Without
        // this, a session already recording when the trial expires would keep
        // recording until the user stops it. Commit the segment, end the
        // session, and recompute async so tray/Settings flip to Read-Only.
        let rotation_now_ms = crate::licensing::now_ms();
        let block = crate::licensing::license_block(
            crate::licensing::cached_status(core).as_ref(),
            rotation_now_ms,
        );
        if block.is_some() {
            crate::licensing::recompute_status_async(core, rotation_now_ms);
        }
        if let Some(outcome) = self.stop_for_license_block_at_rotation(Some(core), block) {
            return outcome;
        }

        if let Some(outcome) =
            self.suspend_if_screen_session_missing_at_rotation(Some(core), &active_sources)
        {
            return outcome;
        }

        if apply_runtime_signal(&mut self.runtime, RuntimeSignal::RotateRequested).is_err() {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }

        let next_index = planned_rotation.next_index;
        let segment_dir = planned_rotation.segment_dir;
        let screen_output_file = planned_rotation.screen_output_file;
        let system_audio_output_path = planned_rotation.system_audio_output_path;
        let microphone_output_path = planned_rotation.microphone_output_path;
        let microphone_audio_dir = microphone_output_path.as_deref().and_then(|p| p.parent());
        let system_audio_dir = system_audio_output_path.as_deref().and_then(|p| p.parent());
        if let Err(error) = create_segment_output_dirs(
            &segment_dir,
            microphone_audio_dir,
            system_audio_dir,
            &active_sources,
        ) {
            crate::debug_log::log_error(format!(
                "failed to prepare capture segment output directories while rotating segments: [{}] {}",
                error.code, error.message
            ));
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }

        let mut next_segment_outputs = empty_output_files();
        let mut next_recording_file = self.runtime.recording_file.clone();
        let mut next_microphone_recording_file = self.runtime.microphone_recording_file.clone();
        let mut next_system_audio_recording_file = self.runtime.system_audio_recording_file.clone();
        let mut legacy_rotated = false;

        // System audio rotates on its own seam, whether or not the screen is
        // rotating — and whether or not there is a screen at all.
        if active_sources.system_audio {
            if let (Some(planner), Some(session)) = (
                system_audio_planner.as_ref(),
                self.runtime.active_system_audio_session.as_mut(),
            ) {
                next_system_audio_recording_file =
                    session.advance_segment(planner, next_index, system_audio_output_path.clone());
                if let Some(file) = next_system_audio_recording_file.clone() {
                    set_current_system_audio_output_file(&mut next_segment_outputs, file);
                }
            }
        }

        if active_sources.screen {
            let rotate_result = capture_screen::rotate_screen_capture_session(
                capture_screen::RotateScreenCaptureSessionArgs {
                    active_session: &mut self.runtime.active_screen_session,
                    segment_dir: &segment_dir,
                    screen_output_file: Some(&screen_output_file),
                },
            );

            match rotate_result {
                Ok(rotated) => {
                    if let Some(file) = rotated.output_files.screen_file {
                        set_current_screen_output_file(&mut next_segment_outputs, file);
                    }
                    next_recording_file = Some(rotated.recording_file);
                }
                Err(error) if error.code == "capture_rotation_requires_restart" => {
                    legacy_rotated = true;
                }
                Err(_) => {
                    // Mid-segment disk-full backstop (ADR 0040): a rotate failure
                    // re-probes free space; if it coincides with low disk, discard
                    // the partial (no corrupt segment) and suspend or stop instead
                    // of the generic fatal failure. Healthy/unmeasurable disk keeps
                    // the existing behavior.
                    match crate::native_capture::segments::handle_mid_segment_write_failure_for_low_disk(
                        core,
                        &mut self.runtime,
                        previous_segment_output_files.as_ref(),
                        recording_file.as_deref(),
                        microphone_recording_file.as_deref(),
                        system_audio_recording_file.as_deref(),
                    ) {
                        crate::native_capture::segments::WriteFailureDiskFullOutcome::Suspended => {
                            cleanup_failed_segment_dirs(
                                &segment_dir,
                                microphone_audio_dir,
                                system_audio_dir,
                            );
                            return TickOutcome::SkipRotation;
                        }
                        crate::native_capture::segments::WriteFailureDiskFullOutcome::Stopped => {
                            cleanup_failed_segment_dirs(
                                &segment_dir,
                                microphone_audio_dir,
                                system_audio_dir,
                            );
                            return TickOutcome::StopLoop;
                        }
                        crate::native_capture::segments::WriteFailureDiskFullOutcome::NotDiskFull => {}
                    }
                    cleanup_failed_segment_dirs(
                        &segment_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                    stop_active_sessions_after_failure(&mut self.runtime);
                    mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            }
        }

        if legacy_rotated {
            if capture_screen::stop_screen_capture_session(
                capture_screen::StopScreenCaptureSessionArgs {
                    active_session: &mut self.runtime.active_screen_session,
                },
            )
            .is_err()
            {
                cleanup_failed_segment_dirs(&segment_dir, microphone_audio_dir, system_audio_dir);
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            }

            let screen_only_sources = CaptureSources {
                screen: active_sources.screen,
                microphone: false,
                system_audio: false,
            };

            let started_segment = start_segment_with_current_privacy_filter(
                core,
                &segment_dir,
                Some(&screen_output_file),
                &screen_only_sources,
                self.runtime.screen_frame_rate,
                &self.runtime.screen_resolution,
                self.runtime.effective_screen_bitrate_bps,
                self.runtime.microphone_device_id_for_capture.as_deref(),
                self.runtime.frame_artifact_tx.clone(),
                None,
            );

            let (
                mut started_outputs,
                started_recording_file,
                started_microphone_recording_file,
                active_screen_session,
                _,
            ) = match started_segment {
                Ok(value) => value,
                Err(_) => {
                    cleanup_failed_segment_dirs(
                        &segment_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                    stop_active_sessions_after_failure(&mut self.runtime);
                    mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            };

            // The screen restart mints fresh outputs; carry over the file the tap
            // rotated onto above, which the restart knows nothing about.
            if let Some(file) = next_system_audio_recording_file.clone() {
                set_current_system_audio_output_file(&mut started_outputs, file);
            }
            next_segment_outputs = started_outputs;
            next_recording_file = started_recording_file;
            next_microphone_recording_file = started_microphone_recording_file;
            self.runtime.active_screen_session = active_screen_session;

            if active_sources.microphone {
                crate::native_capture::activity::process_pending_microphone_vad_frames(
                    &mut self.runtime,
                );
                if let Some(session) = self.runtime.active_microphone_session.as_mut() {
                    let microphone_output_file = microphone_planner
                        .as_ref()
                        .expect("microphone planner should exist when microphone source is enabled")
                        .microphone_file(next_index)
                        .to_string_lossy()
                        .to_string();
                    let mic_finalization = match session
                        .rotate_output_file_returning_finalization(&microphone_output_file)
                    {
                        Ok(finalization) => finalization,
                        Err(_) => {
                            cleanup_failed_segment_dirs(
                                &segment_dir,
                                microphone_audio_dir,
                                system_audio_dir,
                            );
                            cleanup_unusable_segment_artifacts(
                                Some(&next_segment_outputs),
                                next_recording_file.as_deref(),
                                next_microphone_recording_file.as_deref(),
                                next_system_audio_recording_file.as_deref(),
                            );
                            stop_active_sessions_after_failure(&mut self.runtime);
                            mark_runtime_session_failed(&mut self.runtime);
                            return TickOutcome::StopLoop;
                        }
                    };
                    apply_microphone_output_finalization(
                        previous_segment_output_files.as_mut(),
                        &mic_finalization,
                        self.runtime.source_sessions.as_ref(),
                        self.runtime.segment_schedule.as_ref(),
                        self.runtime.current_segment_index,
                    );
                    set_current_microphone_output_file(
                        &mut next_segment_outputs,
                        microphone_output_file.clone(),
                    );
                    next_microphone_recording_file = Some(microphone_output_file);
                }
            }
        } else if active_sources.microphone {
            crate::native_capture::activity::process_pending_microphone_vad_frames(
                &mut self.runtime,
            );
            if let Some(session) = self.runtime.active_microphone_session.as_mut() {
                let microphone_output_file = microphone_planner
                    .as_ref()
                    .expect("microphone planner should exist when microphone source is enabled")
                    .microphone_file(next_index)
                    .to_string_lossy()
                    .to_string();
                let mic_finalization = match session
                    .rotate_output_file_returning_finalization(&microphone_output_file)
                {
                    Ok(finalization) => finalization,
                    Err(_) => {
                        cleanup_failed_segment_dirs(
                            &segment_dir,
                            microphone_audio_dir,
                            system_audio_dir,
                        );
                        cleanup_unusable_segment_artifacts(
                            Some(&next_segment_outputs),
                            next_recording_file.as_deref(),
                            next_microphone_recording_file.as_deref(),
                            next_system_audio_recording_file.as_deref(),
                        );
                        stop_active_sessions_after_failure(&mut self.runtime);
                        mark_runtime_session_failed(&mut self.runtime);
                        return TickOutcome::StopLoop;
                    }
                };
                apply_microphone_output_finalization(
                    previous_segment_output_files.as_mut(),
                    &mic_finalization,
                    self.runtime.source_sessions.as_ref(),
                    self.runtime.segment_schedule.as_ref(),
                    self.runtime.current_segment_index,
                );
                set_current_microphone_output_file(
                    &mut next_segment_outputs,
                    microphone_output_file.clone(),
                );
                next_microphone_recording_file = Some(microphone_output_file);
            }
        }

        if let Some(tx) = self.runtime.frame_artifact_tx.as_ref() {
            crate::native_capture::frame_artifacts::flush_frame_artifacts(tx);
        }

        let previous_segment_committed = match finalize_capture_outputs(
            previous_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        ) {
            Ok(()) => true,
            Err(error)
                if recover_from_segment_finalize_error(
                    "rotating segments",
                    &error,
                    previous_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                ) =>
            {
                false
            }
            Err(error) => {
                // Mid-segment disk-full backstop (ADR 0040): a non-recoverable
                // finalize failure re-probes free space; if it coincides with low
                // disk, discard the partial (no corrupt segment is committed) and
                // suspend or stop rather than failing the session generically.
                // Healthy/unmeasurable disk keeps the existing fatal behavior.
                let disk_full_outcome =
                    crate::native_capture::segments::handle_mid_segment_write_failure_for_low_disk(
                        core,
                        &mut self.runtime,
                        previous_segment_output_files.as_ref(),
                        recording_file.as_deref(),
                        microphone_recording_file.as_deref(),
                        system_audio_recording_file.as_deref(),
                    );
                if disk_full_outcome
                    != crate::native_capture::segments::WriteFailureDiskFullOutcome::NotDiskFull
                {
                    // Disk-full: also discard the just-opened next segment's
                    // partial so nothing corrupt survives, then suspend or stop.
                    cleanup_failed_segment_dirs(
                        &segment_dir,
                        microphone_audio_dir,
                        system_audio_dir,
                    );
                    cleanup_unusable_segment_artifacts(
                        Some(&next_segment_outputs),
                        next_recording_file.as_deref(),
                        next_microphone_recording_file.as_deref(),
                        next_system_audio_recording_file.as_deref(),
                    );
                    crate::debug_log::log_warn(format!(
                        "mid-segment disk-full during segment finalization; discarded partials: [{}] {}",
                        error.code, error.message
                    ));
                    return match disk_full_outcome {
                        crate::native_capture::segments::WriteFailureDiskFullOutcome::Suspended => {
                            TickOutcome::SkipRotation
                        }
                        // Stopped (or the unreachable NotDiskFull) stops the loop.
                        _ => TickOutcome::StopLoop,
                    };
                }
                cleanup_failed_segment_dirs(&segment_dir, microphone_audio_dir, system_audio_dir);
                cleanup_unusable_segment_artifacts(
                    previous_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                );
                cleanup_unusable_segment_artifacts(
                    Some(&next_segment_outputs),
                    next_recording_file.as_deref(),
                    next_microphone_recording_file.as_deref(),
                    next_system_audio_recording_file.as_deref(),
                );
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                crate::debug_log::log_error(format!(
                    "fatal native capture segment finalization failure while rotating segments: [{}] {}",
                    error.code, error.message
                ));
                return TickOutcome::StopLoop;
            }
        };

        if previous_segment_committed {
            if let (Some(committed), Some(segment)) = (
                self.runtime.output_files.as_mut(),
                previous_segment_output_files.as_ref(),
            ) {
                append_committed_segment_output_files(committed, segment);
            }
            crate::native_capture::segments::persist_committed_audio_segments(
                Some(core),
                self.runtime.source_sessions.as_ref(),
                self.runtime.segment_schedule.as_ref(),
                self.runtime.current_segment_index,
                previous_segment_output_files.as_ref(),
            );
            crate::native_capture::segments::warm_scrub_previews_for_committed_screen_outputs(
                Some(core),
                previous_segment_output_files.as_ref(),
            );
        }

        self.runtime.current_segment_index = next_index;
        self.runtime.current_segment_output_files = Some(next_segment_outputs);
        self.runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
            &active_sources,
            self.runtime.inactivity.screen_paused,
            self.runtime.inactivity.microphone_paused,
            self.runtime.inactivity.system_audio_paused,
        );
        self.runtime.recording_file = next_recording_file;
        self.runtime.microphone_recording_file = next_microphone_recording_file;
        self.runtime.system_audio_recording_file = next_system_audio_recording_file;
        if let Err(error) = reanchor_active_segment_timing(&mut self.runtime, "rotating segments") {
            crate::debug_log::log_error(format!(
                "failed to re-anchor native capture segment timing while rotating segments: [{}] {}",
                error.code, error.message
            ));
            stop_active_sessions_after_failure(&mut self.runtime);
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }

        if apply_runtime_signal(&mut self.runtime, RuntimeSignal::SourcesReady).is_err() {
            stop_active_sessions_after_failure(&mut self.runtime);
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }

        TickOutcome::Continue
    }
}
