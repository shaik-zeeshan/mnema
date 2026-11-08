//! The Recording Lifecycle: the one owning seam for start, stop, pause/resume,
//! wake/sleep and the segment loop's tick. Host handlers are thin adapters over
//! it, reaching it through [`NativeCaptureState`].

mod rotation;
mod tick;

use crate::native_capture::activity::current_activity_snapshot;
use crate::native_capture::frame_artifacts::flush_frame_artifacts;
#[cfg(target_os = "macos")]
use crate::native_capture::microphone_away::{
    mac_is_away, should_resume_microphone_after_activity,
};
#[cfg(target_os = "macos")]
use crate::native_capture::microphone_away::{reopen_unwakeable_microphone, tick_microphone_away};
use crate::native_capture::runtime::{
    active_sources_for_inactivity_paused_state, apply_runtime_signal,
    current_segment_sources_for_runtime, ensure_system_audio_planner_for_runtime,
    mark_runtime_session_failed, microphone_planner_for_runtime, refresh_runtime_planner_dates,
    screen_planner_for_runtime, system_audio_planner_for_runtime,
    system_audio_stops_with_suspension, CaptureSuspensionKind,
};
use crate::native_capture::runtime::{
    mark_runtime_session_stopped, request_segment_loop_stop, session_from_runtime,
    stopped_session_from_runtime, NativeCaptureRuntime,
};
use crate::native_capture::segments::{
    apply_microphone_output_finalization, cleanup_failed_segment_dirs, create_segment_output_dirs,
    current_segment_output_files_mut, empty_output_files, handle_inactivity_resume_error,
    pause_microphone_for_inactivity_with_app_handle, pause_runtime_for_inactivity_with_app_handle,
    pause_screen_for_inactivity_with_app_handle, pause_system_audio_for_inactivity_with_app_handle,
    plan_live_rotation_segment, reanchor_active_segment_timing,
    recover_from_segment_finalize_error, recover_screen_capture_after_wake,
    resume_microphone_from_inactivity, resume_runtime_from_inactivity,
    resume_screen_from_inactivity, resume_system_audio_from_inactivity, start_capture_runtime,
    start_segment_with_current_privacy_filter, start_system_audio_family_for_runtime,
    stop_active_sessions_after_failure, stop_capture_runtime,
};
use crate::native_capture_output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
use crate::AppCore;
use capture_runtime::RuntimeSignal;
use capture_types::{
    CaptureErrorResponse, CaptureSources, NativeCaptureSession, RecordingSettings,
};
use capture_vad::MicrophoneVadFallbackNotice;
use std::sync::{Arc, Mutex};

/// The capture lock: one lifecycle, shared by app-core and the host (both manage
/// the same `Arc`).
pub type NativeCaptureState = Arc<Mutex<RecordingLifecycle>>;

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

/// The sources a user resume reopens, and whether the microphone stays down
/// under the away hold. Lock is away: a resume while the Mac is away leaves the
/// microphone held until the tick sees the Mac back. A microphone voice
/// enrollment holds stays down too; the enrollment's restore brings it back.
#[cfg(target_os = "macos")]
pub(super) fn user_resume_sources(
    runtime: &NativeCaptureRuntime,
    sources: CaptureSources,
) -> (CaptureSources, bool) {
    let microphone_held_away = sources.microphone && mac_is_away(runtime);
    let sources = CaptureSources {
        microphone: sources.microphone && !microphone_held_away && !runtime.microphone_lent_out,
        ..sources
    };
    (sources, microphone_held_away)
}

#[derive(Debug, Default)]
pub struct RecordingLifecycle {
    runtime: NativeCaptureRuntime,
    /// Monotonic deadline for the next tap-start retry, and the delay that set
    /// it. Kept here rather than on the runtime because it is the tick's pacing,
    /// not capture state; [`RecordingLifecycle::start`] resets it per session.
    system_audio_start_retry_at_ms: Option<u64>,
    system_audio_start_retry_delay_ms: u64,
    /// Per-source "already warned" flags for inactivity resumes the tick
    /// retries every second; see [`crate::debug_log::streak_level`].
    resume_failure_logged: ResumeFailureLogged,
}

#[derive(Debug, Default)]
struct ResumeFailureLogged {
    microphone: bool,
    system_audio: bool,
    screen: bool,
    all: bool,
}

#[derive(Debug, Clone)]
pub enum StartRecordingLifecycleOutcome {
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
        core: Option<&Arc<AppCore>>,
        error: &CaptureErrorResponse,
    ) -> TickOutcome {
        if let Err(stop_error) = crate::native_capture::segments::suspend_screen_capture(
            core,
            &mut self.runtime,
            error,
            CaptureSuspensionKind::DisplayUnavailable,
        ) {
            // Suspend only errors when the stop must preserve runtime state
            // (an in-flight stop timeout); keep the session and let a later
            // tick or the wake/display recovery paths reconcile it.
            crate::debug_log::log_warn(format!(
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
        core: Option<&Arc<AppCore>>,
        block: Option<crate::licensing::LicenseBlock>,
    ) -> Option<TickOutcome> {
        let block = block?;
        crate::native_capture::segments::graceful_stop_for_license_block(
            core,
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
        core: Option<&Arc<AppCore>>,
        active_sources: &CaptureSources,
    ) -> Option<TickOutcome> {
        if !active_sources.screen || self.runtime.active_screen_session.is_some() {
            return None;
        }

        crate::debug_log::log_warn(
            "segment rotation due but the screen capture session is missing; suspending screen until capture can restart",
        );
        Some(self.suspend_screen_after_unexpected_stop(
            core,
            &CaptureErrorResponse {
                code: "capture_screen_session_missing".to_string(),
                message: "Screen capture session missing at segment rotation".to_string(),
            },
        ))
    }

    pub fn session(&self) -> NativeCaptureSession {
        session_from_runtime(&self.runtime)
    }

    pub fn start(
        &mut self,
        core: Arc<AppCore>,
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
            crate::licensing::cached_status(&core).as_ref(),
            gate_now_ms,
        );
        if let Some(refusal) = Self::refuse_start_for_license_block(block) {
            // Recompute async so the cache/tray/Settings flip from the stale
            // `Trial` to `ReadOnly`; the refusal itself doesn't wait for it.
            crate::licensing::recompute_status_async(&core, gate_now_ms);
            crate::debug_log::log_info(format!("capture refused: {}", refusal.code));
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
        self.resume_failure_logged = ResumeFailureLogged::default();

        start_capture_runtime(
            &mut self.runtime,
            core,
            settings,
            sources,
            microphone_device_id_for_capture,
        )?;

        Ok(StartRecordingLifecycleOutcome::Started(self.session()))
    }

    pub fn stop(
        &mut self,
        core: &Arc<AppCore>,
    ) -> Result<NativeCaptureSession, CaptureErrorResponse> {
        if let Err(error) = stop_capture_runtime(&mut self.runtime, Some(core)) {
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

    pub fn pause_user_capture(
        &mut self,
        core: &Arc<AppCore>,
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
        stop_capture_runtime(&mut self.runtime, Some(core))?;
        self.runtime.user_capture_paused = true;
        self.runtime.current_segment_sources = Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        });
        Ok(self.session())
    }

    pub fn resume_user_capture(
        &mut self,
        core: &Arc<AppCore>,
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
            let (sources, microphone_held_away) = user_resume_sources(&self.runtime, sources);
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
                core,
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
            self.runtime.microphone_away_hold = microphone_held_away;
            self.runtime.current_segment_index = next_index;

            // A user pause stopped the tap with everything else, so a user resume
            // starts it again — including for a session with no screen at all.
            match crate::native_capture::segments::start_system_audio_family_for_runtime(
                &mut self.runtime,
                crate::native_capture::privacy::collect_initial_privacy_filter(core)
                    .excluded_bundle_ids(),
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
                    crate::debug_log::log_warn(format!(
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

    /// Hand the microphone to a bounded out-of-band recording (voice enrollment)
    /// and report whether it was actually taken from a live session, so the
    /// caller knows to hand it back.
    ///
    /// Microphone family only, deliberately: screen and the system-audio tap
    /// neither share the device nor say anything about who is speaking, so
    /// nothing here touches them. It lives on this seam because enrollment must
    /// be runnable anywhere — onboarding orders capture after it, but Settings
    /// re-enroll can run mid-session.
    ///
    /// A stop rather than a soft pause: `pause_output_file` leaves the
    /// AVCaptureSession running, and `capture-microphone`'s activity/VAD feeds are
    /// process-global, so a second concurrent session would corrupt the live
    /// session's VAD boundary trimming.
    #[cfg(target_os = "macos")]
    pub fn release_microphone_for_out_of_band_recording(
        &mut self,
    ) -> Result<bool, CaptureErrorResponse> {
        if self.runtime.active_microphone_session.is_none() {
            return Ok(false);
        }
        crate::native_capture::segments::stop_microphone_session_for_runtime(&mut self.runtime)?;
        self.runtime.active_microphone_session = None;
        self.runtime.microphone_lent_out = true;
        Ok(true)
    }

    /// Give the microphone back after a bounded out-of-band recording. A no-op
    /// unless the runtime still wants it: anything that legitimately took
    /// ownership meanwhile — a device-change reconnect, a user pause, a low-disk
    /// suspension, or a stop — wins.
    ///
    /// A `LowDisk` suspension is the one suspension kind that stops the
    /// microphone on purpose: every source writes to the same full volume
    /// (ADR 0040), and `suspend_screen_capture` already committed the mic file
    /// and nulled `microphone_recording_file`. It restarts the microphone itself
    /// once space returns, so restoring here would both write onto a full volume
    /// and attach a live output file to an already-committed segment. A
    /// `DisplayUnavailable` suspension is deliberately *not* in this list — the
    /// microphone keeps recording through it (ADR 0021, amended).
    ///
    /// An inactivity pause is deliberately NOT a reason to bail either: it is a
    /// SOFT pause that leaves the session listening, because microphone activity
    /// is the only source the microphone family's resume decision reads. The
    /// bounded recording hard-stopped that session, so a pause left standing here
    /// could never end — no session, no samples, no wake-up.
    #[cfg(target_os = "macos")]
    pub fn restore_microphone_after_out_of_band_recording(
        &mut self,
    ) -> Result<(), CaptureErrorResponse> {
        self.runtime.microphone_lent_out = false;
        if !self.runtime.is_running
            || self.runtime.user_capture_paused
            || self
                .runtime
                .capture_suspension
                .as_ref()
                .is_some_and(|suspension| suspension.kind == CaptureSuspensionKind::LowDisk)
            || self.runtime.active_microphone_session.is_some()
            || !self
                .runtime
                .requested_sources
                .as_ref()
                .is_some_and(|sources| sources.microphone)
        {
            return Ok(());
        }
        // Lock is away: rather than record until the next tick holds it off,
        // stay down under the away hold, which the tick lifts on return and then
        // reopens the microphone.
        if mac_is_away(&self.runtime) {
            self.runtime.microphone_away_hold = true;
            return Ok(());
        }

        let device_id = self.runtime.microphone_device_id_for_capture.clone();
        crate::native_capture::microphone::restart_microphone_session_for_runtime(
            &mut self.runtime,
            device_id.as_deref(),
            "restoring microphone after a bounded enrollment recording",
        )?;

        // Clear the soft inactivity pause only once the restart has actually
        // produced a session: the next inactivity tick re-pauses within a tick if
        // the user is still idle, whereas leaving it set strands the microphone
        // with no session to wake it. Order matters — the caller only logs a
        // failed restore, and `resume_microphone_from_inactivity` is the one path
        // that re-arms a requested-but-session-less microphone. It runs only
        // while the family is paused, so clearing the pause on the way *in* would
        // make a failed restore permanent for the rest of the session.
        if self.runtime.inactivity.is_microphone_paused() {
            self.runtime.inactivity.set_family_paused_states(
                self.runtime.inactivity.screen_paused,
                false,
                self.runtime.inactivity.system_audio_paused,
            );
        }

        Ok(())
    }

    pub fn recover_after_wake(
        &mut self,
        core: Option<&Arc<AppCore>>,
    ) -> Result<bool, CaptureErrorResponse> {
        recover_screen_capture_after_wake(&mut self.runtime, core)
    }

    #[cfg(target_os = "macos")]
    pub fn should_attempt_recovery_after_possible_wake(&self) -> bool {
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
    pub fn handle_system_will_sleep(&mut self) -> bool {
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
                crate::debug_log::log_warn(format!(
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

    pub fn runtime(&self) -> &NativeCaptureRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut NativeCaptureRuntime {
        &mut self.runtime
    }

    pub fn take_microphone_vad_fallback_notification(
        &mut self,
    ) -> Option<MicrophoneVadFallbackNotice> {
        self.runtime.microphone_vad.take_new_fallback_notification()
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
