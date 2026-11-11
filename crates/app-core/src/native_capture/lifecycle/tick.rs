//! The tick's inactivity half: the tap's rebuild engine and retry, the away
//! hold, and the per-source inactivity pause/resume reconcile.

use super::*;

impl RecordingLifecycle {
    /// Drives the system-audio tap's rebuild engine.
    ///
    /// Deliberately unconditional on pause state, and deliberately ahead of the
    /// screen reconcile below: the zero-watchdog is the only thing that notices a
    /// wedged tap, and a tap paused for inactivity is exactly where a wedge hides
    /// — the resume trigger is "sound detected", which a wedged tap can never
    /// deliver. That is how the ScreenCaptureKit bug trapped a live session for
    /// 34 minutes (ADR 0052).
    #[cfg(target_os = "macos")]
    fn tick_system_audio(&mut self, core: &Arc<AppCore>) {
        let Some(session) = self.runtime.active_system_audio_session.as_mut() else {
            self.retry_system_audio_start(core);
            return;
        };
        // A rebuild rotates onto a fresh file mid-segment; track it so the next
        // boundary commits it rather than the file the dead tap left behind.
        let rotated_output_file = session.poll();

        // Judged every tick rather than at stop: a denied grant should surface
        // while the user is still recording, not once they happen to stop
        // (ADR 0052). It settles after the first judgement and costs an atomic
        // load thereafter.
        let session_age_ms = self
            .runtime
            .recording_started_at_unix_ms
            .map(|started| crate::native_capture::runtime::now_unix_ms().saturating_sub(started))
            .unwrap_or(0);
        crate::native_capture::system_audio::note_permission_evidence(core, session_age_ms);

        let Some(system_audio_output_file) = rotated_output_file else {
            return;
        };

        self.track_live_system_audio_output_file(system_audio_output_file);
    }

    #[cfg(target_os = "macos")]
    fn track_live_system_audio_output_file(&mut self, output_file: String) {
        self.runtime.system_audio_recording_file = Some(output_file.clone());
        set_current_system_audio_output_file(
            current_segment_output_files_mut(&mut self.runtime),
            output_file,
        );
    }

    /// Retries a tap that never started.
    ///
    /// A start failure costs only system audio, so it is logged rather than
    /// failing the session — but with `requested_sources.system_audio` still true
    /// the session then reports itself healthy while recording no system audio at
    /// all, and the zero-watchdog cannot notice because it lives inside the
    /// session that was never created. The failures worth healing are transient
    /// (coreaudiod restarting, an aggregate-UID collision), so the tick retries on
    /// the watchdog's own shape (ADR 0052).
    ///
    /// `start_system_audio_family_for_runtime` refuses on its own unless system
    /// audio is requested, unpaused and tap-less; only low disk — the one
    /// suspension that stops the tap and restarts it itself (ADR 0040) — has to be
    /// excluded here.
    #[cfg(target_os = "macos")]
    fn retry_system_audio_start(&mut self, core: &Arc<AppCore>) {
        if !should_retry_system_audio_start(&self.runtime) {
            return;
        }

        let now = crate::native_capture::runtime::now_monotonic_marker_ms();
        if self
            .system_audio_start_retry_at_ms
            .is_some_and(|retry_at| now < retry_at)
        {
            return;
        }

        let segment_index = self.runtime.current_segment_index;
        match start_system_audio_family_for_runtime(
            &mut self.runtime,
            crate::native_capture::privacy::collect_initial_privacy_filter(core)
                .excluded_bundle_ids(),
            segment_index,
            "retrying system audio start",
        ) {
            Ok(output_file) => {
                self.system_audio_start_retry_at_ms = None;
                self.system_audio_start_retry_delay_ms = 0;
                if let Some(output_file) = output_file {
                    crate::debug_log::log_info(format!(
                        "{} tap started on retry after an earlier start failure",
                        capture_system_audio::LOG_PREFIX
                    ));
                    self.track_live_system_audio_output_file(output_file);
                }
            }
            Err(error) => {
                self.schedule_system_audio_start_retry(now);
                crate::debug_log::log_warn(format!(
                    "{} tap start retry failed; next attempt in {}ms: [{}] {}",
                    capture_system_audio::LOG_PREFIX,
                    self.system_audio_start_retry_delay_ms,
                    error.code,
                    error.message
                ));
            }
        }
    }

    /// Doubles the retry delay into the [`SYSTEM_AUDIO_START_RETRY_MIN_MS`] ..=
    /// [`SYSTEM_AUDIO_START_RETRY_MAX_MS`] band. A delay of zero is the
    /// never-backed-off state, so the first failure lands on the minimum.
    #[cfg(target_os = "macos")]
    pub(super) fn schedule_system_audio_start_retry(&mut self, now: u64) {
        self.system_audio_start_retry_delay_ms = self
            .system_audio_start_retry_delay_ms
            .saturating_mul(2)
            .clamp(
                SYSTEM_AUDIO_START_RETRY_MIN_MS,
                SYSTEM_AUDIO_START_RETRY_MAX_MS,
            );
        self.system_audio_start_retry_at_ms =
            Some(now.saturating_add(self.system_audio_start_retry_delay_ms));
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn tick_inactivity(&mut self, core: &Arc<AppCore>) -> TickOutcome {
        if self.runtime.user_capture_paused {
            return TickOutcome::SkipRotation;
        }

        self.tick_system_audio(core);
        tick_microphone_away(&mut self.runtime, Some(core));

        if let Some(error) = capture_screen::take_screen_capture_session_stop_error(
            self.runtime.active_screen_session.as_mut(),
        ) {
            if self.runtime.inactivity.is_screen_paused() {
                let _ = self.clear_screen_state_for_sleep_or_stop();
                return TickOutcome::SkipRotation;
            }
            crate::debug_log::log_warn(format!(
                "screen capture stream stopped unexpectedly; suspending screen until capture can restart: [{}] {}",
                error.code, error.message
            ));
            return self.suspend_screen_after_unexpected_stop(Some(core), &error);
        }

        let now = crate::native_capture::runtime::now_monotonic_marker_ms();
        let activity_snapshot = current_activity_snapshot(&mut self.runtime);
        let effective_idle = self
            .runtime
            .inactivity
            .effective_idle_for_snapshot(now, activity_snapshot);

        if should_resume_microphone_after_activity(&mut self.runtime, now, activity_snapshot) {
            if let Err(error) = resume_microphone_from_inactivity(&mut self.runtime) {
                crate::debug_log::log_at(
                    crate::debug_log::streak_level(&mut self.resume_failure_logged.microphone),
                    &format!(
                        "failed to resume microphone capture after activity: [{}] {}",
                        error.code, error.message
                    ),
                );
            } else {
                self.resume_failure_logged.microphone = false;
                let mic_eval = self
                    .runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                crate::debug_log::log(format!(
                    "resumed microphone capture after activity (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
                    mic_eval.effective_idle.idle_ms,
                    mic_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        } else {
            reopen_unwakeable_microphone(
                &mut self.runtime,
                &mut self.resume_failure_logged.microphone,
                || {
                    crate::native_capture::microphone::controller::current_capture_microphone_device_id(core)
                },
            );
        }

        // A lent-out microphone has no session to pause; the call would no-op
        // and log a pause every tick.
        if !self.runtime.microphone_lent_out
            && self
                .runtime
                .inactivity
                .should_pause_microphone_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) =
                pause_microphone_for_inactivity_with_app_handle(&mut self.runtime, Some(core))
            {
                crate::debug_log::log_warn(format!(
                    "failed to pause microphone capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = self
                    .runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                crate::debug_log::log(format!(
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
                crate::debug_log::log_at(
                    crate::debug_log::streak_level(&mut self.resume_failure_logged.system_audio),
                    &format!(
                        "failed to resume system audio capture after activity: [{}] {}",
                        error.code, error.message
                    ),
                );
            } else {
                self.resume_failure_logged.system_audio = false;
                let sa_eval = self
                    .runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                crate::debug_log::log(format!(
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
            if let Err(error) =
                pause_system_audio_for_inactivity_with_app_handle(&mut self.runtime, Some(core))
            {
                crate::debug_log::log_warn(format!(
                    "failed to pause system audio capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = self
                    .runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                crate::debug_log::log(format!(
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
            if let Err(error) = resume_screen_from_inactivity(&mut self.runtime, Some(core)) {
                let level = crate::debug_log::streak_level(&mut self.resume_failure_logged.screen);
                if handle_inactivity_resume_error(&mut self.runtime, error, level) {
                    return TickOutcome::StopLoop;
                }
            } else if !self.runtime.inactivity.is_screen_paused() {
                self.resume_failure_logged.screen = false;
                // Ok with the screen still paused means the resume was deferred
                // (no drawable display yet) — nothing to announce.
                let screen_eval = self
                    .runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                crate::debug_log::log(format!(
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
                pause_screen_for_inactivity_with_app_handle(&mut self.runtime, Some(core))
            {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    crate::native_capture::runtime::mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            } else {
                let screen_eval = self
                    .runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                crate::debug_log::log(format!(
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
                let level = crate::debug_log::streak_level(&mut self.resume_failure_logged.all);
                if handle_inactivity_resume_error(&mut self.runtime, error, level) {
                    return TickOutcome::StopLoop;
                }
            } else {
                self.resume_failure_logged.all = false;
                crate::debug_log::log_info(format!(
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
                pause_runtime_for_inactivity_with_app_handle(&mut self.runtime, Some(core))
            {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    crate::native_capture::runtime::mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            } else {
                crate::debug_log::log_info(format!(
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
}
