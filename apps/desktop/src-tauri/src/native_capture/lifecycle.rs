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
    screen_planner_for_runtime, system_audio_planner_for_runtime, system_audio_stops_with_suspension,
    CaptureSuspensionKind,
};
use super::runtime::{
    mark_runtime_session_stopped, request_segment_loop_stop, session_from_runtime,
    stopped_session_from_runtime, NativeCaptureRuntime,
};
use super::segments::{
    apply_microphone_output_finalization, cleanup_failed_segment_dirs, create_segment_output_dirs,
    current_segment_output_files_mut, empty_output_files, handle_inactivity_resume_error,
    pause_microphone_for_inactivity_with_app_handle, pause_runtime_for_inactivity_with_app_handle,
    pause_screen_for_inactivity_with_app_handle, pause_system_audio_for_inactivity_with_app_handle,
    plan_live_rotation_segment, reanchor_active_segment_timing,
    flush_frame_artifacts, recover_from_segment_finalize_error, recover_screen_capture_after_wake,
    resume_microphone_from_inactivity, resume_runtime_from_inactivity,
    resume_screen_from_inactivity, resume_system_audio_from_inactivity, start_capture_runtime,
    start_segment_with_current_privacy_filter, start_system_audio_family_for_runtime,
    stop_active_sessions_after_failure, stop_capture_runtime,
};
use capture_runtime::RuntimeSignal;
use capture_types::{
    CaptureErrorResponse, CaptureSources, NativeCaptureSession, RecordingSettings,
};
use capture_vad::MicrophoneVadFallbackNotice;

/// The tap-start retry band, mirroring the zero-watchdog's backoff (ADR 0052):
/// often enough that a coreaudiod restart heals within a segment, rarely enough
/// that a permanently unavailable tap is not probed on every tick.
#[cfg(target_os = "macos")]
const SYSTEM_AUDIO_START_RETRY_MIN_MS: u64 = 30_000;
#[cfg(target_os = "macos")]
const SYSTEM_AUDIO_START_RETRY_MAX_MS: u64 = 600_000;

/// Whether the tick should try to bring a missing tap up.
///
/// Low disk is the carve-out: it is the one suspension that deliberately stops
/// the tap, and it restarts it itself once space returns (ADR 0040). Every other
/// suspension leaves the tap alone, so a tap-less runtime under one is a tap that
/// failed to start — exactly the case worth retrying.
#[cfg(target_os = "macos")]
pub(super) fn should_retry_system_audio_start(runtime: &NativeCaptureRuntime) -> bool {
    runtime.is_running
        && runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.system_audio)
        && !runtime.inactivity.is_system_audio_paused()
        && !system_audio_stops_with_suspension(runtime)
        && runtime.active_system_audio_session.is_none()
}

#[derive(Debug, Default)]
pub(crate) struct RecordingLifecycle {
    runtime: NativeCaptureRuntime,
    /// Monotonic deadline for the next tap-start retry, and the delay that set
    /// it. Kept here rather than on the runtime because it is the tick's pacing,
    /// not capture state; [`RecordingLifecycle::start`] resets it per session.
    system_audio_start_retry_at_ms: Option<u64>,
    system_audio_start_retry_delay_ms: u64,
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

        let has_screen_state =
            self.runtime.active_screen_session.is_some() || self.runtime.recording_file.is_some();
        if !has_screen_state {
            return false;
        }

        // Screen state only. The tap has no display dependency, so a sleep leaves
        // it recording — and clearing its file here would orphan the segment it is
        // still writing (ADR 0052).
        self.runtime.active_screen_session = None;
        self.runtime.recording_file = None;
        self.runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
            &requested_sources,
            true,
            self.runtime.inactivity.microphone_paused,
            self.runtime.inactivity.system_audio_paused,
        );

        true
    }

    // An unexpected stream death (display sleep/disconnect, a system-initiated
    // kill) must land in a DisplayUnavailable suspension, not a bare state
    // clear: the suspension makes the rotation boundary skip and hands the
    // restart to `attempt_privacy_suspension_recovery`, which waits for a
    // display and retries with a bounded budget (ADR 0021). A bare clear left
    // the runtime with screen requested but no session and no owner — the next
    // segment rotation then rotated into the missing session and failed the
    // entire recording session fatally.
    #[cfg(target_os = "macos")]
    fn suspend_screen_after_unexpected_stop(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
        error: &CaptureErrorResponse,
    ) -> TickOutcome {
        if let Err(stop_error) = super::segments::suspend_screen_capture(
            app_handle,
            &mut self.runtime,
            error,
            CaptureSuspensionKind::DisplayUnavailable,
        ) {
            // Suspend only errors when the stop must preserve runtime state
            // (an in-flight stop timeout); keep the session and let a later
            // tick or the wake/display recovery paths reconcile it.
            super::debug_log::log(format!(
                "capture suspension could not stop screen capture; preserving runtime state: [{}] {}",
                stop_error.code, stop_error.message
            ));
        }
        TickOutcome::SkipRotation
    }

    // The start-seam half of the license gate: a block becomes an outright
    // refusal (Err) BEFORE any runtime mutation. Deliberately NOT a
    // `CaptureSuspension`: a license block never self-heals (ADR 0021/0040) —
    // it clears only when the user buys/activates — so it must never touch
    // `capture_suspension` nor share its codes/copy. `None` block (allowed or
    // gate-not-yet-run) → no refusal; never lock on unknown.
    fn refuse_start_for_license_block(
        block: Option<crate::licensing::LicenseBlock>,
    ) -> Option<CaptureErrorResponse> {
        let block = block?;
        Some(CaptureErrorResponse {
            code: block.code.to_string(),
            message: block.message.to_string(),
        })
    }

    // The rotation-seam half of the license gate: the current segment is
    // healthy, only opening the *next* one is refused — commit the segment,
    // end the session (StopLoop, never a suspension), and surface the honest
    // notification (revoked vs trial-ended).
    fn stop_for_license_block_at_rotation(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
        block: Option<crate::licensing::LicenseBlock>,
    ) -> Option<TickOutcome> {
        let block = block?;
        super::segments::graceful_stop_for_license_block(
            app_handle,
            &mut self.runtime,
            block.revoked,
        );
        Some(TickOutcome::StopLoop)
    }

    // Backstop at the rotation boundary for any path that still reaches it with
    // screen active but no live session (e.g. a system wake racing the will-sleep
    // teardown before the did-wake recovery re-arms). Rotating would call into the
    // missing session, hit `invalid_runtime_state` and end the whole session;
    // suspend instead so recovery restarts capture when it can.
    //
    // Screen-only by design: a system-audio-only session has no screen session to
    // miss, and suspending one for that would kill a healthy recording.
    #[cfg(target_os = "macos")]
    fn suspend_if_screen_session_missing_at_rotation(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
        active_sources: &CaptureSources,
    ) -> Option<TickOutcome> {
        if !active_sources.screen || self.runtime.active_screen_session.is_some() {
            return None;
        }

        super::debug_log::log(
            "segment rotation due but the screen capture session is missing; suspending screen until capture can restart",
        );
        Some(self.suspend_screen_after_unexpected_stop(
            app_handle,
            &CaptureErrorResponse {
                code: "capture_screen_session_missing".to_string(),
                message: "Screen capture session missing at segment rotation".to_string(),
            },
        ))
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
        // Read-Only Mode gate (licensing). This is the single seam every start
        // path funnels through (command, auto-start, tray). It is deliberately
        // NOT a `CaptureSuspension`: Read-Only Mode does not self-heal and is
        // never a transient-liveness condition (ADR 0021/0040) — it clears only
        // when the user buys a license, so it never touches `capture_suspension`
        // nor shares its codes/copy. `cached_status` is `None` until the deferred
        // gate runs once; treat unknown as allow, never lock on unknown.
        // `capture_allowed_at` also refuses a cached `Trial` whose window has
        // since lapsed (the cache only recomputes on gate events, so without
        // the time check the first start after expiry slips through).
        let gate_now_ms = crate::licensing::now_ms();
        let block = crate::licensing::license_block(
            crate::licensing::cached_status(&app_handle).as_ref(),
            gate_now_ms,
        );
        if let Some(refusal) = Self::refuse_start_for_license_block(block) {
            // Recompute async so the cache/tray/Settings flip from the stale
            // `Trial` to `ReadOnly`; the refusal itself doesn't wait for it.
            crate::licensing::recompute_status_async(&app_handle, gate_now_ms);
            super::debug_log::log(format!("capture refused: {}", refusal.code));
            return Err(refusal);
        }

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

        self.system_audio_start_retry_at_ms = None;
        self.system_audio_start_retry_delay_ms = 0;

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
            create_segment_output_dirs(
                &segment_dir,
                microphone_output_path
                    .as_deref()
                    .and_then(|path| path.parent()),
                None,
                &sources,
            )?;
            let started = start_segment_with_current_privacy_filter(
                app_handle,
                &segment_dir,
                sources.screen.then_some(screen_output_file.as_path()),
                &sources,
                self.runtime.screen_frame_rate,
                &self.runtime.screen_resolution,
                self.runtime.effective_screen_bitrate_bps,
                self.runtime.microphone_device_id_for_capture.as_deref(),
                self.runtime.frame_artifact_tx.clone(),
                microphone_output_path.as_deref(),
            )?;
            let mut segment_outputs = started.0;
            self.runtime
                .output_files
                .get_or_insert_with(empty_output_files);
            self.runtime.recording_file = started.1;
            self.runtime.microphone_recording_file = started.2;
            self.runtime.active_screen_session = started.3;
            self.runtime.active_microphone_session = started.4;
            self.runtime.current_segment_index = next_index;

            // A user pause stopped the tap with everything else, so a user resume
            // starts it again — including for a session with no screen at all.
            match super::segments::start_system_audio_family_for_runtime(
                &mut self.runtime,
                super::privacy::collect_initial_privacy_filter(app_handle).excluded_bundle_ids(),
                next_index,
                "resuming user pause",
            ) {
                Ok(system_audio_recording_file) => {
                    if let Some(system_audio_output_file) = system_audio_recording_file.as_ref() {
                        set_current_system_audio_output_file(
                            &mut segment_outputs,
                            system_audio_output_file.clone(),
                        );
                    }
                    self.runtime.system_audio_recording_file = system_audio_recording_file;
                }
                Err(error) => {
                    super::debug_log::log(format!(
                        "failed to restart system audio while resuming user pause; continuing without it: [{}] {}",
                        error.code, error.message
                    ));
                }
            }
            self.runtime.current_segment_output_files = Some(segment_outputs);
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
        // `NSWorkspaceWillSleep` fires *before* the machine sleeps, so the
        // ScreenCaptureKit stream is still live. Stop the writer now so the tail
        // segment's `.mov` is finalized with its closing `moov` atom — the
        // `ActiveScreenCaptureSession` has no `Drop` that does this, so simply
        // nulling it (as the teardown below does) leaves a truncated, unopenable
        // file. The screen path stays in `current_segment_output_files` so wake
        // recovery finalizes the now-complete tail; without the explicit stop that
        // finalize fails and the whole last segment — plus its buffered OCR frames
        // — is dropped. Flush those frame artifacts too, since the worker's input
        // channel goes quiet once the session is torn down.
        if self.runtime.is_running
            && self
                .runtime
                .requested_sources
                .as_ref()
                .is_some_and(|sources| sources.screen)
            && self.runtime.active_screen_session.is_some()
        {
            if let Err(error) = capture_screen::stop_screen_capture_session(
                capture_screen::StopScreenCaptureSessionArgs {
                    active_session: &mut self.runtime.active_screen_session,
                },
            ) {
                super::debug_log::log(format!(
                    "failed stopping live screen session for system sleep; tail segment may be unfinalizable: [{}] {}",
                    error.code, error.message
                ));
            }
            if let Some(tx) = self.runtime.frame_artifact_tx.as_ref() {
                flush_frame_artifacts(tx);
            }
        }
        self.clear_screen_state_for_sleep_or_stop()
    }

    /// Drives the system-audio tap's rebuild engine.
    ///
    /// Deliberately unconditional on pause state, and deliberately ahead of the
    /// screen reconcile below: the zero-watchdog is the only thing that notices a
    /// wedged tap, and a tap paused for inactivity is exactly where a wedge hides
    /// — the resume trigger is "sound detected", which a wedged tap can never
    /// deliver. That is how the ScreenCaptureKit bug trapped a live session for
    /// 34 minutes (ADR 0052).
    #[cfg(target_os = "macos")]
    fn tick_system_audio(&mut self, app_handle: &tauri::AppHandle) {
        let Some(session) = self.runtime.active_system_audio_session.as_mut() else {
            self.retry_system_audio_start(app_handle);
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
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.system_audio.as_ref())
            .map(|session| super::runtime::now_unix_ms().saturating_sub(session.started_at_unix_ms))
            .unwrap_or(0);
        super::system_audio::note_permission_evidence(app_handle, session_age_ms);

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
    fn retry_system_audio_start(&mut self, app_handle: &tauri::AppHandle) {
        if !should_retry_system_audio_start(&self.runtime) {
            return;
        }

        let now = super::runtime::now_monotonic_marker_ms();
        if self
            .system_audio_start_retry_at_ms
            .is_some_and(|retry_at| now < retry_at)
        {
            return;
        }

        let segment_index = self.runtime.current_segment_index;
        match start_system_audio_family_for_runtime(
            &mut self.runtime,
            super::privacy::collect_initial_privacy_filter(app_handle).excluded_bundle_ids(),
            segment_index,
            "retrying system audio start",
        ) {
            Ok(output_file) => {
                self.system_audio_start_retry_at_ms = None;
                self.system_audio_start_retry_delay_ms = 0;
                if let Some(output_file) = output_file {
                    super::debug_log::log(format!(
                        "{} tap started on retry after an earlier start failure",
                        capture_system_audio::LOG_PREFIX
                    ));
                    self.track_live_system_audio_output_file(output_file);
                }
            }
            Err(error) => {
                self.schedule_system_audio_start_retry(now);
                super::debug_log::log(format!(
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
    fn schedule_system_audio_start_retry(&mut self, now: u64) {
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
    pub(crate) fn tick_inactivity(&mut self, app_handle: &tauri::AppHandle) -> TickOutcome {
        if self.runtime.user_capture_paused {
            return TickOutcome::SkipRotation;
        }

        self.tick_system_audio(app_handle);

        if let Some(error) = capture_screen::take_screen_capture_session_stop_error(
            self.runtime.active_screen_session.as_mut(),
        ) {
            if self.runtime.inactivity.is_screen_paused() {
                let _ = self.clear_screen_state_for_sleep_or_stop();
                return TickOutcome::SkipRotation;
            }
            super::debug_log::log(format!(
                "screen capture stream stopped unexpectedly; suspending screen until capture can restart: [{}] {}",
                error.code, error.message
            ));
            return self.suspend_screen_after_unexpected_stop(Some(app_handle), &error);
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
            } else if !self.runtime.inactivity.is_screen_paused() {
                // Ok with the screen still paused means the resume was deferred
                // (no drawable display yet) — nothing to announce.
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

        // Trial-lapse boundary check: same shape as low-disk — the current
        // segment is healthy, only opening the *next* one is refused. Without
        // this, a session already recording when the trial expires would keep
        // recording until the user stops it. Commit the segment, end the
        // session, and recompute async so tray/Settings flip to Read-Only.
        let rotation_now_ms = crate::licensing::now_ms();
        let block = crate::licensing::license_block(
            crate::licensing::cached_status(app_handle).as_ref(),
            rotation_now_ms,
        );
        if block.is_some() {
            crate::licensing::recompute_status_async(app_handle, rotation_now_ms);
        }
        if let Some(outcome) = self.stop_for_license_block_at_rotation(Some(app_handle), block) {
            return outcome;
        }

        if let Some(outcome) =
            self.suspend_if_screen_session_missing_at_rotation(Some(app_handle), &active_sources)
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

        // System audio rotates on its own seam, whether or not the screen is
        // rotating — and whether or not there is a screen at all.
        if active_sources.system_audio {
            if let (Some(planner), Some(session)) = (
                system_audio_planner.as_ref(),
                self.runtime.active_system_audio_session.as_mut(),
            ) {
                next_system_audio_recording_file = session.advance_segment(
                    planner,
                    next_index,
                    system_audio_output_path.clone(),
                );
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
                app_handle,
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::native_capture::runtime::CaptureSuspension;
    use capture_types::CaptureOutputFiles;

    fn screen_only_sources() -> CaptureSources {
        CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }
    }

    fn system_audio_only_sources() -> CaptureSources {
        CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }
    }

    /// A running system-audio-only session whose tap failed to start: requested,
    /// unpaused, and no session — the shape that used to report itself healthy
    /// while recording nothing at all.
    fn tapless_system_audio_runtime() -> NativeCaptureRuntime {
        NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(system_audio_only_sources()),
            active_system_audio_session: None,
            ..Default::default()
        }
    }

    // Regression: `start_system_audio_family_for_runtime` logs "continuing without
    // it" on Err while `requested_sources.system_audio` stays true, and nothing
    // retried — the rotation arm needs a session, the tick early-returned without
    // one, and the zero-watchdog lives inside the session that was never created.
    // So a transient start failure (coreaudiod restarting, an aggregate-UID
    // collision) bought a whole session that recorded no system audio while the UI
    // showed a healthy recording — and under ADR 0052's audio-only sessions, a
    // recording that records literally nothing.
    #[test]
    fn a_tapless_session_that_still_wants_system_audio_is_retried() {
        assert!(should_retry_system_audio_start(
            &tapless_system_audio_runtime()
        ));
    }

    #[test]
    fn a_low_disk_suspension_is_never_retried_back_into_a_tap() {
        let mut runtime = tapless_system_audio_runtime();
        runtime.capture_suspension = Some(CaptureSuspension::with_kind(
            CaptureSuspensionKind::LowDisk,
            &CaptureErrorResponse {
                code: "capture_low_disk".to_string(),
                message: "low disk".to_string(),
            },
        ));

        assert!(
            !should_retry_system_audio_start(&runtime),
            "low disk is the one suspension that stops the tap on purpose, and it restarts it itself (ADR 0040)"
        );
    }

    #[test]
    fn a_paused_or_unrequested_system_audio_family_is_never_started_by_the_retry() {
        let mut paused = tapless_system_audio_runtime();
        paused
            .inactivity
            .set_family_paused_states(false, false, true);
        assert!(!should_retry_system_audio_start(&paused));

        let mut unrequested = tapless_system_audio_runtime();
        unrequested.requested_sources = Some(screen_only_sources());
        assert!(!should_retry_system_audio_start(&unrequested));

        let mut stopped = tapless_system_audio_runtime();
        stopped.is_running = false;
        assert!(!should_retry_system_audio_start(&stopped));
    }

    // The tap-start retry rides the zero-watchdog's ladder: 30s, doubling, capped
    // at 10 minutes. A start does synchronous Core Audio work, so an unbounded or
    // un-paced retry would probe it on every tick.
    #[test]
    fn the_tap_start_retry_backs_off_from_thirty_seconds_to_a_ten_minute_cap() {
        let mut lifecycle = RecordingLifecycle::default();
        let mut delays = Vec::new();

        for _ in 0..8 {
            lifecycle.schedule_system_audio_start_retry(0);
            delays.push(lifecycle.system_audio_start_retry_delay_ms);
        }

        assert_eq!(
            delays,
            vec![30_000, 60_000, 120_000, 240_000, 480_000, 600_000, 600_000, 600_000]
        );
        assert_eq!(lifecycle.system_audio_start_retry_at_ms, Some(600_000));
    }

    // A running screen-only runtime as the delegate-stop reconcile sees it: the
    // stream is dead, `active_screen_session` already `None`, with the in-flight
    // segment's finalized `.mov` still tracked as the current output.
    fn lifecycle_after_unexpected_screen_stop(screen_file: &str) -> RecordingLifecycle {
        RecordingLifecycle {
            runtime: NativeCaptureRuntime {
                is_running: true,
                requested_sources: Some(screen_only_sources()),
                current_segment_sources: Some(screen_only_sources()),
                output_files: Some(empty_output_files()),
                current_segment_output_files: Some(CaptureOutputFiles {
                    screen_file: Some(screen_file.to_string()),
                    screen_files: vec![screen_file.to_string()],
                    microphone_file: None,
                    microphone_files: Vec::new(),
                    system_audio_file: None,
                    system_audio_files: Vec::new(),
                }),
                recording_file: Some(screen_file.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn write_openable_screen_segment(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("screen-segment.mov");
        std::fs::write(&path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
            .expect("fake openable mov should be written");
        path.to_string_lossy().into_owned()
    }

    // Regression: a display sleep killed the ScreenCaptureKit stream (-3815),
    // the reconcile cleared screen state without entering a suspension, and the
    // next 60s rotation boundary rotated into the missing session — failing the
    // entire recording session. The reconcile must instead enter a
    // DisplayUnavailable suspension that keeps the session alive and commits
    // the in-flight tail segment.
    #[test]
    fn unexpected_screen_stop_suspends_and_commits_tail_instead_of_failing() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_file = write_openable_screen_segment(&temp_dir);
        let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);
        let error = CaptureErrorResponse {
            code: "capture_stream_system_stopped".to_string(),
            message: "ScreenCaptureKit stream stopped unexpectedly: Failed to find any displays or windows to capture (code: -3815)".to_string(),
        };

        let outcome = lifecycle.suspend_screen_after_unexpected_stop(None, &error);

        assert_eq!(outcome, TickOutcome::SkipRotation);
        let runtime = lifecycle.runtime();
        assert!(
            runtime.is_running,
            "an unexpected stream stop must never end the recording session"
        );
        assert_eq!(
            runtime
                .capture_suspension
                .as_ref()
                .map(|suspension| suspension.kind),
            Some(CaptureSuspensionKind::DisplayUnavailable),
            "the stop must land in the DisplayUnavailable suspension so recovery owns the restart"
        );
        assert!(runtime.recording_file.is_none());
        let committed = runtime
            .output_files
            .as_ref()
            .expect("committed outputs should exist");
        assert!(
            committed.screen_files.contains(&screen_file),
            "the in-flight tail segment must be committed, not orphaned"
        );
    }

    // Regression backstop at the rotation seam: screen active, no live session,
    // no suspension owner (e.g. a wake racing the will-sleep teardown) must
    // suspend and skip — before this guard, rotation hit `invalid_runtime_state`
    // and marked the whole session failed.
    #[test]
    fn rotation_with_missing_screen_session_suspends_instead_of_failing() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_file = write_openable_screen_segment(&temp_dir);
        let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);

        let outcome = lifecycle
            .suspend_if_screen_session_missing_at_rotation(None, &screen_only_sources());

        assert_eq!(outcome, Some(TickOutcome::SkipRotation));
        let runtime = lifecycle.runtime();
        assert!(runtime.is_running);
        assert_eq!(
            runtime
                .capture_suspension
                .as_ref()
                .map(|suspension| suspension.kind),
            Some(CaptureSuspensionKind::DisplayUnavailable),
        );
    }

    // License gate ≠ suspension (ADR 0021): a deny at either seam must refuse /
    // stop outright while `capture_suspension` stays None — a license block
    // never self-heals, so it must never enter the recovery machinery.

    #[test]
    fn license_block_at_start_refuses_with_the_gate_code_and_no_suspension() {
        let block = crate::licensing::license_block(
            Some(&capture_types::LicenseStatus::ReadOnly),
            0,
        );
        let refusal = RecordingLifecycle::refuse_start_for_license_block(block)
            .expect("read-only must refuse the start");
        assert_eq!(refusal.code, "capture_refused_read_only");

        let block = crate::licensing::license_block(
            Some(&capture_types::LicenseStatus::Revoked),
            0,
        );
        let refusal = RecordingLifecycle::refuse_start_for_license_block(block)
            .expect("revoked must refuse the start");
        assert_eq!(refusal.code, "capture_refused_revoked");
    }

    #[test]
    fn license_block_at_start_never_refuses_on_unknown_status() {
        // The deferred gate hasn't run yet (cached status None) → allow.
        assert!(RecordingLifecycle::refuse_start_for_license_block(
            crate::licensing::license_block(None, i64::MAX)
        )
        .is_none());
    }

    #[test]
    fn license_block_at_rotation_stops_the_loop_without_a_suspension() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_file = write_openable_screen_segment(&temp_dir);
        let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);

        let block = crate::licensing::license_block(
            Some(&capture_types::LicenseStatus::ReadOnly),
            0,
        );
        let outcome = lifecycle.stop_for_license_block_at_rotation(None, block);

        assert_eq!(outcome, Some(TickOutcome::StopLoop));
        let runtime = lifecycle.runtime();
        assert!(!runtime.is_running, "the session must end at the boundary");
        assert!(
            runtime.capture_suspension.is_none(),
            "a license block is a stop, never a suspension — it cannot self-heal"
        );
        // The healthy in-flight tail segment is committed, not orphaned.
        let committed = runtime
            .output_files
            .as_ref()
            .expect("committed outputs should exist");
        assert!(committed.screen_files.contains(&screen_file));
    }

    #[test]
    fn no_license_block_at_rotation_is_a_no_op() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_file = write_openable_screen_segment(&temp_dir);
        let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);

        let outcome = lifecycle.stop_for_license_block_at_rotation(
            None,
            crate::licensing::license_block(None, 0),
        );

        assert_eq!(outcome, None);
        assert!(lifecycle.runtime().is_running, "an allowed status must not stop the session");
    }

    // A system-audio-only session has no screen session, so the guard must not
    // read "no screen session" as a fault: suspending here would stop a perfectly
    // healthy audio-only recording at its first rotation (ADR 0052).
    #[test]
    fn rotation_guard_ignores_a_system_audio_only_session() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_file = write_openable_screen_segment(&temp_dir);
        let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);
        let sources = CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        };

        let outcome = lifecycle.suspend_if_screen_session_missing_at_rotation(None, &sources);

        assert_eq!(outcome, None);
        assert!(lifecycle.runtime().capture_suspension.is_none());
    }

    #[test]
    fn rotation_guard_ignores_audio_only_sources() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_file = write_openable_screen_segment(&temp_dir);
        let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);
        let sources = CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        };

        let outcome = lifecycle.suspend_if_screen_session_missing_at_rotation(None, &sources);

        assert_eq!(outcome, None);
        assert!(lifecycle.runtime().capture_suspension.is_none());
    }
}
