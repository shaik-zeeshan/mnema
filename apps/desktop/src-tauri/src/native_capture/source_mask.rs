//! The per-source mid-session mask (slice 5): user intent to stop/start an
//! individual source inside a live session, routed through the existing
//! per-family paused-flag seam.
//!
//! Design, in one place:
//! - The mask lives as three flags on [`InactivityState`]
//!   (`user_masked_*`), next to the family paused flags it composes with. A
//!   source is live only when: requested by settings AND not inactivity-paused
//!   AND not user-masked AND not suspended.
//! - **Invariant: masked ⇒ family paused flag set.** Every raw-flag reader —
//!   segment rotation, wake recovery, the system-audio tap-start retry,
//!   suspension recovery, user-pause resume — already treats a paused family as
//!   down, so the mask inherits all of that for free. The mask flag itself only
//!   (a) gates the activity-driven resume decisions so activity can never
//!   resurrect a masked source, and (b) remembers the user's intent for the UI.
//! - The mask is a separate axis from suspensions: a masked source ignores
//!   suspension churn (a screen suspension simply dissolves once recovery sees
//!   the family paused), and masking never creates or clears a suspension.
//! - **Session-scoped:** every session reset rebuilds `InactivityState`, so the
//!   mask clears on stop. Settings remain the cross-session intent.
//! - Unmasking restores the source into the live session immediately (fresh
//!   segment file for that family, per the existing resume semantics); if the
//!   user is genuinely idle the inactivity policy re-pauses it on the next tick
//!   — the same stance the voice-enrollment mic restore takes, and the only
//!   stance that cannot strand a hard-stopped microphone (its resume decision
//!   reads only microphone activity, which a stopped session can never emit).
//!
//! Per-family stop/start mechanics:
//! - Screen: the inactivity pause/resume machinery as-is.
//! - Microphone: the inactivity pause (finalize + commit + bookkeeping), then a
//!   hard stop of the AVCapture session — the voice-enrollment release — so the
//!   OS mic indicator goes dark; unmask starts a fresh session.
//! - System audio: the inactivity pause — the planner hook returns no path, the
//!   tap and its zero-watchdog stay alive writing nothing (a deliberate stop of
//!   the *writer*, never a tap rebuild), exactly how an inactivity pause keeps
//!   the watchdog from fighting a paused tap (ADR 0052).

use super::lifecycle::RecordingLifecycle;
use capture_types::{CaptureErrorResponse, CaptureSources, NativeCaptureSession};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaskableSource {
    Screen,
    Microphone,
    SystemAudio,
}

impl MaskableSource {
    /// Wire spelling shared with the frontend `SourceKey` and the tray.
    pub(crate) fn parse(source: &str) -> Option<Self> {
        match source {
            "screen" => Some(Self::Screen),
            "microphone" => Some(Self::Microphone),
            "systemAudio" => Some(Self::SystemAudio),
            _ => None,
        }
    }
}

fn not_running_error() -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "capture_session_not_running".to_string(),
        message: "No native capture session is running".to_string(),
    }
}

impl RecordingLifecycle {
    /// Set or clear the user mask for one source of the live session.
    ///
    /// Masking never ends the session and never touches the other sources;
    /// masking the last live source is refused (a session needs one).
    pub(crate) fn set_source_masked(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
        source: MaskableSource,
        masked: bool,
    ) -> Result<NativeCaptureSession, CaptureErrorResponse> {
        let runtime = self.runtime_mut();
        if !runtime.is_running {
            return Err(not_running_error());
        }
        let requested = runtime
            .requested_sources
            .clone()
            .ok_or_else(not_running_error)?;

        let is_requested = match source {
            MaskableSource::Screen => requested.screen,
            MaskableSource::Microphone => requested.microphone,
            MaskableSource::SystemAudio => requested.system_audio,
        };
        if !is_requested {
            // A source that is not part of this session cannot join it
            // mid-flight; that is the settings' (next session's) business.
            return Err(CaptureErrorResponse {
                code: "capture_mask_source_not_requested".to_string(),
                message: "This source is not part of the current recording session".to_string(),
            });
        }

        let currently_masked = match source {
            MaskableSource::Screen => runtime.inactivity.user_masked_screen,
            MaskableSource::Microphone => runtime.inactivity.user_masked_microphone,
            MaskableSource::SystemAudio => runtime.inactivity.user_masked_system_audio,
        };
        if currently_masked == masked {
            return Ok(self.session());
        }

        if masked {
            // Refuse to mask the last live-intent source (requested && unmasked).
            let live = CaptureSources {
                screen: requested.screen && !runtime.inactivity.user_masked_screen,
                microphone: requested.microphone && !runtime.inactivity.user_masked_microphone,
                system_audio: requested.system_audio
                    && !runtime.inactivity.user_masked_system_audio,
            };
            let live_after = match source {
                MaskableSource::Screen => CaptureSources {
                    screen: false,
                    ..live
                },
                MaskableSource::Microphone => CaptureSources {
                    microphone: false,
                    ..live
                },
                MaskableSource::SystemAudio => CaptureSources {
                    system_audio: false,
                    ..live
                },
            };
            if !(live_after.screen || live_after.microphone || live_after.system_audio) {
                return Err(CaptureErrorResponse {
                    code: "capture_mask_last_source".to_string(),
                    message: "At least one source must stay live while recording".to_string(),
                });
            }
            self.mask_source(app_handle, source)?;
        } else {
            self.unmask_source(app_handle, source)?;
        }

        Ok(self.session())
    }

    fn mask_source(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
        source: MaskableSource,
    ) -> Result<(), CaptureErrorResponse> {
        let runtime = self.runtime_mut();
        // A legacy global pause holds every family down with no per-family
        // flags; promote it first so the per-family machinery (which the mask
        // rides) owns each family from here on.
        runtime.inactivity.promote_legacy_global_pause_to_family_flags();

        #[cfg(target_os = "macos")]
        {
            let user_paused = runtime.user_capture_paused;
            let low_disk = super::runtime::system_audio_stops_with_suspension(runtime);
            match source {
                MaskableSource::Screen => {
                    // Any screen suspension already owns the screen's physical
                    // state (tail committed, session down) — flags-only there.
                    if !user_paused && runtime.capture_suspension.is_none() {
                        super::segments::pause_screen_for_inactivity_with_app_handle(
                            runtime, app_handle,
                        )?;
                    }
                }
                MaskableSource::Microphone => {
                    // Low disk is the one suspension that already stopped the
                    // mic itself (ADR 0040); a screen suspension leaves the mic
                    // recording, so the pause still applies under one.
                    if !user_paused && !low_disk {
                        super::segments::pause_microphone_for_inactivity_with_app_handle(
                            runtime, app_handle,
                        )?;
                    }
                    // The voice-enrollment release: a hard stop, not a soft
                    // pause, so the OS microphone indicator goes dark. The
                    // inactivity pause above already finalized and committed
                    // the in-flight file, so stopping the session here only
                    // releases the device.
                    if let Some(session) = runtime.active_microphone_session.as_mut() {
                        session.stop()?;
                    }
                    runtime.active_microphone_session = None;
                }
                MaskableSource::SystemAudio => {
                    if !user_paused && !low_disk {
                        super::segments::pause_system_audio_for_inactivity_with_app_handle(
                            runtime, app_handle,
                        )?;
                    }
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = app_handle;

        // Set the mask flag and enforce the invariant masked ⇒ family paused
        // flag, whether the physical stop above ran (it set the flag) or was
        // skipped because a pause/suspension already had the family down.
        let runtime = self.runtime_mut();
        let inactivity = &mut runtime.inactivity;
        match source {
            MaskableSource::Screen => inactivity.user_masked_screen = true,
            MaskableSource::Microphone => inactivity.user_masked_microphone = true,
            MaskableSource::SystemAudio => inactivity.user_masked_system_audio = true,
        }
        inactivity.set_family_paused_states(
            inactivity.screen_paused || inactivity.user_masked_screen,
            inactivity.microphone_paused || inactivity.user_masked_microphone,
            inactivity.system_audio_paused || inactivity.user_masked_system_audio,
        );
        Ok(())
    }

    fn unmask_source(
        &mut self,
        app_handle: Option<&tauri::AppHandle>,
        source: MaskableSource,
    ) -> Result<(), CaptureErrorResponse> {
        let runtime = self.runtime_mut();
        // Clear the mask first: the resume paths below are gated on it.
        match source {
            MaskableSource::Screen => runtime.inactivity.user_masked_screen = false,
            MaskableSource::Microphone => runtime.inactivity.user_masked_microphone = false,
            MaskableSource::SystemAudio => runtime.inactivity.user_masked_system_audio = false,
        }

        let user_paused = runtime.user_capture_paused;
        #[cfg(target_os = "macos")]
        let physical_owner_holds_source = match source {
            // Any screen suspension owns the screen restart (its recovery
            // driver restarts the screen once it can).
            MaskableSource::Screen => user_paused || runtime.capture_suspension.is_some(),
            // Only low disk stops the audio families (ADR 0040); its recovery
            // restarts every family whose paused flag is clear.
            MaskableSource::Microphone | MaskableSource::SystemAudio => {
                user_paused || super::runtime::system_audio_stops_with_suspension(runtime)
            }
        };
        #[cfg(not(target_os = "macos"))]
        let physical_owner_holds_source = user_paused;

        if physical_owner_holds_source {
            // Flags-only: clear the family paused flag so the owner (user
            // resume, low-disk recovery, suspension recovery) restarts the
            // source when it brings the session back.
            let inactivity = &mut runtime.inactivity;
            match source {
                MaskableSource::Screen => inactivity.set_family_paused_states(
                    false,
                    inactivity.microphone_paused,
                    inactivity.system_audio_paused,
                ),
                MaskableSource::Microphone => inactivity.set_family_paused_states(
                    inactivity.screen_paused,
                    false,
                    inactivity.system_audio_paused,
                ),
                MaskableSource::SystemAudio => inactivity.set_family_paused_states(
                    inactivity.screen_paused,
                    inactivity.microphone_paused,
                    false,
                ),
            }
            return Ok(());
        }

        // Live session: restore the source now, on the existing resume seams
        // (fresh segment file per family). A failure rolls the mask back so the
        // UI still shows the source off and the user can retry.
        #[cfg(target_os = "macos")]
        {
            let result = match source {
                MaskableSource::Screen => {
                    super::segments::resume_screen_from_inactivity(runtime, app_handle)
                }
                MaskableSource::Microphone => {
                    super::segments::resume_microphone_from_inactivity(runtime)
                }
                MaskableSource::SystemAudio => {
                    super::segments::resume_system_audio_from_inactivity(runtime)
                }
            };
            if let Err(error) = result {
                let inactivity = &mut runtime.inactivity;
                match source {
                    MaskableSource::Screen => inactivity.user_masked_screen = true,
                    MaskableSource::Microphone => inactivity.user_masked_microphone = true,
                    MaskableSource::SystemAudio => inactivity.user_masked_system_audio = true,
                }
                return Err(error);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = app_handle;

        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::native_capture::runtime::{
        mark_runtime_session_stopped, CaptureSuspension, CaptureSuspensionKind,
        NativeCaptureRuntime,
    };

    fn all_sources() -> CaptureSources {
        CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }
    }

    /// A running session with no live backends — the shape the deterministic
    /// lifecycle tests use (no real ScreenCaptureKit/AVFoundation/Core Audio).
    fn running_lifecycle(sources: CaptureSources) -> RecordingLifecycle {
        let mut lifecycle = RecordingLifecycle::default();
        *lifecycle.runtime_mut() = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(sources),
            ..Default::default()
        };
        lifecycle
    }

    #[test]
    fn masking_a_source_sets_mask_and_family_paused_flag_without_ending_the_session() {
        let mut lifecycle = running_lifecycle(all_sources());

        let session = lifecycle
            .set_source_masked(None, MaskableSource::Microphone, true)
            .expect("mask should succeed");

        // The real session-liveness field: masking must never end the session.
        // (`session.is_running` folds in a screen-session heartbeat this
        // fabricated backend-less runtime cannot satisfy.)
        assert!(lifecycle.runtime().is_running);
        assert!(session.masked_sources.microphone);
        assert!(!session.masked_sources.screen);
        let runtime = lifecycle.runtime();
        assert!(runtime.inactivity.user_masked_microphone);
        assert!(
            runtime.inactivity.microphone_paused,
            "masked must imply the family paused flag so every raw-flag reader sees it down"
        );
        assert!(!runtime.inactivity.screen_paused);
        assert!(!runtime.inactivity.system_audio_paused);
    }

    // The audio families round-trip on the live resume seams. (The screen's
    // live resume needs a real segment planner + ScreenCaptureKit, so its
    // round-trip is covered by `masked_screen_ignores_screen_suspension_churn`
    // via the deterministic flags-only path instead.)
    #[test]
    fn mask_round_trips_per_source() {
        for source in [MaskableSource::Microphone, MaskableSource::SystemAudio] {
            let mut lifecycle = running_lifecycle(all_sources());
            lifecycle
                .set_source_masked(None, source, true)
                .expect("mask should succeed");
            let session = lifecycle
                .set_source_masked(None, source, false)
                .expect("unmask should succeed");

            assert!(lifecycle.runtime().is_running);
            assert_eq!(
                session.masked_sources,
                CaptureSources::default(),
                "unmask must clear the mask for {source:?}"
            );
            let inactivity = &lifecycle.runtime().inactivity;
            assert!(
                !inactivity.screen_paused
                    && !inactivity.microphone_paused
                    && !inactivity.system_audio_paused,
                "unmask must release the family paused flag for {source:?}"
            );
        }
    }

    #[test]
    fn masking_is_idempotent_and_masking_the_last_live_source_is_refused() {
        let mut lifecycle = running_lifecycle(all_sources());
        lifecycle
            .set_source_masked(None, MaskableSource::Screen, true)
            .expect("mask screen");
        lifecycle
            .set_source_masked(None, MaskableSource::Screen, true)
            .expect("re-masking is a no-op");
        lifecycle
            .set_source_masked(None, MaskableSource::Microphone, true)
            .expect("mask microphone");

        let error = lifecycle
            .set_source_masked(None, MaskableSource::SystemAudio, true)
            .expect_err("the last live source must not be maskable");
        assert_eq!(error.code, "capture_mask_last_source");
        assert!(
            lifecycle.runtime().is_running,
            "masking N-1 sources keeps the session alive"
        );
    }

    #[test]
    fn masking_an_unrequested_source_is_refused() {
        let mut lifecycle = running_lifecycle(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        });
        let error = lifecycle
            .set_source_masked(None, MaskableSource::Microphone, true)
            .expect_err("an unrequested source cannot join mid-session");
        assert_eq!(error.code, "capture_mask_source_not_requested");
    }

    #[test]
    fn mask_requires_a_running_session() {
        let mut lifecycle = RecordingLifecycle::default();
        let error = lifecycle
            .set_source_masked(None, MaskableSource::Screen, true)
            .expect_err("no session, no mask");
        assert_eq!(error.code, "capture_session_not_running");
    }

    // Composition with the inactivity pause: a source both inactivity-paused
    // and masked stays down until both clear — activity cannot resurrect it
    // while masked (`should_resume_*` is mask-gated), and clearing the mask is
    // what finally restores it.
    #[test]
    fn masked_plus_inactivity_paused_stays_down_until_both_clear() {
        let mut lifecycle = running_lifecycle(all_sources());
        // Genuine inactivity pause first.
        lifecycle
            .runtime_mut()
            .inactivity
            .set_family_paused_states(false, true, false);
        lifecycle
            .set_source_masked(None, MaskableSource::Microphone, true)
            .expect("mask on top of the inactivity pause");

        // The activity-driven resume decision must refuse while masked, no
        // matter how fresh the activity is.
        let inactivity = &mut lifecycle.runtime_mut().inactivity;
        inactivity.enabled = true;
        inactivity.idle_timeout_seconds = 60;
        let snapshot = crate::native_capture::inactivity::ActivitySnapshot {
            system_input_idle_ms: Some(0),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(0),
            microphone_activity: crate::native_capture::inactivity::AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(1.0),
            },
            system_audio_activity: crate::native_capture::inactivity::AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(1.0),
            },
        };
        assert!(
            !inactivity.should_resume_microphone_from_inactivity(1_000, snapshot),
            "activity must not resume a masked microphone"
        );

        // Unmask restores it (the resume seam handles the planner-less test
        // runtime by clearing the flags without a physical start).
        lifecycle
            .set_source_masked(None, MaskableSource::Microphone, false)
            .expect("unmask");
        let inactivity = &mut lifecycle.runtime_mut().inactivity;
        assert!(!inactivity.user_masked_microphone);
        assert!(!inactivity.microphone_paused);
        assert!(
            inactivity.should_resume_microphone_from_inactivity(1_000, snapshot) == false,
            "nothing left paused, so the resume decision goes quiet again"
        );
    }

    // The mask is user intent, a separate axis from suspensions: masking the
    // screen under a display suspension is flags-only (the suspension already
    // owns the physical state), the session stays alive, and recovery/wake
    // machinery reads the masked screen as paused (`is_screen_paused`) so it
    // stands down instead of fighting the mask.
    #[test]
    fn masked_screen_ignores_screen_suspension_churn() {
        let mut lifecycle = running_lifecycle(all_sources());
        lifecycle.runtime_mut().capture_suspension = Some(CaptureSuspension::with_kind(
            CaptureSuspensionKind::DisplayUnavailable,
            &CaptureErrorResponse {
                code: "capture_display_unavailable".to_string(),
                message: "no display".to_string(),
            },
        ));

        lifecycle
            .set_source_masked(None, MaskableSource::Screen, true)
            .expect("mask under a display suspension is flags-only");
        let runtime = lifecycle.runtime();
        assert!(runtime.is_running);
        assert!(runtime.inactivity.user_masked_screen);
        assert!(
            runtime.inactivity.is_screen_paused(),
            "suspension recovery and wake recovery both key off is_screen_paused, so the masked screen reads as not-recoverable"
        );
        assert!(
            runtime.capture_suspension.is_some(),
            "masking never clears a suspension itself — recovery dissolves it on its own tick"
        );
        assert!(
            !lifecycle.should_attempt_recovery_after_possible_wake(),
            "wake recovery must never fight a user-masked screen"
        );

        // Unmask while the suspension still stands: flags-only — the mask and
        // the family paused flag clear so the suspension's own recovery driver
        // restarts the screen when it can; no physical resume happens here.
        lifecycle
            .set_source_masked(None, MaskableSource::Screen, false)
            .expect("unmask under a suspension is flags-only");
        let runtime = lifecycle.runtime();
        assert!(!runtime.inactivity.user_masked_screen);
        assert!(!runtime.inactivity.screen_paused);
        assert!(runtime.capture_suspension.is_some());
        assert!(runtime.is_running);
    }

    // The zero-watchdog's sibling: a masked system-audio family must never be
    // brought back by the tap-start retry.
    #[test]
    fn tap_start_retry_never_fights_a_user_masked_tap() {
        let mut lifecycle = running_lifecycle(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        });
        lifecycle
            .set_source_masked(None, MaskableSource::SystemAudio, true)
            .expect("mask system audio");
        assert!(!super::super::lifecycle::should_retry_system_audio_start(
            lifecycle.runtime()
        ));
    }

    // The enrollment restore may run while the user masked the mic mid-clip;
    // it must respect the mask instead of restarting the microphone.
    #[test]
    fn enrollment_restore_never_resurrects_a_masked_microphone() {
        let mut lifecycle = running_lifecycle(all_sources());
        lifecycle
            .set_source_masked(None, MaskableSource::Microphone, true)
            .expect("mask microphone");

        lifecycle
            .restore_microphone_after_out_of_band_recording()
            .expect("restore must be a quiet no-op while masked");
        let runtime = lifecycle.runtime();
        assert!(runtime.active_microphone_session.is_none());
        assert!(runtime.inactivity.user_masked_microphone);
        assert!(runtime.inactivity.microphone_paused);
    }

    // Clear-on-stop: the mask is session-scoped user intent; settings carry the
    // cross-session intent.
    #[test]
    fn session_stop_clears_the_mask() {
        let mut lifecycle = running_lifecycle(all_sources());
        lifecycle
            .set_source_masked(None, MaskableSource::Screen, true)
            .expect("mask screen");

        mark_runtime_session_stopped(lifecycle.runtime_mut());

        let inactivity = &lifecycle.runtime().inactivity;
        assert!(!inactivity.user_masked_screen);
        assert!(!inactivity.user_masked_microphone);
        assert!(!inactivity.user_masked_system_audio);
    }

    // A legacy global pause (is_paused, no family flags) is promoted to
    // per-family flags before the mask lands, so the dead legacy resume path
    // can never resurrect a masked source and the unmasked families keep their
    // own per-family resume.
    #[test]
    fn masking_during_a_legacy_global_pause_promotes_it_to_family_flags() {
        let mut lifecycle = running_lifecycle(all_sources());
        lifecycle.runtime_mut().inactivity.is_paused = true;

        lifecycle
            .set_source_masked(None, MaskableSource::Microphone, true)
            .expect("mask during a legacy global pause");

        let inactivity = &lifecycle.runtime().inactivity;
        assert!(inactivity.screen_paused && inactivity.microphone_paused);
        assert!(inactivity.system_audio_paused);
        assert!(inactivity.user_masked_microphone);
    }
}
