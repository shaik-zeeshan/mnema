use super::activity::current_activity_snapshot;
use super::output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
use super::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal,
    current_segment_sources_for_runtime, ensure_system_audio_planner_for_runtime,
    mark_runtime_session_failed, microphone_planner_for_runtime, refresh_runtime_planner_dates,
    screen_planner_for_runtime, system_audio_planner_for_runtime,
};
use super::runtime::{
    mark_runtime_session_stopped, request_segment_loop_stop, session_from_runtime,
    stopped_session_from_runtime, NativeCaptureRuntime,
};
use super::segments::{
    apply_microphone_output_finalization, cleanup_failed_segment_dirs, create_segment_output_dirs,
    empty_output_files, handle_inactivity_resume_error,
    pause_microphone_for_inactivity_with_app_handle, pause_runtime_for_inactivity_with_app_handle,
    pause_screen_for_inactivity_with_app_handle, pause_system_audio_for_inactivity_with_app_handle,
    plan_live_rotation_segment, reanchor_active_segment_timing,
    recover_from_segment_finalize_error, recover_screen_capture_after_wake,
    resume_microphone_from_inactivity, resume_runtime_from_inactivity,
    resume_screen_from_inactivity, resume_system_audio_from_inactivity, start_capture_runtime,
    start_segment_with_current_privacy_filter, stop_active_sessions_after_failure,
    stop_capture_runtime,
};
use capture_runtime::RuntimeSignal;
use capture_types::{
    CaptureErrorResponse, CaptureSources, NativeCaptureSession, RecordingSettings,
};
use capture_vad::MicrophoneVadFallbackNotice;

#[derive(Debug, Default)]
pub(crate) struct RecordingLifecycle {
    runtime: NativeCaptureRuntime,
}

#[derive(Debug, Clone)]
pub(crate) enum StartRecordingLifecycleOutcome {
    Started(NativeCaptureSession),
    AlreadyRunning(NativeCaptureSession),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TickOutcome {
    Continue,
    SkipRotation,
    StopLoop,
}

impl RecordingLifecycle {
    #[cfg(target_os = "macos")]
    fn clear_screen_state_for_sleep_or_stop(&mut self) -> bool {
        let Some(requested_sources) = self.runtime.requested_sources.clone() else {
            return false;
        };

        if !self.runtime.is_running || !requested_sources.screen {
            return false;
        }

        let has_screen_state = self.runtime.active_screen_session.is_some()
            || self.runtime.recording_file.is_some()
            || self.runtime.system_audio_recording_file.is_some();
        if !has_screen_state {
            return false;
        }

        self.runtime.active_screen_session = None;
        self.runtime.recording_file = None;
        self.runtime.system_audio_recording_file = None;
        self.runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
            &requested_sources,
            true,
            self.runtime.inactivity.microphone_paused,
            true,
        );

        if let Some(outputs) = self.runtime.current_segment_output_files.as_mut() {
            if outputs.screen_file.is_none() && outputs.screen_files.is_empty() {
                outputs.system_audio_file = None;
                outputs.system_audio_files.clear();
            }
        }

        true
    }

    pub(crate) fn session(&self) -> NativeCaptureSession {
        session_from_runtime(&self.runtime)
    }

    pub(crate) fn start(
        &mut self,
        app_handle: tauri::AppHandle,
        settings: &RecordingSettings,
        sources: CaptureSources,
        microphone_device_id_for_capture: Option<String>,
    ) -> Result<StartRecordingLifecycleOutcome, CaptureErrorResponse> {
        if self.runtime.is_running {
            if self.runtime.requested_sources.as_ref() != Some(&sources) {
                return Err(CaptureErrorResponse {
                    code: "capture_session_already_running".to_string(),
                    message: "A native capture session is already running with different sources"
                        .to_string(),
                });
            }

            return Ok(StartRecordingLifecycleOutcome::AlreadyRunning(
                self.session(),
            ));
        }

        start_capture_runtime(
            &mut self.runtime,
            app_handle,
            settings,
            sources,
            microphone_device_id_for_capture,
        )?;

        Ok(StartRecordingLifecycleOutcome::Started(self.session()))
    }

    pub(crate) fn stop(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Result<NativeCaptureSession, CaptureErrorResponse> {
        if let Err(error) = stop_capture_runtime(&mut self.runtime, Some(app_handle)) {
            if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                return Err(error);
            }

            request_segment_loop_stop(&self.runtime);
            mark_runtime_session_stopped(&mut self.runtime);
            return Err(error);
        }

        request_segment_loop_stop(&self.runtime);
        mark_runtime_session_stopped(&mut self.runtime);
        Ok(stopped_session_from_runtime(&self.runtime))
    }

    pub(crate) fn pause_user_capture(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Result<NativeCaptureSession, CaptureErrorResponse> {
        if !self.runtime.is_running {
            return Err(CaptureErrorResponse {
                code: "capture_session_not_running".to_string(),
                message: "No native capture session is running".to_string(),
            });
        }
        if self.runtime.user_capture_paused {
            return Ok(self.session());
        }
        stop_capture_runtime(&mut self.runtime, Some(app_handle))?;
        self.runtime.user_capture_paused = true;
        self.runtime.current_segment_sources = Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        });
        Ok(self.session())
    }

    pub(crate) fn resume_user_capture(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Result<NativeCaptureSession, CaptureErrorResponse> {
        if !self.runtime.is_running {
            return Err(CaptureErrorResponse {
                code: "capture_session_not_running".to_string(),
                message: "No native capture session is running".to_string(),
            });
        }
        if !self.runtime.user_capture_paused {
            return Ok(self.session());
        }
        #[cfg(target_os = "macos")]
        {
            let sources =
                self.runtime
                    .requested_sources
                    .clone()
                    .ok_or_else(|| CaptureErrorResponse {
                        code: "capture_resume_missing_sources".to_string(),
                        message:
                            "Cannot resume recording because the requested sources are missing"
                                .to_string(),
                    })?;
            let screen_planner = screen_planner_for_runtime(&self.runtime)
                .cloned()
                .ok_or_else(|| CaptureErrorResponse {
                    code: "capture_resume_missing_planner".to_string(),
                    message: "Cannot resume recording because the segment planner is missing"
                        .to_string(),
                })?;
            let microphone_planner = microphone_planner_for_runtime(&self.runtime).cloned();
            let system_audio_planner = if sources.system_audio {
                ensure_system_audio_planner_for_runtime(&mut self.runtime, "resuming user pause")?
            } else {
                system_audio_planner_for_runtime(&self.runtime).cloned()
            };
            let next_index = self.runtime.current_segment_index.saturating_add(1);
            let segment_dir = screen_planner.segment_dir(next_index);
            let screen_output_file = screen_planner.segment_screen_output(next_index);
            let microphone_output_path = sources
                .microphone
                .then(|| {
                    microphone_planner
                        .as_ref()
                        .map(|planner| planner.microphone_file(next_index))
                })
                .flatten();
            let system_audio_output_path = sources
                .system_audio
                .then(|| {
                    system_audio_planner
                        .as_ref()
                        .map(|planner| planner.system_audio_file(next_index))
                })
                .flatten();
            create_segment_output_dirs(
                &segment_dir,
                microphone_output_path
                    .as_deref()
                    .and_then(|path| path.parent()),
                system_audio_output_path
                    .as_deref()
                    .and_then(|path| path.parent()),
                &sources,
            )?;
            let started = start_segment_with_current_privacy_filter(
                app_handle,
                &segment_dir,
                sources.screen.then_some(screen_output_file.as_path()),
                system_audio_output_path.as_deref(),
                &sources,
                self.runtime.screen_frame_rate,
                &self.runtime.screen_resolution,
                self.runtime.effective_screen_bitrate_bps,
                self.runtime.microphone_device_id_for_capture.as_deref(),
                self.runtime.frame_artifact_tx.clone(),
                microphone_output_path.as_deref(),
            )?;
            self.runtime.current_segment_output_files = Some(started.0.clone());
            self.runtime
                .output_files
                .get_or_insert_with(empty_output_files);
            self.runtime.recording_file = started.1;
            self.runtime.microphone_recording_file = started.2;
            self.runtime.system_audio_recording_file = started.3;
            self.runtime.active_screen_session = started.4;
            self.runtime.active_microphone_session = started.5;
            self.runtime.current_segment_index = next_index;
            self.runtime.runtime_controller = Default::default();
            apply_runtime_signal(&mut self.runtime, RuntimeSignal::StartRequested)?;
            apply_runtime_signal(&mut self.runtime, RuntimeSignal::SourcesReady)?;
        }
        self.runtime.user_capture_paused = false;
        self.runtime.current_segment_sources = self.runtime.requested_sources.clone();
        if let Some(control) = self.runtime.segment_loop_control.as_ref() {
            control.notify();
        }
        Ok(self.session())
    }

    pub(crate) fn recover_after_wake(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
    ) -> Result<bool, CaptureErrorResponse> {
        recover_screen_capture_after_wake(&mut self.runtime, app_handle)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn should_attempt_recovery_after_possible_wake(&self) -> bool {
        if !self.runtime.is_running {
            return false;
        }

        if !self
            .runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.screen)
        {
            return false;
        }

        // Never fight a deliberate suspension: a DisplayUnavailable/LowDisk/privacy
        // suspension or a manual user pause owns the screen state and re-arms
        // through its own path. Recovering here would race that owner.
        if self.runtime.capture_suspension.is_some() || self.runtime.user_capture_paused {
            return false;
        }

        !self.runtime.inactivity.is_screen_paused()
            && self.runtime.recording_file.is_none()
            && !capture_screen::screen_capture_session_is_live(
                self.runtime.active_screen_session.as_ref(),
            )
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn handle_system_will_sleep(&mut self) -> bool {
        self.clear_screen_state_for_sleep_or_stop()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn tick_inactivity(&mut self, app_handle: &tauri::AppHandle) -> TickOutcome {
        if self.runtime.user_capture_paused {
            return TickOutcome::SkipRotation;
        }
        if let Some(error) = capture_screen::take_screen_capture_session_stop_error(
            self.runtime.active_screen_session.as_mut(),
        ) {
            if self.runtime.inactivity.is_screen_paused() {
                let _ = self.clear_screen_state_for_sleep_or_stop();
                return TickOutcome::SkipRotation;
            }
            super::debug_log::log(format!(
                "screen capture stream stopped unexpectedly; reconciling runtime state: [{}] {}",
                error.code, error.message
            ));
            let _ = self.clear_screen_state_for_sleep_or_stop();
            return TickOutcome::SkipRotation;
        }

        let now = super::runtime::now_monotonic_marker_ms();
        let activity_snapshot = current_activity_snapshot(&mut self.runtime);
        let effective_idle = self
            .runtime
            .inactivity
            .effective_idle_for_snapshot(now, activity_snapshot);

        if self
            .runtime
            .inactivity
            .should_resume_microphone_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_microphone_from_inactivity(&mut self.runtime) {
                super::debug_log::log(format!(
                    "failed to resume microphone capture after activity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = self
                    .runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "resumed microphone capture after activity (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
                    mic_eval.effective_idle.idle_ms,
                    mic_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_pause_microphone_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) =
                pause_microphone_for_inactivity_with_app_handle(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to pause microphone capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = self
                    .runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "paused microphone capture for inactivity threshold crossing (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
                    mic_eval.effective_idle.idle_ms,
                    mic_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_resume_system_audio_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_system_audio_from_inactivity(&mut self.runtime) {
                super::debug_log::log(format!(
                    "failed to resume system audio capture after activity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = self
                    .runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "resumed system audio capture after activity (system_audio_effective_idle_ms={}, system_audio_effective_source={}, idle_timeout_seconds={})",
                    sa_eval.effective_idle.idle_ms,
                    sa_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_pause_system_audio_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) = pause_system_audio_for_inactivity_with_app_handle(
                &mut self.runtime,
                Some(app_handle),
            ) {
                super::debug_log::log(format!(
                    "failed to pause system audio capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = self
                    .runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "paused system audio capture for inactivity threshold crossing (system_audio_effective_idle_ms={}, system_audio_effective_source={}, idle_timeout_seconds={})",
                    sa_eval.effective_idle.idle_ms,
                    sa_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        let privacy_suspended = self.runtime.capture_suspension.is_some();

        if !privacy_suspended
            && self
                .runtime
                .inactivity
                .should_resume_screen_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_screen_from_inactivity(&mut self.runtime, Some(app_handle)) {
                if handle_inactivity_resume_error(&mut self.runtime, error) {
                    return TickOutcome::StopLoop;
                }
            } else {
                let screen_eval = self
                    .runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "resumed screen capture after activity (screen_effective_idle_ms={}, screen_effective_source={}, idle_timeout_seconds={})",
                    screen_eval.effective_idle.idle_ms,
                    screen_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if !privacy_suspended
            && self
                .runtime
                .inactivity
                .should_pause_screen_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) =
                pause_screen_for_inactivity_with_app_handle(&mut self.runtime, Some(app_handle))
            {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    super::runtime::mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            } else {
                let screen_eval = self
                    .runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "paused screen capture for inactivity threshold crossing (screen_effective_idle_ms={}, screen_effective_source={}, idle_timeout_seconds={})",
                    screen_eval.effective_idle.idle_ms,
                    screen_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_resume_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_runtime_from_inactivity(&mut self.runtime) {
                if handle_inactivity_resume_error(&mut self.runtime, error) {
                    return TickOutcome::StopLoop;
                }
            } else {
                super::debug_log::log(format!(
                    "resumed native capture after activity (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                    effective_idle.idle_ms,
                    effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_pause_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) =
                pause_runtime_for_inactivity_with_app_handle(&mut self.runtime, Some(app_handle))
            {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    super::runtime::mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            } else {
                super::debug_log::log(format!(
                    "paused native capture for inactivity threshold crossing (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                    effective_idle.idle_ms,
                    effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }

            return TickOutcome::SkipRotation;
        }

        if self.runtime.inactivity.is_paused && self.runtime.current_segment_output_files.is_none()
        {
            return TickOutcome::SkipRotation;
        }

        TickOutcome::Continue
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn tick_rotation(&mut self, app_handle: &tauri::AppHandle) -> TickOutcome {
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
                    super::debug_log::log(format!(
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
        match super::segments::maybe_suspend_for_low_disk_at_boundary(
            app_handle,
            &mut self.runtime,
        ) {
            super::segments::LowDiskBoundaryOutcome::Proceed => {}
            super::segments::LowDiskBoundaryOutcome::Suspended => {
                return TickOutcome::SkipRotation;
            }
            super::segments::LowDiskBoundaryOutcome::Stopped => {
                return TickOutcome::StopLoop;
            }
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
            super::debug_log::log(format!(
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

        if active_sources.screen || active_sources.system_audio {
            let rotate_result = capture_screen::rotate_screen_capture_session(
                capture_screen::RotateScreenCaptureSessionArgs {
                    active_session: &mut self.runtime.active_screen_session,
                    segment_dir: &segment_dir,
                    screen_output_file: Some(&screen_output_file),
                    system_audio_output_path: system_audio_output_path.as_deref(),
                },
            );

            match rotate_result {
                Ok(rotated) => {
                    if let Some(file) = rotated.output_files.screen_file {
                        set_current_screen_output_file(&mut next_segment_outputs, file);
                    }
                    if let Some(file) = rotated.output_files.system_audio_file {
                        set_current_system_audio_output_file(&mut next_segment_outputs, file);
                    }
                    next_recording_file = Some(rotated.recording_file);
                    next_system_audio_recording_file = rotated.system_audio_recording_file;
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
                    match super::segments::handle_mid_segment_write_failure_for_low_disk(
                        app_handle,
                        &mut self.runtime,
                        previous_segment_output_files.as_ref(),
                        recording_file.as_deref(),
                        microphone_recording_file.as_deref(),
                        system_audio_recording_file.as_deref(),
                    ) {
                        super::segments::WriteFailureDiskFullOutcome::Suspended => {
                            cleanup_failed_segment_dirs(
                                &segment_dir,
                                microphone_audio_dir,
                                system_audio_dir,
                            );
                            return TickOutcome::SkipRotation;
                        }
                        super::segments::WriteFailureDiskFullOutcome::Stopped => {
                            cleanup_failed_segment_dirs(
                                &segment_dir,
                                microphone_audio_dir,
                                system_audio_dir,
                            );
                            return TickOutcome::StopLoop;
                        }
                        super::segments::WriteFailureDiskFullOutcome::NotDiskFull => {}
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
                    inactivity_tail_trim_seconds: 0,
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
                system_audio: active_sources.system_audio,
            };
            let legacy_system_audio_path = screen_only_sources
                .system_audio
                .then(|| system_audio_output_path.as_deref())
                .flatten();

            let started_segment = start_segment_with_current_privacy_filter(
                app_handle,
                &segment_dir,
                Some(&screen_output_file),
                legacy_system_audio_path,
                &screen_only_sources,
                self.runtime.screen_frame_rate,
                &self.runtime.screen_resolution,
                self.runtime.effective_screen_bitrate_bps,
                self.runtime.microphone_device_id_for_capture.as_deref(),
                self.runtime.frame_artifact_tx.clone(),
                None,
            );

            let (
                started_outputs,
                started_recording_file,
                started_microphone_recording_file,
                started_system_audio_recording_file,
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

            next_segment_outputs = started_outputs;
            next_recording_file = started_recording_file;
            next_microphone_recording_file = started_microphone_recording_file;
            next_system_audio_recording_file = started_system_audio_recording_file;
            self.runtime.active_screen_session = active_screen_session;

            if active_sources.microphone {
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
            super::segments::flush_frame_artifacts(tx);
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
                    super::segments::handle_mid_segment_write_failure_for_low_disk(
                        app_handle,
                        &mut self.runtime,
                        previous_segment_output_files.as_ref(),
                        recording_file.as_deref(),
                        microphone_recording_file.as_deref(),
                        system_audio_recording_file.as_deref(),
                    );
                if disk_full_outcome != super::segments::WriteFailureDiskFullOutcome::NotDiskFull {
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
                    super::debug_log::log(format!(
                        "mid-segment disk-full during segment finalization; discarded partials: [{}] {}",
                        error.code, error.message
                    ));
                    return match disk_full_outcome {
                        super::segments::WriteFailureDiskFullOutcome::Suspended => {
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
                super::debug_log::log(format!(
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
            super::segments::persist_committed_audio_segments(
                Some(app_handle),
                self.runtime.source_sessions.as_ref(),
                self.runtime.segment_schedule.as_ref(),
                self.runtime.current_segment_index,
                previous_segment_output_files.as_ref(),
            );
            super::segments::warm_scrub_previews_for_committed_screen_outputs(
                Some(app_handle),
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
            super::debug_log::log(format!(
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

    pub(crate) fn runtime(&self) -> &NativeCaptureRuntime {
        &self.runtime
    }

    pub(crate) fn runtime_mut(&mut self) -> &mut NativeCaptureRuntime {
        &mut self.runtime
    }

    pub(crate) fn take_microphone_vad_fallback_notification(
        &mut self,
    ) -> Option<MicrophoneVadFallbackNotice> {
        self.runtime.microphone_vad.take_new_fallback_notification()
    }
}
