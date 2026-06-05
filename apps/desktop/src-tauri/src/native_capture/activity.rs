use super::inactivity::{
    ActivityPolicyEvaluation, ActivityPolicyEvaluations, ActivitySnapshot, ActivitySourceKind,
    AudioActivitySourceState,
};
use super::lifecycle::RecordingLifecycle;
use super::NativeCaptureState;
use capture_microphone as microphone_capture;
use capture_types::{
    AudioActivityDecision, AudioActivitySample, IdleDebugActivitySource, IdleDebugInfo,
    MicrophoneVadStatus, RuntimeSourceStatus, RuntimeSourcesStatus,
};
use capture_vad::{configured_adapter_as_str, MicrophonePcmVadFrame};
use std::sync::MutexGuard;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::runtime::{
    microphone_backend_active_for_runtime, microphone_probe_active_for_runtime,
    system_audio_writer_active_for_runtime,
};
use super::runtime::{now_monotonic_marker_ms, NativeCaptureRuntime};

#[derive(Clone, Copy)]
enum AudioPeakReadMode {
    Take,
    Peek,
}

/// Raw sample-facing reading exposed on the debug surface.
///
/// For audio, these values come directly from the capture crates' latest sample
/// bookkeeping and intentionally remain separate from the threshold-qualified
/// idle the inactivity policy uses internally.
#[derive(Debug, Clone, Copy)]
struct RawActivityReading {
    last_unix_ms: Option<u64>,
    level: Option<f32>,
}

/// Threshold-qualified projection for an audio source as seen by inactivity
/// policy evaluation.
#[derive(Debug, Clone, Copy)]
struct QualifiedAudioReading {
    enabled: bool,
    idle_ms: Option<u64>,
    activity_threshold: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct AudioSignalDebugProjection {
    raw_sample: RawActivityReading,
    qualified: QualifiedAudioReading,
}

#[derive(Debug, Clone, Copy)]
struct IdleDebugAudioProjection {
    microphone: AudioSignalDebugProjection,
    system_audio: AudioSignalDebugProjection,
}

fn current_system_idle_ms() -> Option<u64> {
    super::system_idle::current_system_idle_ms()
}

#[cfg(target_os = "windows")]
fn system_audio_activity_idle_ms() -> Option<u64> {
    microphone_capture::system_audio_activity_idle_ms()
}

#[cfg(target_os = "macos")]
fn system_audio_activity_idle_ms() -> Option<u64> {
    capture_screen::system_audio_activity_idle_ms()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_audio_activity_idle_ms() -> Option<u64> {
    None
}

#[cfg(target_os = "windows")]
fn system_audio_activity_level() -> Option<f32> {
    microphone_capture::system_audio_activity_level()
}

#[cfg(target_os = "macos")]
fn system_audio_activity_level() -> Option<f32> {
    capture_screen::system_audio_activity_level()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_audio_activity_level() -> Option<f32> {
    None
}

#[cfg(target_os = "windows")]
fn last_system_audio_activity_unix_ms() -> Option<u64> {
    microphone_capture::last_system_audio_activity_unix_ms()
}

#[cfg(target_os = "macos")]
fn last_system_audio_activity_unix_ms() -> Option<u64> {
    capture_screen::last_system_audio_activity_unix_ms()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn last_system_audio_activity_unix_ms() -> Option<u64> {
    None
}

#[cfg(target_os = "windows")]
fn take_system_audio_activity_window_peak_level() -> Option<f32> {
    microphone_capture::take_system_audio_activity_window_peak_level()
}

#[cfg(target_os = "macos")]
fn take_system_audio_activity_window_peak_level() -> Option<f32> {
    capture_screen::take_system_audio_activity_window_peak_level()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn take_system_audio_activity_window_peak_level() -> Option<f32> {
    None
}

#[cfg(target_os = "windows")]
fn peek_system_audio_activity_window_peak_level() -> Option<f32> {
    microphone_capture::peek_system_audio_activity_window_peak_level()
}

#[cfg(target_os = "macos")]
fn peek_system_audio_activity_window_peak_level() -> Option<f32> {
    capture_screen::peek_system_audio_activity_window_peak_level()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn peek_system_audio_activity_window_peak_level() -> Option<f32> {
    None
}

fn capture_source_requested(
    runtime: &NativeCaptureRuntime,
    selector: fn(&capture_types::CaptureSources) -> bool,
) -> bool {
    runtime.is_running && runtime.requested_sources.as_ref().is_some_and(selector)
}

fn process_pending_microphone_vad_frames(runtime: &mut NativeCaptureRuntime) {
    if !capture_source_requested(runtime, |sources| sources.microphone)
        || !runtime.microphone_vad.uses_vad_adapter()
    {
        return;
    }

    for frame in microphone_capture::take_microphone_vad_pcm_frames(96) {
        let vad_frame = MicrophonePcmVadFrame {
            samples: &frame.samples,
            sample_rate_hz: frame.sample_rate_hz,
            captured_at_unix_ms: frame.captured_at_unix_ms,
            normalized_peak_level: frame.normalized_peak_level,
        };

        match runtime.microphone_vad.process_pcm_frame(vad_frame) {
            Ok(outcome) => {
                if outcome.vad_speech_detected {
                    microphone_capture::record_microphone_vad_speech_event(
                        microphone_capture::MicrophoneVadSpeechEvent {
                            media_start_secs: frame.media_start_secs,
                            media_end_secs: frame.media_end_secs,
                        },
                    );
                }
            }
            Err(error) => {
                super::debug_log::log_warn(format!(
                    "failed to process microphone VAD PCM frame: {error}"
                ));
                microphone_capture::disable_microphone_vad_boundary_trim_for_current_output();
                break;
            }
        }
    }
}

pub(crate) fn current_activity_snapshot(runtime: &mut NativeCaptureRuntime) -> ActivitySnapshot {
    current_activity_snapshot_with_audio_peak_mode(runtime, AudioPeakReadMode::Take)
}

pub(super) fn current_activity_snapshot_for_debug(
    runtime: &mut NativeCaptureRuntime,
) -> ActivitySnapshot {
    current_activity_snapshot_with_audio_peak_mode(runtime, AudioPeakReadMode::Peek)
}

fn current_activity_snapshot_with_audio_peak_mode(
    runtime: &mut NativeCaptureRuntime,
    audio_peak_read_mode: AudioPeakReadMode,
) -> ActivitySnapshot {
    #[cfg(target_os = "macos")]
    if should_poll_screen_activity(
        runtime.is_running,
        capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()),
    ) {
        capture_screen::poll_screen_activity();
    }

    if matches!(audio_peak_read_mode, AudioPeakReadMode::Take) {
        process_pending_microphone_vad_frames(runtime);
    }

    let microphone_peak_level = match audio_peak_read_mode {
        AudioPeakReadMode::Take => microphone_capture::take_microphone_activity_window_peak_level(),
        AudioPeakReadMode::Peek => microphone_capture::peek_microphone_activity_window_peak_level(),
    };
    let microphone_speech = match audio_peak_read_mode {
        AudioPeakReadMode::Take => runtime.microphone_vad.decide_from_peak_level(
            microphone_peak_level,
            microphone_capture::microphone_activity_idle_ms(),
            runtime.inactivity.microphone_activity_threshold(),
        ),
        AudioPeakReadMode::Peek => runtime.microphone_vad.peek_decision_from_peak_level(
            microphone_peak_level,
            microphone_capture::microphone_activity_idle_ms(),
            runtime.inactivity.microphone_activity_threshold(),
        ),
    };

    ActivitySnapshot {
        system_input_idle_ms: current_system_idle_ms(),
        screen_activity_enabled: capture_source_requested(runtime, |sources| sources.screen),
        screen_activity_idle_ms: capture_screen::screen_activity_idle_ms(),
        microphone_activity: AudioActivitySourceState {
            enabled: capture_source_requested(runtime, |sources| sources.microphone),
            idle_ms: microphone_speech.idle_ms,
            latest_normalized_level: microphone_speech.latest_normalized_level,
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: capture_source_requested(runtime, |sources| sources.system_audio),
            idle_ms: system_audio_activity_idle_ms(),
            latest_normalized_level: match audio_peak_read_mode {
                AudioPeakReadMode::Take => take_system_audio_activity_window_peak_level(),
                AudioPeakReadMode::Peek => peek_system_audio_activity_window_peak_level(),
            },
        },
    }
}

#[cfg(target_os = "macos")]
pub(super) fn should_poll_screen_activity(is_running: bool, screen_stream_live: bool) -> bool {
    // Only poll via CGDisplayCreateImage when no live capture stream can supply
    // screen activity samples. Soft-paused ScreenCaptureKit sessions keep the
    // stream alive, so polling there duplicates compositor work.
    !is_running || !screen_stream_live
}

pub(super) fn lock_runtime_for_idle_debug(
    state: &NativeCaptureState,
) -> MutexGuard<'_, RecordingLifecycle> {
    match state.lock() {
        Ok(runtime) => runtime,
        Err(poisoned) => {
            super::debug_log::log(
                "native capture state poisoned while reading idle debug; returning best-effort snapshot",
            );
            poisoned.into_inner()
        }
    }
}

pub(super) fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    let mut runtime = lock_runtime_for_idle_debug(&state);
    let runtime = runtime.runtime_mut();
    let now = now_monotonic_marker_ms();
    let system_idle_ms = current_system_idle_ms();
    let screen_activity_last_unix_ms = capture_screen::last_screen_activity_unix_ms();
    let screen_activity_idle_ms = capture_screen::screen_activity_idle_ms();
    let microphone_raw_sample = RawActivityReading {
        last_unix_ms: microphone_capture::last_microphone_activity_unix_ms(),
        level: microphone_capture::microphone_activity_level(),
    };
    let system_audio_raw_sample = RawActivityReading {
        last_unix_ms: last_system_audio_activity_unix_ms(),
        level: system_audio_activity_level(),
    };
    let activity_snapshot = current_activity_snapshot_for_debug(runtime);
    let combined_policy = runtime
        .inactivity
        .evaluate_policy_for_snapshot(now, activity_snapshot);
    let policies = runtime
        .inactivity
        .evaluate_policies_for_snapshot(now, activity_snapshot);
    let audio_projection = idle_debug_audio_projection(
        &combined_policy,
        microphone_raw_sample,
        system_audio_raw_sample,
    );
    let family_fields = idle_debug_family_fields(&policies, &runtime.inactivity);
    let effective_idle = combined_policy.effective_idle;
    let system_idle_available = system_idle_ms.is_some();
    let detector_source = if cfg!(target_os = "macos") {
        if system_idle_available {
            "core_graphics".to_string()
        } else {
            "core_graphics_unavailable".to_string()
        }
    } else if cfg!(target_os = "windows") {
        if system_idle_available {
            "get_last_input_info".to_string()
        } else {
            "get_last_input_info_unavailable".to_string()
        }
    } else {
        "unavailable".to_string()
    };

    IdleDebugInfo {
        system_idle_ms,
        system_idle_available,
        inactivity_enabled: runtime.inactivity.enabled,
        idle_timeout_seconds: runtime.inactivity.idle_timeout_seconds,
        is_inactivity_paused: runtime.inactivity.is_paused,
        detector_source,
        activity_mode: match runtime.inactivity.activity_mode {
            capture_types::InactivityActivityMode::SystemInputOnly => {
                "system_input_only".to_string()
            }
            capture_types::InactivityActivityMode::SystemInputOrScreen => {
                "system_input_or_screen".to_string()
            }
            capture_types::InactivityActivityMode::SystemInputOrScreenOrAudio => {
                "system_input_or_screen_or_audio".to_string()
            }
        },
        microphone_activity_sensitivity: runtime.inactivity.microphone_activity_sensitivity,
        system_audio_activity_sensitivity: runtime.inactivity.system_audio_activity_sensitivity,
        screen_activity_last_unix_ms,
        screen_activity_idle_ms,
        microphone_activity_sample: AudioActivitySample {
            last_unix_ms: audio_projection.microphone.raw_sample.last_unix_ms,
            level: audio_projection.microphone.raw_sample.level,
        },
        microphone_activity_decision: AudioActivityDecision {
            enabled: audio_projection.microphone.qualified.enabled,
            idle_ms: audio_projection.microphone.qualified.idle_ms,
            activity_threshold: audio_projection
                .microphone
                .qualified
                .activity_threshold
                .or_else(|| Some(runtime.inactivity.microphone_activity_threshold())),
            detector: Some(
                runtime
                    .microphone_vad
                    .effective_adapter()
                    .as_str()
                    .to_string(),
            ),
        },
        system_audio_activity_sample: AudioActivitySample {
            last_unix_ms: audio_projection.system_audio.raw_sample.last_unix_ms,
            level: audio_projection.system_audio.raw_sample.level,
        },
        system_audio_activity_decision: AudioActivityDecision {
            enabled: audio_projection.system_audio.qualified.enabled,
            idle_ms: audio_projection.system_audio.qualified.idle_ms,
            activity_threshold: audio_projection
                .system_audio
                .qualified
                .activity_threshold
                .or_else(|| Some(runtime.inactivity.system_audio_activity_threshold())),
            detector: Some("peak_level".to_string()),
        },
        microphone_vad: MicrophoneVadStatus {
            configured_adapter: configured_adapter_as_str(
                runtime.microphone_vad.configured_adapter(),
            )
            .to_string(),
            effective_adapter: runtime
                .microphone_vad
                .effective_adapter()
                .as_str()
                .to_string(),
            fallback_reason: runtime.microphone_vad.fallback_reason().map(str::to_string),
        },
        effective_idle_ms: effective_idle.idle_ms,
        effective_idle_source: effective_idle.source.as_str().to_string(),
        screen_effective_idle_ms: family_fields.screen_effective_idle_ms,
        screen_effective_idle_source: family_fields.screen_effective_idle_source,
        screen_paused: family_fields.screen_paused,
        microphone_effective_idle_ms: family_fields.microphone_effective_idle_ms,
        microphone_effective_idle_source: family_fields.microphone_effective_idle_source,
        microphone_paused: family_fields.microphone_paused,
        system_audio_effective_idle_ms: family_fields.system_audio_effective_idle_ms,
        system_audio_effective_idle_source: family_fields.system_audio_effective_idle_source,
        system_audio_paused: family_fields.system_audio_paused,
        activity_sources: idle_debug_activity_sources(&combined_policy),
        runtime_sources: build_runtime_sources_status(&runtime),
    }
}

fn policy_source_sample(
    policy: &ActivityPolicyEvaluation,
    kind: ActivitySourceKind,
) -> Option<&super::inactivity::ActivitySourceSample> {
    policy.sources.iter().find(|source| source.kind == kind)
}

fn qualified_audio_reading(
    policy: &ActivityPolicyEvaluation,
    kind: ActivitySourceKind,
) -> QualifiedAudioReading {
    let source = policy_source_sample(policy, kind);

    QualifiedAudioReading {
        enabled: source.is_some_and(|source| source.enabled),
        idle_ms: source.and_then(|source| source.idle_ms),
        activity_threshold: source.and_then(|source| source.activity_threshold),
    }
}

fn idle_debug_audio_projection(
    policy: &ActivityPolicyEvaluation,
    microphone_raw_sample: RawActivityReading,
    system_audio_raw_sample: RawActivityReading,
) -> IdleDebugAudioProjection {
    IdleDebugAudioProjection {
        microphone: AudioSignalDebugProjection {
            raw_sample: microphone_raw_sample,
            qualified: qualified_audio_reading(policy, ActivitySourceKind::MicrophoneCapture),
        },
        system_audio: AudioSignalDebugProjection {
            raw_sample: system_audio_raw_sample,
            qualified: qualified_audio_reading(policy, ActivitySourceKind::SystemAudioCapture),
        },
    }
}

pub(super) fn build_runtime_sources_status(runtime: &NativeCaptureRuntime) -> RuntimeSourcesStatus {
    let requested_screen = runtime.requested_sources.as_ref().is_some_and(|s| s.screen);
    let requested_mic = runtime
        .requested_sources
        .as_ref()
        .is_some_and(|s| s.microphone);
    let requested_sys = runtime
        .requested_sources
        .as_ref()
        .is_some_and(|s| s.system_audio);
    // User Capture Pause tears down the live sessions while the Capture
    // Session stays alive, so the per-source pills must report "paused"
    // rather than falling through to the "starting" state the dead
    // sessions would otherwise imply.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let user_paused = runtime.user_capture_paused;

    #[cfg(target_os = "macos")]
    {
        let screen_session =
            capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref());
        let mic_session = microphone_probe_active_for_runtime(runtime);
        // System audio runs over the screen session; "session active" means the
        // host screen session is up. Writer active is gated by the dedicated
        // truth helper.
        let sys_session = screen_session;

        let mic_writer = microphone_backend_active_for_runtime(runtime);
        let sys_writer = system_audio_writer_active_for_runtime(runtime);
        // Screen "writer active": session attached and not paused; the screen
        // writer is implicit in the session lifetime.
        let screen_writer = screen_session && !runtime.inactivity.is_screen_paused();
        let privacy_suspension_reason = runtime
            .privacy_capture_suspension
            .as_ref()
            .map(|suspension| suspension.reason.clone());

        RuntimeSourcesStatus {
            screen: RuntimeSourceStatus {
                requested: requested_screen,
                paused: user_paused || runtime.inactivity.is_screen_paused(),
                session_active: Some(screen_session),
                writer_active: Some(screen_writer),
                output_path: runtime.recording_file.clone(),
                reason: if requested_screen {
                    privacy_suspension_reason.clone()
                } else {
                    Some("not_requested".to_string())
                },
            },
            microphone: RuntimeSourceStatus {
                requested: requested_mic,
                paused: user_paused || runtime.inactivity.is_microphone_paused(),
                session_active: Some(mic_session),
                writer_active: Some(mic_writer),
                output_path: runtime.microphone_recording_file.clone(),
                reason: if requested_mic {
                    None
                } else {
                    Some("not_requested".to_string())
                },
            },
            system_audio: RuntimeSourceStatus {
                requested: requested_sys,
                paused: user_paused || runtime.inactivity.is_system_audio_paused(),
                session_active: Some(sys_session),
                writer_active: Some(sys_writer),
                output_path: runtime.system_audio_recording_file.clone(),
                reason: if requested_sys {
                    privacy_suspension_reason
                } else {
                    Some("not_requested".to_string())
                },
            },
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Windows mirrors the macOS truths with one structural difference:
        // system audio is an independent WASAPI source (ADR 0022) with its
        // own session, so its "session active" checks that session instead
        // of riding on the screen session.
        let screen_session =
            capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref());
        let mic_session = microphone_probe_active_for_runtime(runtime);
        let sys_session = runtime.active_system_audio_session.is_some();

        let screen_writer = screen_session && !runtime.inactivity.is_screen_paused();
        let mic_writer = microphone_backend_active_for_runtime(runtime);
        let sys_writer = system_audio_writer_active_for_runtime(runtime);

        let reason_for = |requested: bool| {
            if requested {
                None
            } else {
                Some("not_requested".to_string())
            }
        };

        RuntimeSourcesStatus {
            screen: RuntimeSourceStatus {
                requested: requested_screen,
                paused: user_paused || runtime.inactivity.is_screen_paused(),
                session_active: Some(screen_session),
                writer_active: Some(screen_writer),
                output_path: runtime.recording_file.clone(),
                reason: reason_for(requested_screen),
            },
            microphone: RuntimeSourceStatus {
                requested: requested_mic,
                paused: user_paused || runtime.inactivity.is_microphone_paused(),
                session_active: Some(mic_session),
                writer_active: Some(mic_writer),
                output_path: runtime.microphone_recording_file.clone(),
                reason: reason_for(requested_mic),
            },
            system_audio: RuntimeSourceStatus {
                requested: requested_sys,
                paused: user_paused || runtime.inactivity.is_system_audio_paused(),
                session_active: Some(sys_session),
                writer_active: Some(sys_writer),
                output_path: runtime.system_audio_recording_file.clone(),
                reason: reason_for(requested_sys),
            },
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let stub = |requested: bool, paused: bool| RuntimeSourceStatus {
            requested,
            paused,
            session_active: None,
            writer_active: None,
            output_path: None,
            reason: Some("unsupported_platform".to_string()),
        };
        RuntimeSourcesStatus {
            screen: stub(requested_screen, runtime.inactivity.is_screen_paused()),
            microphone: stub(requested_mic, runtime.inactivity.is_microphone_paused()),
            system_audio: stub(requested_sys, runtime.inactivity.is_system_audio_paused()),
        }
    }
}

pub(super) fn idle_debug_activity_sources(
    policy: &ActivityPolicyEvaluation,
) -> Vec<IdleDebugActivitySource> {
    policy
        .sources
        .iter()
        .map(|source| IdleDebugActivitySource {
            kind: source.kind.as_str().to_string(),
            enabled: source.enabled,
            available: source.available,
            idle_ms: source.idle_ms,
            latest_normalized_level: source.latest_normalized_level,
            activity_threshold: source.activity_threshold,
            selected: source.kind == policy.effective_idle.source,
        })
        .collect()
}

/// Build the separate screen/microphone/system-audio debug fields from split
/// policy evaluations. This is a pure function testable without Tauri state.
pub(super) fn idle_debug_family_fields(
    policies: &ActivityPolicyEvaluations,
    inactivity: &super::inactivity::InactivityState,
) -> IdleDebugFamilyFields {
    IdleDebugFamilyFields {
        screen_effective_idle_ms: policies.screen.effective_idle.idle_ms,
        screen_effective_idle_source: policies.screen.effective_idle.source.as_str().to_string(),
        screen_paused: inactivity.is_screen_paused(),
        microphone_effective_idle_ms: policies.microphone.effective_idle.idle_ms,
        microphone_effective_idle_source: policies
            .microphone
            .effective_idle
            .source
            .as_str()
            .to_string(),
        microphone_paused: inactivity.is_microphone_paused(),
        system_audio_effective_idle_ms: policies.system_audio.effective_idle.idle_ms,
        system_audio_effective_idle_source: policies
            .system_audio
            .effective_idle
            .source
            .as_str()
            .to_string(),
        system_audio_paused: inactivity.is_system_audio_paused(),
    }
}

/// Separated screen/microphone/system-audio fields extracted for debug surfaces.
#[derive(Debug, Clone)]
pub(super) struct IdleDebugFamilyFields {
    pub screen_effective_idle_ms: u64,
    pub screen_effective_idle_source: String,
    pub screen_paused: bool,
    pub microphone_effective_idle_ms: u64,
    pub microphone_effective_idle_source: String,
    pub microphone_paused: bool,
    pub system_audio_effective_idle_ms: u64,
    pub system_audio_effective_idle_source: String,
    pub system_audio_paused: bool,
}

#[cfg(test)]
mod tests {
    use super::super::inactivity::{ActivitySourceSample, EffectiveIdle};
    use super::*;

    #[test]
    fn audio_projection_keeps_raw_samples_separate_from_qualified_policy_fields() {
        let projection = idle_debug_audio_projection(
            &ActivityPolicyEvaluation {
                effective_idle: EffectiveIdle {
                    source: ActivitySourceKind::MicrophoneCapture,
                    idle_ms: 250,
                },
                sources: vec![
                    ActivitySourceSample {
                        kind: ActivitySourceKind::MicrophoneCapture,
                        enabled: true,
                        available: true,
                        idle_ms: Some(250),
                        latest_normalized_level: Some(0.4),
                        activity_threshold: Some(0.3),
                    },
                    ActivitySourceSample {
                        kind: ActivitySourceKind::SystemAudioCapture,
                        enabled: false,
                        available: false,
                        idle_ms: None,
                        latest_normalized_level: Some(0.1),
                        activity_threshold: Some(0.2),
                    },
                ],
            },
            RawActivityReading {
                last_unix_ms: Some(10),
                level: Some(0.9),
            },
            RawActivityReading {
                last_unix_ms: Some(20),
                level: Some(0.8),
            },
        );

        assert_eq!(projection.microphone.raw_sample.last_unix_ms, Some(10));
        assert_eq!(projection.microphone.raw_sample.level, Some(0.9));
        assert!(projection.microphone.qualified.enabled);
        assert_eq!(projection.microphone.qualified.idle_ms, Some(250));
        assert_eq!(
            projection.microphone.qualified.activity_threshold,
            Some(0.3)
        );

        assert_eq!(projection.system_audio.raw_sample.last_unix_ms, Some(20));
        assert_eq!(projection.system_audio.raw_sample.level, Some(0.8));
        assert!(!projection.system_audio.qualified.enabled);
        assert_eq!(projection.system_audio.qualified.idle_ms, None);
        assert_eq!(
            projection.system_audio.qualified.activity_threshold,
            Some(0.2)
        );
    }
}
