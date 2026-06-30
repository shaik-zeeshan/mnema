use super::activity::current_activity_snapshot;
use super::output::append_committed_segment_output_files;
#[cfg(target_os = "macos")]
use super::output::cleanup_unusable_segment_artifacts;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::output::finalize_capture_outputs;
use super::output::{
    set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
#[cfg(target_os = "macos")]
use super::runtime::{
    active_sources_for_inactivity_paused_state, ensure_system_audio_planner_for_runtime,
};
use super::runtime::{
    apply_runtime_signal, current_segment_sources_for_runtime, mark_runtime_session_failed,
    microphone_planner_for_runtime, refresh_runtime_planner_dates, screen_planner_for_runtime,
    system_audio_planner_for_runtime,
};
use super::runtime::{
    mark_runtime_session_stopped, request_segment_loop_stop, session_from_runtime,
    stopped_session_from_runtime, NativeCaptureRuntime,
};
#[cfg(target_os = "windows")]
use super::segments::start_windows_active_segment;
#[cfg(target_os = "macos")]
use super::segments::{apply_microphone_output_finalization, plan_live_rotation_segment};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::segments::{
    cleanup_failed_segment_dirs, create_segment_output_dirs, reanchor_active_segment_timing,
    stop_active_sessions_after_failure,
};
use super::segments::{empty_output_files, start_capture_runtime, stop_capture_runtime};
#[cfg(target_os = "macos")]
use super::segments::{
    handle_inactivity_resume_error, pause_microphone_for_inactivity_with_app_handle,
    pause_runtime_for_inactivity_with_app_handle, pause_screen_for_inactivity_with_app_handle,
    pause_system_audio_for_inactivity_with_app_handle, recover_from_segment_finalize_error,
    recover_screen_capture_after_wake, resume_microphone_from_inactivity,
    resume_runtime_from_inactivity, resume_screen_from_inactivity,
    resume_system_audio_from_inactivity, start_segment_with_current_privacy_filter,
};
#[cfg(target_os = "windows")]
use super::segments::{
    pause_microphone_for_inactivity_with_app_handle, pause_runtime_for_inactivity_with_app_handle,
    pause_runtime_for_system_suspend_with_app_handle, pause_screen_for_inactivity_with_app_handle,
    pause_screen_for_transient_liveness, pause_system_audio_for_inactivity_with_app_handle,
    resume_microphone_from_inactivity, resume_runtime_from_inactivity,
    resume_runtime_from_system_suspend, resume_screen_from_inactivity,
    resume_system_audio_from_inactivity,
};
use capture_runtime::RuntimeSignal;
use capture_types::{
    CaptureErrorResponse, CaptureSources, NativeCaptureSession, RecordingSettings,
};
use capture_vad::MicrophoneVadFallbackNotice;

#[derive(Debug, Default)]
pub(crate) struct RecordingLifecycle {
    runtime: NativeCaptureRuntime,
    /// In-process workstation-lock signal, maintained from the `WTS_SESSION_LOCK` /
    /// `WTS_SESSION_UNLOCK` handlers (ADR 0023). Read by the `DisplayAsleep` guarded
    /// resume so a display-on while the session is still locked never resumes
    /// capture — the `sleep-then-lock` overlap leaves the single `screen_pause_reason`
    /// as `DisplayAsleep` (the already-paused screen makes the lock pause a no-op), so
    /// the lock fact has to be tracked out-of-band rather than inferred from the
    /// reason. This reuses the existing `SessionLock` notification rather than issuing
    /// a fresh Win32 lock query at display-on time; it is not multi-reason stacking.
    #[cfg(target_os = "windows")]
    session_locked: bool,
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
    /// React to the Windows screen capture session stopping mid-recording.
    ///
    /// Per ADR 0023, a `GraphicsCaptureItem.Closed` / not-live signal (monitor
    /// disconnect, lid close, session lock, sleep) is a transient liveness
    /// condition, not a fatal error: instead of failing the session we enter a
    /// screen-only transient suspension (reusing the inactivity pause path) and
    /// keep the session alive so the throttled display-present probe in
    /// `tick_inactivity` can auto-resume it. Only a genuine, non-transient stop
    /// error still fails the session.
    ///
    /// Guards against re-triggering while already transient-paused: the pause path
    /// clears `active_screen_session` (so the not-live branch can't fire again) and
    /// sets `screen_paused`, which both branches below check.
    #[cfg(target_os = "windows")]
    fn handle_windows_screen_capture_stop(&mut self) -> TickOutcome {
        if self.runtime.inactivity.is_screen_paused() {
            // Already suspended (inactivity or transient liveness). The dead
            // session has been stopped/cleared; nothing to react to until resume.
            return TickOutcome::Continue;
        }

        let stop_error = capture_screen::take_screen_capture_session_stop_error(
            self.runtime.active_screen_session.as_mut(),
        );
        if let Some(error) = stop_error {
            if capture_screen::screen_capture_stop_error_is_transient_liveness(&error.code) {
                return self.suspend_windows_screen_for_transient_liveness(
                    super::inactivity::TransientLivenessTrigger::DisplayUnavailable,
                    &format!("backend stop error [{}] {}", error.code, error.message),
                );
            }

            super::debug_log::log(format!(
                "windows screen capture stopped unexpectedly; failing runtime session: [{}] {}",
                error.code, error.message
            ));
            stop_active_sessions_after_failure(&mut self.runtime);
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }

        if self.runtime.active_screen_session.is_some()
            && !capture_screen::screen_capture_session_is_live(
                self.runtime.active_screen_session.as_ref(),
            )
        {
            // Not-live without a surfaced backend stop error is the same
            // display-death family (ADR 0023) → transient suspension, not failure.
            return self.suspend_windows_screen_for_transient_liveness(
                super::inactivity::TransientLivenessTrigger::DisplayUnavailable,
                "session not live without a backend error",
            );
        }

        TickOutcome::Continue
    }

    /// Enter a screen-only transient-liveness suspension and keep the session
    /// alive (ADR 0023). The screen capture session is already dead/unavailable;
    /// the pause path finalizes the partial segment and records the supplied
    /// `TransientLiveness { trigger }` reason.
    #[cfg(target_os = "windows")]
    fn suspend_windows_screen_for_transient_liveness(
        &mut self,
        trigger: super::inactivity::TransientLivenessTrigger,
        cause: &str,
    ) -> TickOutcome {
        super::debug_log::log(format!(
            "windows screen capture became unavailable ({cause}); suspending screen as transient liveness and keeping the session alive (ADR 0023)"
        ));
        if let Err(error) = pause_screen_for_transient_liveness(&mut self.runtime, trigger) {
            // The pause path tolerates the dead-session stop error internally, so a
            // returned error here is a genuine bookkeeping failure; reconcile and
            // continue rather than killing the recording on a transient condition.
            super::debug_log::log(format!(
                "windows transient-liveness screen suspension reported an issue: [{}] {}",
                error.code, error.message
            ));
        }
        TickOutcome::Continue
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn handle_windows_session_lock(&mut self) -> Option<NativeCaptureSession> {
        // Record the workstation-lock fact regardless of whether the screen pause
        // below proceeds: in the `sleep-then-lock` ordering the screen is already
        // paused for `DisplayAsleep`, so the pause itself no-ops, but the
        // `DisplayAsleep` guarded resume still has to know the session is locked.
        self.session_locked = true;

        if !self.runtime.is_running
            || self.runtime.user_capture_paused
            || self.runtime.inactivity.is_screen_paused()
            || !self
                .runtime
                .requested_sources
                .as_ref()
                .is_some_and(|sources| sources.screen)
        {
            return None;
        }

        if let Err(error) = pause_screen_for_transient_liveness(
            &mut self.runtime,
            super::inactivity::TransientLivenessTrigger::SessionLock,
        ) {
            super::debug_log::log(format!(
                "windows session-lock screen suspension reported an issue: [{}] {}",
                error.code, error.message
            ));
        }

        if matches!(
            self.runtime.inactivity.screen_pause_reason(),
            Some(super::inactivity::ScreenPauseReason::TransientLiveness {
                trigger: super::inactivity::TransientLivenessTrigger::SessionLock,
            })
        ) {
            Some(session_from_runtime(&self.runtime))
        } else {
            None
        }
    }

    #[cfg(target_os = "windows")]
    fn windows_session_unlock_can_resume_screen(&self) -> bool {
        matches!(
            self.runtime.inactivity.screen_pause_reason(),
            Some(super::inactivity::ScreenPauseReason::TransientLiveness {
                trigger: super::inactivity::TransientLivenessTrigger::SessionLock,
            })
        )
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn handle_windows_session_unlock(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Option<NativeCaptureSession> {
        // The workstation is unlocked again; clear the lock signal so a subsequent
        // `DisplayAsleep` display-on can resume. Done before the screen-resume gate so
        // the flag tracks OS lock state even when this unlock cannot itself resume the
        // screen (e.g. the screen is paused for `DisplayAsleep`, not `SessionLock`).
        self.session_locked = false;

        if !self.windows_session_unlock_can_resume_screen() {
            return None;
        }

        if let Err(error) = resume_screen_from_inactivity(&mut self.runtime, Some(app_handle)) {
            let microphone_paused = self.runtime.inactivity.microphone_paused;
            let system_audio_paused = self.runtime.inactivity.system_audio_paused;
            self.runtime
                .inactivity
                .set_family_paused_states_with_reason(
                    true,
                    microphone_paused,
                    system_audio_paused,
                    super::inactivity::ScreenPauseReason::TransientLiveness {
                        trigger: super::inactivity::TransientLivenessTrigger::SessionLock,
                    },
                );
            super::debug_log::log(format!(
                "windows session-unlock screen resume failed; leaving screen suspended: [{}] {}",
                error.code, error.message
            ));
            return None;
        }

        Some(session_from_runtime(&self.runtime))
    }

    /// Pause screen capture because the console display went to sleep (DPMS off,
    /// `GUID_CONSOLE_DISPLAY_STATE` → off; ADR 0023). Mirrors
    /// [`handle_windows_session_lock`]: a screen-only transient-liveness pause that
    /// keeps the session alive. No-ops when capture is not running, is user-paused,
    /// the screen is already paused (so an existing `SessionLock` / `SystemSuspend` /
    /// `DisplayUnavailable` reason is never downgraded to `DisplayAsleep`), or the
    /// screen is not a requested source.
    #[cfg(target_os = "windows")]
    pub(crate) fn handle_windows_display_asleep(&mut self) -> Option<NativeCaptureSession> {
        if !self.runtime.is_running
            || self.runtime.user_capture_paused
            || self.runtime.inactivity.is_screen_paused()
            || !self
                .runtime
                .requested_sources
                .as_ref()
                .is_some_and(|sources| sources.screen)
        {
            return None;
        }

        if let Err(error) = pause_screen_for_transient_liveness(
            &mut self.runtime,
            super::inactivity::TransientLivenessTrigger::DisplayAsleep,
        ) {
            super::debug_log::log(format!(
                "windows display-asleep screen suspension reported an issue: [{}] {}",
                error.code, error.message
            ));
        }

        if matches!(
            self.runtime.inactivity.screen_pause_reason(),
            Some(super::inactivity::ScreenPauseReason::TransientLiveness {
                trigger: super::inactivity::TransientLivenessTrigger::DisplayAsleep,
            })
        ) {
            Some(session_from_runtime(&self.runtime))
        } else {
            None
        }
    }

    /// Whether a console display-on (`GUID_CONSOLE_DISPLAY_STATE` → on) should resume
    /// screen capture *now* (ADR 0023). The `DisplayAsleep` resume is event-driven —
    /// the throttled display-present probe cannot observe DPMS — and is guarded so a
    /// display-on never resumes capture while the session is otherwise unavailable.
    /// Resumes only when ALL hold:
    /// 1. the single screen pause reason is `TransientLiveness { DisplayAsleep }`
    ///    (so a `SessionLock` / `SystemSuspend` / `DisplayUnavailable` pause, which
    ///    own their own resume paths, is never resumed here — the lock-then-sleep
    ///    ordering keeps the reason `SessionLock` and falls out here), AND
    /// 2. the session is not locked (the out-of-band `session_locked` signal, set by
    ///    the existing `WTS_SESSION_LOCK`/`UNLOCK` handlers — needed because the
    ///    sleep-then-lock ordering leaves the reason as `DisplayAsleep`), AND
    /// 3. the system is not suspend-paused (`is_system_suspend_paused`).
    ///
    /// When the guard fails the display-on is a no-op and capture stays paused;
    /// whatever later clears the lock/suspend resumes via its own existing path.
    #[cfg(target_os = "windows")]
    fn windows_display_awake_can_resume_screen(&self) -> bool {
        matches!(
            self.runtime.inactivity.screen_pause_reason(),
            Some(super::inactivity::ScreenPauseReason::TransientLiveness {
                trigger: super::inactivity::TransientLivenessTrigger::DisplayAsleep,
            })
        ) && !self.session_locked
            && !self.runtime.inactivity.is_system_suspend_paused()
    }

    /// Resume screen capture because the console display woke (DPMS on; ADR 0023),
    /// gated by [`windows_display_awake_can_resume_screen`]. Mirrors
    /// [`handle_windows_session_unlock`]: on a resume failure the screen is left
    /// suspended with its `DisplayAsleep` reason intact (never fail the session on a
    /// transient condition) and `None` is returned.
    #[cfg(target_os = "windows")]
    pub(crate) fn handle_windows_display_awake(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Option<NativeCaptureSession> {
        if !self.windows_display_awake_can_resume_screen() {
            return None;
        }

        if let Err(error) = resume_screen_from_inactivity(&mut self.runtime, Some(app_handle)) {
            let microphone_paused = self.runtime.inactivity.microphone_paused;
            let system_audio_paused = self.runtime.inactivity.system_audio_paused;
            self.runtime
                .inactivity
                .set_family_paused_states_with_reason(
                    true,
                    microphone_paused,
                    system_audio_paused,
                    super::inactivity::ScreenPauseReason::TransientLiveness {
                        trigger: super::inactivity::TransientLivenessTrigger::DisplayAsleep,
                    },
                );
            super::debug_log::log(format!(
                "windows display-on screen resume failed; leaving screen suspended: [{}] {}",
                error.code, error.message
            ));
            return None;
        }

        Some(session_from_runtime(&self.runtime))
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn handle_windows_system_suspend(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Option<NativeCaptureSession> {
        if !self.runtime.is_running || self.runtime.user_capture_paused {
            return None;
        }

        match pause_runtime_for_system_suspend_with_app_handle(&mut self.runtime, Some(app_handle)) {
            Ok(true) => Some(session_from_runtime(&self.runtime)),
            Ok(false) => None,
            Err(error) => {
                super::debug_log::log(format!(
                    "windows system-suspend transient-liveness suspension failed; keeping session alive: [{}] {}",
                    error.code, error.message
                ));
                None
            }
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn handle_windows_system_resume(
        &mut self,
        app_handle: &tauri::AppHandle,
    ) -> Option<NativeCaptureSession> {
        if !self.runtime.is_running || self.runtime.user_capture_paused {
            return None;
        }

        match resume_runtime_from_system_suspend(&mut self.runtime, Some(app_handle)) {
            Ok(true) => Some(session_from_runtime(&self.runtime)),
            Ok(false) => None,
            Err(error) => {
                super::debug_log::log(format!(
                    "windows system-resume transient-liveness restart failed; leaving suspended families paused for retry: [{}] {}",
                    error.code, error.message
                ));
                None
            }
        }
    }

    /// Decide whether a transient-liveness screen pause should resume *now*,
    /// encoding the exact gate→mark→predicate ordering the tick relies on (ADR
    /// 0023). Returns `true` only when:
    /// 1. the screen is paused for a `TransientLiveness` reason (cross-trigger
    ///    isolation — an `Inactivity` pause is never resumed here), and
    /// 2. a probe is due per the throttle window, and
    /// 3. the display-present probe (invoked only when due, so the Win32 call stays
    ///    rate limited) reports a display is back.
    ///
    /// When a probe is due, the throttle marker is advanced *before* the resume
    /// predicate is evaluated — the same `now`. The predicate
    /// (`should_resume_screen_from_transient_liveness`) therefore must not re-check
    /// the throttle (it would see the just-set marker and always return false,
    /// making auto-resume dead code); this helper owns the single throttle gate.
    /// Factored out of [`try_resume_windows_screen_from_transient_liveness`] so the
    /// ordering can be tested without a real `AppHandle` or Win32 probe.
    #[cfg(target_os = "windows")]
    fn transient_liveness_resume_decision(
        &mut self,
        now: u64,
        probe_display_present: impl FnOnce() -> bool,
    ) -> bool {
        use super::inactivity::{ScreenPauseReason, TransientLivenessTrigger};

        // Cross-trigger isolation (ADR 0023): the display-present probe
        // (`windows_display_present` = `SM_CMONITORS > 0`) is true during `Win+L`
        // because the monitors stay attached, so it must NOT resume a `SessionLock`
        // pause — that would restart WGC against the locked secure desktop. A
        // session lock resumes EXCLUSIVELY via `WTS_SESSION_UNLOCK`. Only
        // `DisplayUnavailable` / `SystemSuspend` recover through this probe.
        if !matches!(
            self.runtime.inactivity.screen_pause_reason(),
            Some(ScreenPauseReason::TransientLiveness {
                trigger:
                    TransientLivenessTrigger::DisplayUnavailable
                    | TransientLivenessTrigger::SystemSuspend
            })
        ) {
            return false;
        }
        if !self.runtime.inactivity.is_transient_liveness_probe_due(now) {
            return false;
        }

        self.runtime.inactivity.mark_transient_liveness_probe(now);
        let display_present = probe_display_present();
        self.runtime
            .inactivity
            .should_resume_screen_from_transient_liveness(display_present, now)
    }

    /// Throttled display-present probe + auto-resume for a transient-liveness
    /// screen pause (ADR 0023). Only runs when the screen is paused for a
    /// `TransientLiveness` reason (cross-trigger isolation — an `Inactivity` pause
    /// is never resumed here), and only attempts a resume when a probe is due and a
    /// display/session is present again. A resume failure is logged and tolerated;
    /// the screen stays suspended with its reason intact for the next probe.
    #[cfg(target_os = "windows")]
    fn try_resume_windows_screen_from_transient_liveness(
        &mut self,
        now: u64,
        app_handle: &tauri::AppHandle,
    ) {
        // Decide whether to resume using the real display-present probe, in the
        // same gate→mark→predicate ordering the decision helper encodes (and which
        // the regression test drives with a stubbed display value). The probe is
        // only invoked when a probe is actually due, so the throttle still rate
        // limits the Win32 call. Route through `screen_display_available` for
        // symmetry with the macOS recovery path (it delegates to
        // `windows_display_present` on Windows).
        if !self.transient_liveness_resume_decision(now, capture_screen::screen_display_available) {
            return;
        }

        if let Err(error) = resume_screen_from_inactivity(&mut self.runtime, Some(app_handle)) {
            // Display raced away again or WGC re-init failed: never fail the
            // session on a transient resume (mirrors macOS retry philosophy). The
            // screen stays suspended with its `TransientLiveness` reason; the next
            // throttled probe retries.
            super::debug_log::log(format!(
                "windows transient-liveness screen resume attempt failed; leaving screen suspended and retrying on the next probe: [{}] {}",
                error.code, error.message
            ));
            return;
        }

        super::debug_log::log(
            "resumed Windows screen capture after transient-liveness recovery (display/session present again)"
                .to_string(),
        );
    }

    /// Recreate audio families whose WASAPI session was detached by a system-suspend
    /// pause but whose wake re-init failed (`resume_runtime_from_system_suspend`).
    /// Such a family is requested + paused but has no live session, and — unlike the
    /// screen, which recovers via the display-present probe — it has no
    /// activity-driven resume trigger, because its activity atomics stall while the
    /// session is detached. Recreate each family independently on a throttled cadence
    /// so a permanently-failing device never blocks the others, and so an audio-only
    /// session (no screen to ride along with) still recovers (ADR 0023). A normal
    /// inactivity pause keeps the session attached, so this only fires after a
    /// detaching suspend pause.
    #[cfg(target_os = "windows")]
    fn try_recover_detached_windows_audio_families(
        &mut self,
        now: u64,
        app_handle: &tauri::AppHandle,
    ) {
        let Some(requested) = self.runtime.requested_sources.clone() else {
            return;
        };
        let microphone_detached = requested.microphone
            && self.runtime.inactivity.is_microphone_paused()
            && self.runtime.active_microphone_session.is_none();
        let system_audio_detached = requested.system_audio
            && self.runtime.inactivity.is_system_audio_paused()
            && self.runtime.active_system_audio_session.is_none();
        if !microphone_detached && !system_audio_detached {
            return;
        }
        if !self.runtime.inactivity.is_detached_audio_recovery_due(now) {
            return;
        }
        self.runtime
            .inactivity
            .mark_detached_audio_recovery_probe(now);

        if microphone_detached {
            if let Err(error) =
                resume_microphone_from_inactivity(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to recover detached Windows microphone after system suspend; will retry on the next probe: [{}] {}",
                    error.code, error.message
                ));
            } else {
                super::debug_log::log(
                    "recovered detached Windows microphone after a failed system-suspend wake re-init"
                        .to_string(),
                );
            }
        }
        if system_audio_detached {
            if let Err(error) =
                resume_system_audio_from_inactivity(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to recover detached Windows system audio after system suspend; will retry on the next probe: [{}] {}",
                    error.code, error.message
                ));
            } else {
                super::debug_log::log(
                    "recovered detached Windows system audio after a failed system-suspend wake re-init"
                        .to_string(),
                );
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn all_requested_families_paused_for_inactivity(&self) -> bool {
        let Some(sources) = self.runtime.requested_sources.as_ref() else {
            return false;
        };

        let has_requested_source = sources.screen || sources.microphone || sources.system_audio;
        has_requested_source
            && (!sources.screen || self.runtime.inactivity.is_screen_paused())
            && (!sources.microphone || self.runtime.inactivity.is_microphone_paused())
            && (!sources.system_audio || self.runtime.inactivity.is_system_audio_paused())
    }

    /// True when the screen is paused for a transient-liveness reason (ADR 0023).
    /// Used to keep activity-driven resume paths from resuming a screen whose
    /// display may still be gone — only the throttled display-present probe resumes
    /// a transient-liveness pause.
    #[cfg(target_os = "windows")]
    fn screen_paused_for_transient_liveness(&self) -> bool {
        matches!(
            self.runtime.inactivity.screen_pause_reason(),
            Some(super::inactivity::ScreenPauseReason::TransientLiveness { .. })
        )
    }

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
        #[cfg(target_os = "windows")]
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
            self.runtime
                .output_files
                .get_or_insert_with(empty_output_files);
            self.runtime.runtime_controller = Default::default();
            apply_runtime_signal(&mut self.runtime, RuntimeSignal::StartRequested)?;
            start_windows_active_segment(
                Some(app_handle),
                &mut self.runtime,
                &sources,
                "resuming user pause",
            )?;
            // A user resume starts a fresh full segment for every requested family,
            // so any lingering inactivity/transient-liveness family-pause state is
            // now stale against a live session (ADR 0023). Clearing it here also
            // clears `screen_pause_reason` and the pause-start timestamp, so the
            // transient-liveness probe stops watching a screen that is live again and
            // the activity resume-all path is no longer wrongly gated.
            self.runtime
                .inactivity
                .set_family_paused_states(false, false, false);
            apply_runtime_signal(&mut self.runtime, RuntimeSignal::SourcesReady)?;
        }
        self.runtime.user_capture_paused = false;
        self.runtime.current_segment_sources = self.runtime.requested_sources.clone();
        if let Some(control) = self.runtime.segment_loop_control.as_ref() {
            control.notify();
        }
        Ok(self.session())
    }

    #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
        {
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
            #[cfg(target_os = "macos")]
            let resume_result = resume_microphone_from_inactivity(&mut self.runtime);
            #[cfg(target_os = "windows")]
            let resume_result =
                resume_microphone_from_inactivity(&mut self.runtime, Some(app_handle));
            if let Err(error) = resume_result {
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
            #[cfg(target_os = "macos")]
            let resume_result = resume_system_audio_from_inactivity(&mut self.runtime);
            #[cfg(target_os = "windows")]
            let resume_result =
                resume_system_audio_from_inactivity(&mut self.runtime, Some(app_handle));
            if let Err(error) = resume_result {
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

        #[cfg(target_os = "macos")]
        let privacy_suspended = self.runtime.capture_suspension.is_some();
        #[cfg(target_os = "windows")]
        let privacy_suspended = false;

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

    #[cfg(target_os = "windows")]
    pub(crate) fn tick_inactivity(&mut self, app_handle: &tauri::AppHandle) -> TickOutcome {
        if self.runtime.user_capture_paused {
            return TickOutcome::SkipRotation;
        }
        if self.handle_windows_screen_capture_stop() == TickOutcome::StopLoop {
            return TickOutcome::StopLoop;
        }

        let now = super::runtime::now_monotonic_marker_ms();
        if self.runtime.inactivity.is_system_suspend_paused() {
            return TickOutcome::SkipRotation;
        }

        // Throttled transient-liveness auto-resume (ADR 0023): when the screen is
        // paused for a transient-liveness reason and a probe is due, check whether
        // a display/session is present again and, if so, resume the screen via the
        // shared inactivity resume path. A failed resume (display raced away, WGC
        // init error) must never fail the session — log and let the next throttled
        // probe retry, mirroring macOS's `DISPLAY_UNAVAILABLE_RECOVERY_INTERVAL`.
        self.try_resume_windows_screen_from_transient_liveness(now, app_handle);
        // Recover audio families whose WASAPI session was detached by a
        // system-suspend pause whose wake re-init failed; they have no
        // activity-driven resume trigger while detached (ADR 0023).
        self.try_recover_detached_windows_audio_families(now, app_handle);
        let activity_snapshot = current_activity_snapshot(&mut self.runtime);
        let effective_idle = self
            .runtime
            .inactivity
            .effective_idle_for_snapshot(now, activity_snapshot);

        // Cross-trigger isolation (ADR 0023): the activity-driven resume-all-families
        // branch must never resume a screen paused for transient liveness — the
        // display may still be gone. The throttled display-present probe above owns
        // that resume; audio families still resume via their per-family blocks below.
        if !self.screen_paused_for_transient_liveness()
            && self.all_requested_families_paused_for_inactivity()
            && effective_idle.idle_ms
                < self
                    .runtime
                    .inactivity
                    .idle_timeout_seconds
                    .saturating_mul(1000)
        {
            if let Err(error) = resume_runtime_from_inactivity(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to resume Windows native capture after activity: [{}] {}",
                    error.code, error.message
                ));
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            }
            super::debug_log::log(format!(
                "resumed Windows native capture after activity (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                effective_idle.idle_ms,
                effective_idle.source.as_str(),
                self.runtime.inactivity.idle_timeout_seconds
            ));
            return TickOutcome::SkipRotation;
        }

        if self
            .runtime
            .inactivity
            .should_resume_microphone_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) =
                resume_microphone_from_inactivity(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to resume Windows microphone capture after activity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = self
                    .runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "resumed Windows microphone capture after activity (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
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
                    "failed to pause Windows microphone capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let mic_eval = self
                    .runtime
                    .inactivity
                    .evaluate_microphone_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "paused Windows microphone capture for inactivity threshold crossing (microphone_effective_idle_ms={}, microphone_effective_source={}, idle_timeout_seconds={})",
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
            if let Err(error) =
                resume_system_audio_from_inactivity(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to resume Windows system-audio capture after activity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = self
                    .runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "resumed Windows system-audio capture after activity (system_audio_effective_idle_ms={}, system_audio_effective_source={}, idle_timeout_seconds={})",
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
                    "failed to pause Windows system-audio capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
            } else {
                let sa_eval = self
                    .runtime
                    .inactivity
                    .evaluate_system_audio_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "paused Windows system-audio capture for inactivity threshold crossing (system_audio_effective_idle_ms={}, system_audio_effective_source={}, idle_timeout_seconds={})",
                    sa_eval.effective_idle.idle_ms,
                    sa_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_resume_screen_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_screen_from_inactivity(&mut self.runtime, Some(app_handle)) {
                super::debug_log::log(format!(
                    "failed to resume Windows screen capture after activity: [{}] {}",
                    error.code, error.message
                ));
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            } else {
                let screen_eval = self
                    .runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "resumed Windows screen capture after activity (screen_effective_idle_ms={}, screen_effective_source={}, idle_timeout_seconds={})",
                    screen_eval.effective_idle.idle_ms,
                    screen_eval.effective_idle.source.as_str(),
                    self.runtime.inactivity.idle_timeout_seconds
                ));
            }
        }

        if self
            .runtime
            .inactivity
            .should_pause_screen_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) =
                pause_screen_for_inactivity_with_app_handle(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to pause Windows screen capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            } else {
                let screen_eval = self
                    .runtime
                    .inactivity
                    .evaluate_screen_policy_for_snapshot(now, activity_snapshot);
                super::debug_log::log(format!(
                    "paused Windows screen capture for inactivity threshold crossing (screen_effective_idle_ms={}, screen_effective_source={}, idle_timeout_seconds={})",
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
            if let Err(error) = resume_runtime_from_inactivity(&mut self.runtime, Some(app_handle))
            {
                super::debug_log::log(format!(
                    "failed to resume Windows native capture after activity: [{}] {}",
                    error.code, error.message
                ));
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            } else {
                super::debug_log::log(format!(
                    "resumed Windows native capture after activity (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
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
                super::debug_log::log(format!(
                    "failed to pause Windows native capture for inactivity: [{}] {}",
                    error.code, error.message
                ));
                stop_active_sessions_after_failure(&mut self.runtime);
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            } else {
                super::debug_log::log(format!(
                    "paused Windows native capture for inactivity threshold crossing (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
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
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "windows")]
        let active_sources = current_segment_sources_for_runtime(&self.runtime).unwrap_or(sources);

        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "windows")]
        let system_audio_planner = system_audio_planner_for_runtime(&self.runtime).cloned();

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
        #[cfg(target_os = "macos")]
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
                #[cfg(target_os = "macos")]
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

        #[cfg(target_os = "macos")]
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
        }

        #[cfg(target_os = "macos")]
        if !legacy_rotated && active_sources.microphone {
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

        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "windows")]
        let previous_segment_committed = true;

        if previous_segment_committed {
            if let (Some(committed), Some(segment)) = (
                self.runtime.output_files.as_mut(),
                previous_segment_output_files.as_ref(),
            ) {
                append_committed_segment_output_files(committed, segment);
            }
            #[cfg(target_os = "macos")]
            super::segments::persist_committed_audio_segments(
                Some(app_handle),
                self.runtime.source_sessions.as_ref(),
                self.runtime.segment_schedule.as_ref(),
                self.runtime.current_segment_index,
                previous_segment_output_files.as_ref(),
            );
            #[cfg(target_os = "macos")]
            super::segments::warm_scrub_previews_for_committed_screen_outputs(
                Some(app_handle),
                previous_segment_output_files.as_ref(),
            );
        }

        self.runtime.current_segment_index = next_index;
        self.runtime.current_segment_output_files = Some(next_segment_outputs);
        #[cfg(target_os = "macos")]
        {
            self.runtime.current_segment_sources = active_sources_for_inactivity_paused_state(
                &active_sources,
                self.runtime.inactivity.screen_paused,
                self.runtime.inactivity.microphone_paused,
                self.runtime.inactivity.system_audio_paused,
            );
        }
        #[cfg(target_os = "windows")]
        {
            self.runtime.current_segment_sources = Some(active_sources);
        }
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

    /// Windows segment rotation across the independent screen and microphone
    /// sources (ADR 0022). Unlike macOS, an audio-only session has no screen
    /// planner/session and no per-segment screen workspace: the screen-stopped
    /// check and the screen rotation arm are gated on whether screen is actually
    /// an active source, and the microphone arm rotates the WASAPI/MF session on
    /// the same wall-clock `CaptureClock`/`SegmentSchedule` boundary.
    #[cfg(target_os = "windows")]
    pub(crate) fn tick_rotation(&mut self, app_handle: &tauri::AppHandle) -> TickOutcome {
        // Screen liveness is only meaningful when screen is an active source; the
        // helper is a no-op when there is no active screen session (audio-only).
        if self.handle_windows_screen_capture_stop() == TickOutcome::StopLoop {
            return TickOutcome::StopLoop;
        }

        refresh_runtime_planner_dates(&mut self.runtime);

        let Some(_sources) = self.runtime.requested_sources.clone() else {
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

        if !schedule.segment_duration_reached(clock.elapsed()) {
            return TickOutcome::SkipRotation;
        }

        let active_sources =
            current_segment_sources_for_runtime(&self.runtime).unwrap_or(CaptureSources {
                screen: false,
                microphone: false,
                system_audio: false,
            });
        if !active_sources.screen && !active_sources.microphone && !active_sources.system_audio {
            return TickOutcome::SkipRotation;
        }

        // A screen planner is required only when screen is an active source; an
        // audio-only session rotates without one.
        let screen_planner = screen_planner_for_runtime(&self.runtime).cloned();
        if active_sources.screen && screen_planner.is_none() {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }
        let microphone_planner = microphone_planner_for_runtime(&self.runtime).cloned();
        let system_audio_planner = system_audio_planner_for_runtime(&self.runtime).cloned();

        let next_index =
            super::segments::next_emitted_segment_index(self.runtime.current_segment_index);

        let recording_file = self.runtime.recording_file.clone();
        let microphone_recording_file = self.runtime.microphone_recording_file.clone();
        let system_audio_recording_file = self.runtime.system_audio_recording_file.clone();
        let requested_sources = self.runtime.requested_sources.clone();
        let mut previous_segment_output_files = self.runtime.current_segment_output_files.clone();

        if apply_runtime_signal(&mut self.runtime, RuntimeSignal::RotateRequested).is_err() {
            mark_runtime_session_failed(&mut self.runtime);
            return TickOutcome::StopLoop;
        }

        let mut next_segment_outputs = empty_output_files();
        let mut next_recording_file = self.runtime.recording_file.clone();
        let mut next_microphone_recording_file = self.runtime.microphone_recording_file.clone();
        let mut next_system_audio_recording_file = self.runtime.system_audio_recording_file.clone();
        let mut screen_segment_dir: Option<std::path::PathBuf> = None;

        // Thread the probed finalization duration of each rotated-out audio source
        // through to `persist_committed_audio_segments` so non-final audio segments
        // get their real `.m4a` duration rather than the scheduled fallback
        // (`audio_file_duration_ms` is a None stub on Windows). Mirrors the stop
        // path's `stop_known_durations` map.
        let mut rotation_known_durations: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        // --- Screen rotation (only when screen is an active source) ---
        if active_sources.screen {
            let screen_planner = screen_planner
                .as_ref()
                .expect("screen planner is present when screen is an active source");
            let segment_dir = screen_planner.segment_dir(next_index);
            let screen_output_file = screen_planner.segment_screen_output(next_index);
            if let Err(error) =
                create_segment_output_dirs(&segment_dir, None, None, &active_sources)
            {
                super::debug_log::log(format!(
                    "failed to prepare Windows screen segment directory while rotating: [{}] {}",
                    error.code, error.message
                ));
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            }
            screen_segment_dir = Some(segment_dir.clone());

            match capture_screen::rotate_screen_capture_session(
                capture_screen::RotateScreenCaptureSessionArgs {
                    active_session: &mut self.runtime.active_screen_session,
                    segment_dir: &segment_dir,
                    screen_output_file: Some(&screen_output_file),
                    system_audio_output_path: None,
                },
            ) {
                Ok(rotated) => {
                    if let Some(file) = rotated.output_files.screen_file {
                        set_current_screen_output_file(&mut next_segment_outputs, file);
                    }
                    next_recording_file = Some(rotated.recording_file);
                }
                Err(_) => {
                    cleanup_failed_segment_dirs(&segment_dir, None, None);
                    stop_active_sessions_after_failure(&mut self.runtime);
                    mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            }
        }

        // --- Microphone rotation (only when microphone is an active source) ---
        if active_sources.microphone {
            let Some(planner) = microphone_planner.as_ref() else {
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            };
            let microphone_output_path = planner.microphone_file(next_index);
            if let Some(audio_dir) = microphone_output_path.parent() {
                if let Err(error) = std::fs::create_dir_all(audio_dir) {
                    super::debug_log::log(format!(
                        "failed to prepare Windows microphone audio directory while rotating: {error}"
                    ));
                    if let Some(dir) = screen_segment_dir.as_deref() {
                        cleanup_failed_segment_dirs(dir, None, None);
                    }
                    stop_active_sessions_after_failure(&mut self.runtime);
                    mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            }
            let microphone_output_file = microphone_output_path.to_string_lossy().to_string();
            if let Some(session) = self.runtime.active_microphone_session.as_mut() {
                match session.rotate_output_file_returning_finalization(&microphone_output_file) {
                    Ok(finalization) => {
                        if let (Some(file), Some(ms)) =
                            (finalization.output_file.as_deref(), finalization.duration_ms)
                        {
                            rotation_known_durations.insert(file.to_string(), ms);
                        }
                        super::segments::apply_windows_microphone_output_finalization(
                            previous_segment_output_files.as_mut(),
                            &finalization,
                        );
                    }
                    Err(error) => {
                        super::debug_log::log(format!(
                            "failed to rotate Windows microphone capture: [{}] {}",
                            error.code, error.message
                        ));
                        if let Some(dir) = screen_segment_dir.as_deref() {
                            cleanup_failed_segment_dirs(dir, None, None);
                        }
                        stop_active_sessions_after_failure(&mut self.runtime);
                        mark_runtime_session_failed(&mut self.runtime);
                        return TickOutcome::StopLoop;
                    }
                }
                set_current_microphone_output_file(
                    &mut next_segment_outputs,
                    microphone_output_file.clone(),
                );
                next_microphone_recording_file = Some(microphone_output_file);
            }
        }

        // --- System audio rotation (independent WASAPI render-loopback source) ---
        if active_sources.system_audio {
            let Some(planner) = system_audio_planner.as_ref() else {
                mark_runtime_session_failed(&mut self.runtime);
                return TickOutcome::StopLoop;
            };
            let system_audio_output_path = planner.system_audio_file(next_index);
            if let Some(audio_dir) = system_audio_output_path.parent() {
                if let Err(error) = std::fs::create_dir_all(audio_dir) {
                    super::debug_log::log(format!(
                        "failed to prepare Windows system-audio directory while rotating: {error}"
                    ));
                    if let Some(dir) = screen_segment_dir.as_deref() {
                        cleanup_failed_segment_dirs(dir, None, None);
                    }
                    stop_active_sessions_after_failure(&mut self.runtime);
                    mark_runtime_session_failed(&mut self.runtime);
                    return TickOutcome::StopLoop;
                }
            }
            let system_audio_output_file = system_audio_output_path.to_string_lossy().to_string();
            if let Some(session) = self.runtime.active_system_audio_session.as_mut() {
                match session.rotate_output_file_returning_finalization(&system_audio_output_file) {
                    Ok(finalization) => {
                        if let (Some(file), Some(ms)) =
                            (finalization.output_file.as_deref(), finalization.duration_ms)
                        {
                            rotation_known_durations.insert(file.to_string(), ms);
                        }
                        super::segments::apply_windows_system_audio_output_finalization(
                            previous_segment_output_files.as_mut(),
                            &finalization,
                        );
                    }
                    Err(error) => {
                        super::debug_log::log(format!(
                            "failed to rotate Windows system-audio capture: [{}] {}",
                            error.code, error.message
                        ));
                        if let Some(dir) = screen_segment_dir.as_deref() {
                            cleanup_failed_segment_dirs(dir, None, None);
                        }
                        stop_active_sessions_after_failure(&mut self.runtime);
                        mark_runtime_session_failed(&mut self.runtime);
                        return TickOutcome::StopLoop;
                    }
                }
                set_current_system_audio_output_file(
                    &mut next_segment_outputs,
                    system_audio_output_file.clone(),
                );
                next_system_audio_recording_file = Some(system_audio_output_file);
            }
        }

        if let Some(tx) = self.runtime.frame_artifact_tx.as_ref() {
            super::segments::flush_frame_artifacts(tx);
        }

        // --- Finalize + commit the segment that just closed ---
        if let Err(error) = finalize_capture_outputs(
            previous_segment_output_files.as_mut(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        ) {
            super::debug_log::log(format!(
                "Windows capture output finalization reported an issue while rotating: [{}] {}",
                error.code, error.message
            ));
        }

        if let (Some(committed), Some(segment)) = (
            self.runtime.output_files.as_mut(),
            previous_segment_output_files.as_ref(),
        ) {
            append_committed_segment_output_files(committed, segment);
        }
        // Commit Audio Segments without enqueuing audio processing jobs on Windows.
        super::segments::persist_committed_audio_segments(
            Some(app_handle),
            self.runtime.source_sessions.as_ref(),
            self.runtime.segment_schedule.as_ref(),
            self.runtime.current_segment_index,
            previous_segment_output_files.as_ref(),
            &rotation_known_durations,
        );
        // Enqueue scrub-preview generation for the rotated-out screen segment;
        // its SFI1 frame-index sidecar was finalized above so it is now
        // scrub-eligible (issue #83). The shared path no-ops otherwise.
        super::segments::warm_scrub_previews_for_committed_screen_outputs(
            Some(app_handle),
            previous_segment_output_files.as_ref(),
        );

        self.runtime.current_segment_index = next_index;
        self.runtime.current_segment_output_files = Some(next_segment_outputs);
        self.runtime.current_segment_sources = Some(active_sources);
        self.runtime.recording_file = next_recording_file;
        self.runtime.microphone_recording_file = next_microphone_recording_file;
        self.runtime.system_audio_recording_file = next_system_audio_recording_file;
        if let Err(error) = reanchor_active_segment_timing(&mut self.runtime, "rotating segments") {
            super::debug_log::log(format!(
                "failed to re-anchor Windows capture segment timing while rotating: [{}] {}",
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

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use capture_runtime::RuntimeState;
    use capture_screen::{RotatedCaptureOutputs, ScreenCaptureSession};
    use capture_types::{
        CaptureErrorResponse, CaptureOutputFiles, SourceSessionMeta, SourceSessions,
    };
    use std::path::Path;

    #[derive(Debug)]
    struct FakeScreenCaptureSession {
        live: bool,
        pending_stop_error: Option<CaptureErrorResponse>,
    }

    impl ScreenCaptureSession for FakeScreenCaptureSession {
        fn rotate(
            &mut self,
            _segment_dir: &Path,
            _screen_output_file: Option<&Path>,
            _system_audio_output_path: Option<&Path>,
        ) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
            Ok(RotatedCaptureOutputs {
                recording_file: "fake.mp4".to_string(),
                system_audio_recording_file: None,
                output_files: CaptureOutputFiles {
                    screen_file: None,
                    screen_files: Vec::new(),
                    microphone_file: None,
                    microphone_files: Vec::new(),
                    system_audio_file: None,
                    system_audio_files: Vec::new(),
                },
            })
        }

        fn stop(&mut self, _inactivity_tail_trim_seconds: u64) -> Result<(), CaptureErrorResponse> {
            self.live = false;
            Ok(())
        }

        fn is_live(&self) -> bool {
            self.live
        }

        fn take_stop_error(&mut self) -> Option<CaptureErrorResponse> {
            self.pending_stop_error.take()
        }

        fn supports_frame_export(&self) -> bool {
            false
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn closed_error() -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: capture_screen::SCREEN_CAPTURE_ITEM_CLOSED_ERROR_CODE.to_string(),
            message: "fake monitor disconnect".to_string(),
        }
    }

    fn genuine_stop_error() -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: "screen_capture_encoder_failed".to_string(),
            message: "fake non-transient encoder failure".to_string(),
        }
    }

    fn lifecycle_with_screen_session(session: FakeScreenCaptureSession) -> RecordingLifecycle {
        let mut lifecycle = RecordingLifecycle::default();
        lifecycle.runtime.is_running = true;
        lifecycle.runtime.runtime_state = RuntimeState::Running;
        lifecycle.runtime.active_screen_session = Some(Box::new(session));
        lifecycle.runtime.requested_sources = Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        });
        lifecycle
    }

    #[test]
    fn windows_transient_stop_error_suspends_screen_without_failing() {
        // ADR 0023: a `GraphicsCaptureItem.Closed` stop error is transient — the
        // session must survive with the screen suspended for transient liveness,
        // not be failed.
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });

        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert!(lifecycle.runtime.is_running);
        assert_eq!(lifecycle.runtime.runtime_state, RuntimeState::Running);
        assert!(lifecycle.runtime.active_screen_session.is_none());
        assert!(lifecycle.runtime.inactivity.is_screen_paused());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(
                super::super::inactivity::ScreenPauseReason::TransientLiveness {
                    trigger: super::super::inactivity::TransientLivenessTrigger::DisplayUnavailable,
                }
            )
        );
    }

    #[test]
    fn windows_dead_screen_session_without_error_suspends_screen_without_failing() {
        // ADR 0023: not-live without a backend stop error is the same
        // display-death family → transient suspension, not failure.
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: None,
        });

        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert!(lifecycle.runtime.is_running);
        assert_eq!(lifecycle.runtime.runtime_state, RuntimeState::Running);
        assert!(lifecycle.runtime.active_screen_session.is_none());
        assert!(lifecycle.runtime.inactivity.is_screen_paused());
        assert!(matches!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(super::super::inactivity::ScreenPauseReason::TransientLiveness { .. })
        ));
    }

    #[test]
    fn windows_genuine_stop_error_still_marks_runtime_failed() {
        // A non-transient stop error keeps the existing fail-the-session path.
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(genuine_stop_error()),
        });

        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::StopLoop
        );
        assert!(!lifecycle.runtime.is_running);
        assert_eq!(lifecycle.runtime.runtime_state, RuntimeState::Failed);
        assert!(lifecycle.runtime.active_screen_session.is_none());
    }

    use super::super::inactivity::{InactivityState, ScreenPauseReason, TransientLivenessTrigger};

    fn transient_pause_reason() -> ScreenPauseReason {
        ScreenPauseReason::TransientLiveness {
            trigger: TransientLivenessTrigger::DisplayUnavailable,
        }
    }

    fn session_lock_pause_reason() -> ScreenPauseReason {
        ScreenPauseReason::TransientLiveness {
            trigger: TransientLivenessTrigger::SessionLock,
        }
    }

    fn display_asleep_pause_reason() -> ScreenPauseReason {
        ScreenPauseReason::TransientLiveness {
            trigger: TransientLivenessTrigger::DisplayAsleep,
        }
    }

    fn screen_only_inactivity_state() -> InactivityState {
        InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            last_activity_monotonic_ms: 0,
            ..InactivityState::default()
        }
    }

    #[test]
    fn windows_session_lock_records_session_lock_reason() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });

        let session = lifecycle
            .handle_windows_session_lock()
            .expect("session lock should pause requested screen capture");

        assert!(session.is_running);
        assert!(session.is_inactivity_paused);
        assert!(lifecycle.runtime.active_screen_session.is_none());
        assert!(lifecycle.runtime.inactivity.is_screen_paused());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(session_lock_pause_reason())
        );
        assert!(lifecycle.windows_session_unlock_can_resume_screen());
    }

    #[test]
    fn windows_session_lock_noops_when_recording_cannot_accept_screen_pause() {
        let mut not_running = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        not_running.runtime.is_running = false;
        assert!(not_running.handle_windows_session_lock().is_none());
        assert!(!not_running.runtime.inactivity.is_screen_paused());

        let mut screen_not_requested = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        screen_not_requested.runtime.requested_sources = Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        });
        assert!(screen_not_requested.handle_windows_session_lock().is_none());
        assert!(!screen_not_requested.runtime.inactivity.is_screen_paused());

        let mut user_paused = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        user_paused.runtime.user_capture_paused = true;
        assert!(user_paused.handle_windows_session_lock().is_none());
        assert!(!user_paused.runtime.inactivity.is_screen_paused());
    }

    #[test]
    fn windows_session_lock_preserves_audio_pause_flags_and_source_sessions() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        lifecycle.runtime.requested_sources = Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        });
        lifecycle.runtime.current_segment_sources = Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        });
        let source_sessions = SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "screen-session".to_string(),
                started_at_unix_ms: 1,
            }),
            microphone: Some(SourceSessionMeta {
                session_id: "microphone-session".to_string(),
                started_at_unix_ms: 2,
            }),
            system_audio: Some(SourceSessionMeta {
                session_id: "system-audio-session".to_string(),
                started_at_unix_ms: 3,
            }),
        };
        lifecycle.runtime.source_sessions = Some(source_sessions.clone());

        let session = lifecycle
            .handle_windows_session_lock()
            .expect("session lock should pause only the screen family");

        assert!(lifecycle.runtime.inactivity.is_screen_paused());
        assert!(!lifecycle.runtime.inactivity.is_microphone_paused());
        assert!(!lifecycle.runtime.inactivity.is_system_audio_paused());
        assert_eq!(
            lifecycle.runtime.source_sessions,
            Some(source_sessions.clone())
        );
        assert_eq!(session.source_sessions, Some(source_sessions));
        assert_eq!(
            lifecycle.runtime.current_segment_sources,
            Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: true,
            })
        );
    }

    #[test]
    fn windows_session_unlock_gate_rejects_display_unavailable_and_inactivity() {
        let mut display_unavailable = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });
        assert_eq!(
            display_unavailable.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert_eq!(
            display_unavailable.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason())
        );
        assert!(!display_unavailable.windows_session_unlock_can_resume_screen());
        assert_eq!(
            display_unavailable.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason())
        );

        let mut inactivity = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        super::super::segments::pause_screen_for_inactivity_with_app_handle(
            &mut inactivity.runtime,
            None,
        )
        .expect("inactivity screen pause should succeed");
        assert_eq!(
            inactivity.runtime.inactivity.screen_pause_reason(),
            Some(ScreenPauseReason::Inactivity)
        );
        assert!(!inactivity.windows_session_unlock_can_resume_screen());
        assert_eq!(
            inactivity.runtime.inactivity.screen_pause_reason(),
            Some(ScreenPauseReason::Inactivity)
        );
    }

    #[test]
    fn windows_repeated_session_lock_preserves_existing_pause_reason() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });
        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason())
        );

        assert!(lifecycle.handle_windows_session_lock().is_none());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason())
        );
    }

    // Slice 8: a console display-off (DPMS) pauses the screen as a transient-liveness
    // suspension recording `TransientLiveness { DisplayAsleep }`, and the resume gate
    // accepts it (display-on can resume an unlocked, unsuspended DisplayAsleep pause).
    #[test]
    fn windows_display_off_pauses_screen_for_display_asleep() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });

        let session = lifecycle
            .handle_windows_display_asleep()
            .expect("display-off should pause requested screen capture");

        assert!(session.is_running);
        assert!(session.is_inactivity_paused);
        assert!(lifecycle.runtime.active_screen_session.is_none());
        assert!(lifecycle.runtime.inactivity.is_screen_paused());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(display_asleep_pause_reason())
        );
        assert!(lifecycle.windows_display_awake_can_resume_screen());
    }

    // Slice 8 guarded resume — sleep-then-lock ordering (the hazard). Display sleeps
    // first (DisplayAsleep pause), then the workstation locks while already paused, so
    // the single reason stays DisplayAsleep but the out-of-band lock signal is set. A
    // display-on must NOT resume capture while the session is still locked.
    #[test]
    fn windows_display_asleep_then_session_lock_blocks_display_on_resume() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });

        lifecycle
            .handle_windows_display_asleep()
            .expect("display-off should pause for DisplayAsleep");
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(display_asleep_pause_reason())
        );
        assert!(
            lifecycle.windows_display_awake_can_resume_screen(),
            "before the lock, an unlocked DisplayAsleep pause is resumable"
        );

        // The workstation locks. The screen is already paused, so the lock pause
        // no-ops on the single reason — but the lock signal is still recorded.
        assert!(lifecycle.handle_windows_session_lock().is_none());
        assert!(lifecycle.session_locked);
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(display_asleep_pause_reason()),
            "the single-reason model keeps DisplayAsleep; the lock is tracked out-of-band"
        );

        // A display-on while locked must NOT resume: capture stays paused.
        assert!(
            !lifecycle.windows_display_awake_can_resume_screen(),
            "display-on must not resume capture while the session is locked (sleep-then-lock)"
        );
        assert!(lifecycle.runtime.inactivity.is_screen_paused());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(display_asleep_pause_reason())
        );
    }

    // Slice 8 guarded resume — lock-then-sleep ordering. The workstation locks first
    // (SessionLock pause), then the display sleeps while already paused, so the
    // DisplayAsleep pause no-ops and must not downgrade the SessionLock reason. A
    // display-on must NOT spuriously resume: the reason is SessionLock (not
    // DisplayAsleep) and the session is still locked. SessionLock resumes exclusively
    // via WTS unlock.
    #[test]
    fn windows_session_lock_then_display_asleep_blocks_display_on_resume() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });

        lifecycle
            .handle_windows_session_lock()
            .expect("session lock should pause requested screen capture");
        assert!(lifecycle.session_locked);
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(session_lock_pause_reason())
        );

        // The display then sleeps while the screen is already paused for SessionLock.
        assert!(lifecycle.handle_windows_display_asleep().is_none());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(session_lock_pause_reason()),
            "DisplayAsleep must not overwrite an existing SessionLock pause reason"
        );

        // A display-on must NOT resume a SessionLock pause.
        assert!(
            !lifecycle.windows_display_awake_can_resume_screen(),
            "display-on must not resume a SessionLock pause (lock-then-sleep)"
        );
        assert!(lifecycle.runtime.inactivity.is_screen_paused());
    }

    // Slice 8: the guarded-resume predicate is gated on lock AND suspend state, both
    // read from existing in-process signals (never a fresh query). Clearing the lock
    // (as a WTS unlock does) restores resumability while the reason is still
    // DisplayAsleep; a concurrent system-suspend independently blocks it.
    #[test]
    fn windows_display_awake_predicate_requires_unlocked_and_unsuspended() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        lifecycle
            .handle_windows_display_asleep()
            .expect("display-off should pause for DisplayAsleep");
        assert!(lifecycle.windows_display_awake_can_resume_screen());

        // Locked blocks the resume; clearing the lock restores it (reason unchanged).
        lifecycle.session_locked = true;
        assert!(!lifecycle.windows_display_awake_can_resume_screen());
        lifecycle.session_locked = false;
        assert!(lifecycle.windows_display_awake_can_resume_screen());

        // A concurrent system-suspend pause independently blocks the resume.
        lifecycle.runtime.inactivity.system_suspend_paused_sources = Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        });
        assert!(lifecycle.runtime.inactivity.is_system_suspend_paused());
        assert!(
            !lifecycle.windows_display_awake_can_resume_screen(),
            "display-on must not resume while system-suspend paused"
        );
    }

    // Acceptance criterion 1 (lifecycle-level): the suspend path records
    // `TransientLiveness { DisplayUnavailable }`, and a *later* inactivity pause on
    // a fresh session records `Inactivity` — the discriminator distinguishes the
    // two reasons through the real pause machinery, not just the unit setters.
    #[test]
    fn windows_transient_suspend_then_inactivity_pause_record_distinct_reasons() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });

        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason()),
            "the transient suspend path must record TransientLiveness {{ DisplayUnavailable }}"
        );

        // Clear the pause as a resume would, then drive a genuine inactivity pause
        // through the same shared screen-pause primitive and confirm the reason
        // discriminator now reads `Inactivity`.
        lifecycle
            .runtime
            .inactivity
            .set_family_paused_states(false, false, false);
        assert_eq!(lifecycle.runtime.inactivity.screen_pause_reason(), None);

        lifecycle.runtime.active_screen_session = Some(Box::new(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        }));
        super::super::segments::pause_screen_for_inactivity_with_app_handle(
            &mut lifecycle.runtime,
            None,
        )
        .expect("inactivity screen pause should succeed for a live session");
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(ScreenPauseReason::Inactivity),
            "a subsequent inactivity pause must record the Inactivity reason"
        );
    }

    // Acceptance criterion 2(b) at tick level: a display-present condition must not
    // resume an `Inactivity`-paused screen, and no transient probe should even be
    // considered for it. The transient resume gate in `tick_inactivity`
    // (`screen_pause_reason()` matches `TransientLiveness`) and the pure
    // `should_resume_screen_from_transient_liveness` predicate both reject an
    // inactivity pause regardless of display presence.
    #[test]
    fn windows_inactivity_paused_screen_is_not_transient_resumed_by_present_display() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: true,
            pending_stop_error: None,
        });
        super::super::segments::pause_screen_for_inactivity_with_app_handle(
            &mut lifecycle.runtime,
            None,
        )
        .expect("inactivity screen pause should succeed");
        assert!(lifecycle.runtime.inactivity.is_screen_paused());

        // The lifecycle guard the tick uses to decide whether to run the transient
        // probe at all rejects an inactivity pause.
        assert!(
            !lifecycle.screen_paused_for_transient_liveness(),
            "an inactivity pause must not be seen as a transient-liveness pause"
        );
        // And the pure predicate refuses to resume it even with a present display.
        assert!(
            !lifecycle
                .runtime
                .inactivity
                .should_resume_screen_from_transient_liveness(true, 1_000_000),
            "a display-present probe must never resume an inactivity-paused screen"
        );
    }

    // Acceptance criterion 2(a) at lifecycle level: while the screen is paused for
    // transient liveness, fresh user activity must NOT resume it (the activity
    // resume path is gated by `screen_paused_for_transient_liveness`), even though
    // the inactivity resume predicate would otherwise fire on the active snapshot.
    #[test]
    fn windows_transient_paused_screen_is_not_resumed_by_user_activity() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });
        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert!(lifecycle.screen_paused_for_transient_liveness());

        // A fully-active snapshot well past the resume guard window.
        let active_snapshot = super::super::inactivity::ActivitySnapshot {
            system_input_idle_ms: Some(0),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(0),
            microphone_activity: super::super::inactivity::AudioActivitySourceState::default(),
            system_audio_activity: super::super::inactivity::AudioActivitySourceState::default(),
        };
        assert!(
            !lifecycle
                .runtime
                .inactivity
                .should_resume_screen_from_inactivity(1_000_000, active_snapshot),
            "user activity must not resume a transient-liveness screen pause"
        );
    }

    // Acceptance criterion 3 (status reflection): a transient suspension keeps the
    // session running and surfaces a paused screen family via `is_screen_paused()`
    // / the per-family flags the status surface reads.
    #[test]
    fn windows_transient_suspension_reflects_paused_status_with_session_running() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: None,
        });

        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert!(lifecycle.runtime.is_running, "session must keep running");
        assert_eq!(lifecycle.runtime.runtime_state, RuntimeState::Running);
        assert!(
            lifecycle.runtime.inactivity.is_screen_paused(),
            "screen family must read as paused"
        );
        assert!(
            lifecycle.runtime.inactivity.is_paused,
            "is_paused-style status must reflect the screen pause"
        );
    }

    // Acceptance criterion 4 (throttle, lifecycle-level): consecutive ticks inside
    // the 2s recovery interval perform at most one probe — modeled by asserting
    // `last_transient_liveness_probe_monotonic_ms` only advances when a probe is
    // due. `tick_inactivity` itself needs a real `tauri::AppHandle` (and calls the
    // real `windows_display_present()` Win32 API), so we exercise the throttle
    // bookkeeping the tick relies on directly.
    #[test]
    fn windows_transient_probe_throttle_advances_marker_only_when_due() {
        let mut state = screen_only_inactivity_state();
        state.set_family_paused_states_with_reason(true, false, false, transient_pause_reason());

        // First probe is always due; mark it.
        assert!(state.is_transient_liveness_probe_due(10_000));
        state.mark_transient_liveness_probe(10_000);
        assert_eq!(
            state.last_transient_liveness_probe_monotonic_ms,
            Some(10_000)
        );

        // A tick 1s later (inside the 2s interval) is not due; the tick's gate
        // (`is_transient_liveness_probe_due`) returns false so no probe runs and the
        // marker does not advance.
        assert!(!state.is_transient_liveness_probe_due(11_000));

        // A tick at +2s is due; mark advances.
        assert!(state.is_transient_liveness_probe_due(12_000));
        state.mark_transient_liveness_probe(12_000);
        assert_eq!(
            state.last_transient_liveness_probe_monotonic_ms,
            Some(12_000)
        );
    }

    // Acceptance criterion 5 (partial): with screen as the only requested source, a
    // transient suspension leaves every requested family paused, which means the
    // activity-driven resume-all branch is gated off (it requires
    // `!screen_paused_for_transient_liveness`) — the loop cannot churn the screen
    // back on via activity, and rotation has nothing live to rotate.
    #[test]
    fn windows_screen_only_transient_suspension_pauses_all_requested_families() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });
        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );

        assert!(
            lifecycle.runtime.is_running,
            "session must survive screen-only loss"
        );
        assert!(
            lifecycle.all_requested_families_paused_for_inactivity(),
            "screen-only request with the screen paused means all requested families are paused"
        );
        assert!(
            lifecycle.screen_paused_for_transient_liveness(),
            "the activity resume-all branch must stay gated off while transient-paused"
        );
    }

    // Finding 1 (BLOCKER) regression at the segments-path level: with the screen
    // transient-paused, driving the real Windows microphone inactivity pause path
    // must preserve the screen's `TransientLiveness { DisplayUnavailable }` reason
    // and its pause-start timestamp (it now routes through the audio-only family
    // setter instead of `set_family_paused_states`, which would have clobbered both).
    #[test]
    fn windows_microphone_inactivity_pause_preserves_transient_screen_reason() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });
        lifecycle.runtime.requested_sources = Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        });

        // Enter the transient-liveness screen suspension via the real path.
        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason())
        );
        let pause_started_at = lifecycle.runtime.inactivity.screen_paused_at_monotonic_ms;
        assert!(pause_started_at.is_some());

        // Microphone crosses the inactivity threshold. No live microphone session is
        // attached (the `if let Some` guard skips it) and finalize tolerates errors,
        // so this exercises the family-state bookkeeping the regression is about.
        super::super::segments::pause_microphone_for_inactivity_with_app_handle(
            &mut lifecycle.runtime,
            None,
        )
        .expect("microphone inactivity pause should succeed");

        assert!(lifecycle.runtime.inactivity.is_microphone_paused());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason()),
            "microphone inactivity pause must not clobber the transient screen reason"
        );
        assert_eq!(
            lifecycle.runtime.inactivity.screen_paused_at_monotonic_ms, pause_started_at,
            "microphone inactivity pause must not reset the screen pause-start timestamp"
        );
        assert!(lifecycle.screen_paused_for_transient_liveness());

        // A microphone resume (state side, as `resume_microphone_from_inactivity`
        // performs after restarting the mic segment) must likewise leave the screen
        // reason and timestamp intact.
        lifecycle
            .runtime
            .inactivity
            .set_audio_family_paused_states(false, false);
        assert!(!lifecycle.runtime.inactivity.is_microphone_paused());
        assert_eq!(
            lifecycle.runtime.inactivity.screen_pause_reason(),
            Some(transient_pause_reason()),
            "microphone resume must not clear the transient screen reason"
        );
        assert_eq!(
            lifecycle.runtime.inactivity.screen_paused_at_monotonic_ms, pause_started_at,
            "microphone resume must not reset the screen pause-start timestamp"
        );
    }

    // Finding 2 (BLOCKER) regression: drive the REAL gate→mark→predicate ordering
    // the tick uses (`transient_liveness_resume_decision`, which the tick calls with
    // the live `windows_display_present` probe) with a stubbed display value. The
    // decision must be true when a display is present, false when absent, and the
    // second tick within the 2s throttle window must not probe again (so a present
    // display on a throttled tick yields no resume).
    #[test]
    fn windows_transient_resume_decision_fires_when_display_present_and_throttles() {
        let mut lifecycle = lifecycle_with_screen_session(FakeScreenCaptureSession {
            live: false,
            pending_stop_error: Some(closed_error()),
        });
        assert_eq!(
            lifecycle.handle_windows_screen_capture_stop(),
            TickOutcome::Continue
        );
        assert!(lifecycle.screen_paused_for_transient_liveness());

        // No display present at the first due probe: no resume, but the probe ran.
        let mut probes = 0u32;
        assert!(
            !lifecycle.transient_liveness_resume_decision(10_000, || {
                probes += 1;
                false
            }),
            "absent display must not resume"
        );
        assert_eq!(probes, 1, "first probe is due and runs");

        // A display IS present and a probe is due: the decision fires. This is the
        // case the self-poisoning throttle bug made permanently false.
        assert!(
            lifecycle.transient_liveness_resume_decision(12_000, || {
                probes += 1;
                true
            }),
            "present display on a due probe must resume"
        );
        assert_eq!(probes, 2, "second probe at +2s is due and runs");

        // A tick 1s later is inside the 2s throttle window: the probe must NOT run
        // again and the decision must be false even though a display is present.
        assert!(
            !lifecycle.transient_liveness_resume_decision(13_000, || {
                probes += 1;
                true
            }),
            "a throttled tick must not resume"
        );
        assert_eq!(
            probes, 2,
            "the throttled tick must not invoke the display probe again"
        );
    }
}
