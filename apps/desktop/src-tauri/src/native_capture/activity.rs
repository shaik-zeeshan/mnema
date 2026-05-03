use crate::native_capture_inactivity::{
    ActivityPolicyEvaluation, ActivityPolicyEvaluations, ActivitySnapshot, AudioActivitySourceState,
};
use capture_microphone as microphone_capture;
use serde::Serialize;
use std::sync::MutexGuard;

#[cfg(target_os = "macos")]
use super::runtime::{
    microphone_backend_active_for_runtime, microphone_probe_active_for_runtime,
    system_audio_writer_active_for_runtime,
};
use super::runtime::{now_monotonic_marker_ms, NativeCaptureRuntime, NativeCaptureState};

#[derive(Clone, Copy)]
enum AudioPeakReadMode {
    Take,
    Peek,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDebugInfo {
    pub system_idle_ms: Option<u64>,
    pub system_idle_available: bool,
    pub inactivity_enabled: bool,
    pub idle_timeout_seconds: u64,
    pub is_inactivity_paused: bool,
    pub detector_source: String,
    pub activity_mode: String,
    pub microphone_activity_sensitivity: u8,
    pub system_audio_activity_sensitivity: u8,
    pub microphone_activity_threshold: f32,
    pub system_audio_activity_threshold: f32,
    pub screen_activity_last_unix_ms: Option<u64>,
    pub screen_activity_idle_ms: Option<u64>,
    pub microphone_activity_last_unix_ms: Option<u64>,
    pub microphone_activity_idle_ms: Option<u64>,
    pub microphone_activity_level: Option<f32>,
    pub microphone_activity_enabled: bool,
    pub system_audio_activity_last_unix_ms: Option<u64>,
    pub system_audio_activity_idle_ms: Option<u64>,
    pub system_audio_activity_level: Option<f32>,
    pub system_audio_activity_enabled: bool,
    pub effective_idle_ms: u64,
    #[serde(rename = "effectiveActivitySource")]
    pub effective_idle_source: String,
    pub screen_effective_idle_ms: u64,
    #[serde(rename = "screenEffectiveActivitySource")]
    pub screen_effective_idle_source: String,
    pub screen_paused: bool,
    pub microphone_effective_idle_ms: u64,
    #[serde(rename = "microphoneEffectiveActivitySource")]
    pub microphone_effective_idle_source: String,
    pub microphone_paused: bool,
    pub system_audio_effective_idle_ms: u64,
    #[serde(rename = "systemAudioEffectiveActivitySource")]
    pub system_audio_effective_idle_source: String,
    pub system_audio_paused: bool,
    pub activity_sources: Vec<IdleDebugActivitySource>,
    /// Operational truth for each capture source family — what is requested,
    /// whether the underlying capture session/writer is currently attached, and
    /// the on-disk output path when known. Distinguishes "requested but paused"
    /// from "session running" from "writer attached/active".
    pub runtime_sources: RuntimeSourcesStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSourcesStatus {
    pub screen: RuntimeSourceStatus,
    pub microphone: RuntimeSourceStatus,
    pub system_audio: RuntimeSourceStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSourceStatus {
    /// Source was requested by the active recording.
    pub requested: bool,
    /// Source family is currently inactivity-paused.
    pub paused: bool,
    /// Native capture session for this source is currently attached/running.
    /// On non-macOS or before recording starts, this is null with a reason.
    pub session_active: Option<bool>,
    /// Output writer for this source is currently attached and accepting samples
    /// (i.e. session running AND not paused AND output file resolved). null when
    /// the platform cannot report this (non-macOS).
    pub writer_active: Option<bool>,
    /// Last known on-disk output path for the active segment, when available.
    pub output_path: Option<String>,
    /// Short machine-readable reason when truth is unavailable (e.g. "non_macos",
    /// "not_running"). null when fields are populated normally.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDebugActivitySource {
    pub kind: String,
    pub enabled: bool,
    pub available: bool,
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
    pub activity_threshold: Option<f32>,
    pub selected: bool,
}

#[cfg(target_os = "macos")]
fn current_system_idle_ms() -> Option<u64> {
    crate::native_capture_system_idle::current_system_idle_ms()
}

#[cfg(not(target_os = "macos"))]
fn current_system_idle_ms() -> Option<u64> {
    None
}

fn capture_source_requested(
    runtime: &NativeCaptureRuntime,
    selector: fn(&capture_types::CaptureSources) -> bool,
) -> bool {
    runtime.is_running && runtime.requested_sources.as_ref().is_some_and(selector)
}

pub(super) fn current_activity_snapshot(runtime: &NativeCaptureRuntime) -> ActivitySnapshot {
    current_activity_snapshot_with_audio_peak_mode(runtime, AudioPeakReadMode::Take)
}

pub(super) fn current_activity_snapshot_for_debug(runtime: &NativeCaptureRuntime) -> ActivitySnapshot {
    current_activity_snapshot_with_audio_peak_mode(runtime, AudioPeakReadMode::Peek)
}

fn current_activity_snapshot_with_audio_peak_mode(
    runtime: &NativeCaptureRuntime,
    audio_peak_read_mode: AudioPeakReadMode,
) -> ActivitySnapshot {
    // Only poll screen activity via CGDisplayCreateImage when the capture
    // stream is not running.  While the stream is active, the stream output
    // callback already updates screen activity from frame samples — polling on
    // top of that introduces redundant CGDisplayCreateImage snapshots whose
    // fingerprints can drift from the stream fingerprints and cause false
    // activity changes.  During inactivity pauses (stream stopped) we still
    // need polling to detect resumed screen changes.
    #[cfg(target_os = "macos")]
    if !runtime.is_running
        || runtime.inactivity.is_screen_paused()
        || runtime.active_screen_session.is_none()
    {
        capture_screen::poll_screen_activity();
    }

    ActivitySnapshot {
        system_input_idle_ms: current_system_idle_ms(),
        screen_activity_idle_ms: capture_screen::screen_activity_idle_ms(),
        microphone_activity: AudioActivitySourceState {
            enabled: capture_source_requested(runtime, |sources| sources.microphone),
            idle_ms: microphone_capture::microphone_activity_idle_ms(),
            // The inactivity loop polls at a coarse interval, so use the peak
            // seen since the last poll rather than a single instantaneous sample.
            latest_normalized_level: match audio_peak_read_mode {
                AudioPeakReadMode::Take => {
                    microphone_capture::take_microphone_activity_window_peak_level()
                }
                AudioPeakReadMode::Peek => {
                    microphone_capture::peek_microphone_activity_window_peak_level()
                }
            },
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: capture_source_requested(runtime, |sources| sources.system_audio),
            idle_ms: capture_screen::system_audio_activity_idle_ms(),
            latest_normalized_level: match audio_peak_read_mode {
                AudioPeakReadMode::Take => {
                    capture_screen::take_system_audio_activity_window_peak_level()
                }
                AudioPeakReadMode::Peek => {
                    capture_screen::peek_system_audio_activity_window_peak_level()
                }
            },
        },
    }
}

pub(super) fn lock_runtime_for_idle_debug(
    state: &NativeCaptureState,
) -> MutexGuard<'_, NativeCaptureRuntime> {
    match state.lock() {
        Ok(runtime) => runtime,
        Err(poisoned) => {
            crate::native_capture_debug_log::log(
                "native capture state poisoned while reading idle debug; returning best-effort snapshot",
            );
            poisoned.into_inner()
        }
    }
}

pub(super) fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    let mut runtime = lock_runtime_for_idle_debug(&state);
    let now = now_monotonic_marker_ms();
    let system_idle_ms = crate::native_capture_system_idle::current_system_idle_ms();
    let screen_activity_last_unix_ms = capture_screen::last_screen_activity_unix_ms();
    let screen_activity_idle_ms = capture_screen::screen_activity_idle_ms();
    let microphone_activity_last_unix_ms = microphone_capture::last_microphone_activity_unix_ms();
    let microphone_activity_idle_ms = microphone_capture::microphone_activity_idle_ms();
    let microphone_activity_level = microphone_capture::microphone_activity_level();
    let system_audio_activity_last_unix_ms = capture_screen::last_system_audio_activity_unix_ms();
    let system_audio_activity_idle_ms = capture_screen::system_audio_activity_idle_ms();
    let system_audio_activity_level = capture_screen::system_audio_activity_level();
    let activity_snapshot = current_activity_snapshot_for_debug(&runtime);
    let combined_policy = runtime
        .inactivity
        .evaluate_policy_for_snapshot(now, activity_snapshot);
    let policies = runtime
        .inactivity
        .evaluate_policies_for_snapshot(now, activity_snapshot);
    let effective_idle = combined_policy.effective_idle;
    let microphone_activity_enabled = activity_snapshot.microphone_activity.enabled;
    let system_audio_activity_enabled = activity_snapshot.system_audio_activity.enabled;
    let screen_paused = runtime.inactivity.is_screen_paused();
    let microphone_paused = runtime.inactivity.is_microphone_paused();
    let system_audio_paused = runtime.inactivity.is_system_audio_paused();
    let system_idle_available = system_idle_ms.is_some();
    let detector_source = if cfg!(target_os = "macos") {
        if system_idle_available {
            "core_graphics".to_string()
        } else {
            "core_graphics_unavailable".to_string()
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
        microphone_activity_threshold: runtime.inactivity.microphone_activity_threshold(),
        system_audio_activity_threshold: runtime.inactivity.system_audio_activity_threshold(),
        screen_activity_last_unix_ms,
        screen_activity_idle_ms,
        microphone_activity_last_unix_ms,
        microphone_activity_idle_ms,
        microphone_activity_level,
        microphone_activity_enabled,
        system_audio_activity_last_unix_ms,
        system_audio_activity_idle_ms,
        system_audio_activity_level,
        system_audio_activity_enabled,
        effective_idle_ms: effective_idle.idle_ms,
        effective_idle_source: effective_idle.source.as_str().to_string(),
        screen_effective_idle_ms: policies.screen.effective_idle.idle_ms,
        screen_effective_idle_source: policies.screen.effective_idle.source.as_str().to_string(),
        screen_paused,
        microphone_effective_idle_ms: policies.microphone.effective_idle.idle_ms,
        microphone_effective_idle_source: policies
            .microphone
            .effective_idle
            .source
            .as_str()
            .to_string(),
        microphone_paused,
        system_audio_effective_idle_ms: policies.system_audio.effective_idle.idle_ms,
        system_audio_effective_idle_source: policies
            .system_audio
            .effective_idle
            .source
            .as_str()
            .to_string(),
        system_audio_paused,
        activity_sources: idle_debug_activity_sources(&combined_policy),
        runtime_sources: build_runtime_sources_status(&runtime),
    }
}

fn build_runtime_sources_status(runtime: &NativeCaptureRuntime) -> RuntimeSourcesStatus {
    let requested_screen = runtime.requested_sources.as_ref().is_some_and(|s| s.screen);
    let requested_mic = runtime
        .requested_sources
        .as_ref()
        .is_some_and(|s| s.microphone);
    let requested_sys = runtime
        .requested_sources
        .as_ref()
        .is_some_and(|s| s.system_audio);

    #[cfg(target_os = "macos")]
    {
        let screen_session = runtime.active_screen_session.is_some();
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

        RuntimeSourcesStatus {
            screen: RuntimeSourceStatus {
                requested: requested_screen,
                paused: runtime.inactivity.is_screen_paused(),
                session_active: Some(screen_session),
                writer_active: Some(screen_writer),
                output_path: runtime.recording_file.clone(),
                reason: if requested_screen {
                    None
                } else {
                    Some("not_requested".to_string())
                },
            },
            microphone: RuntimeSourceStatus {
                requested: requested_mic,
                paused: runtime.inactivity.is_microphone_paused(),
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
                paused: runtime.inactivity.is_system_audio_paused(),
                session_active: Some(sys_session),
                writer_active: Some(sys_writer),
                output_path: runtime.system_audio_recording_file.clone(),
                reason: if requested_sys {
                    None
                } else {
                    Some("not_requested".to_string())
                },
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let stub = |requested: bool, paused: bool| RuntimeSourceStatus {
            requested,
            paused,
            session_active: None,
            writer_active: None,
            output_path: None,
            reason: Some("non_macos".to_string()),
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
    inactivity: &crate::native_capture_inactivity::InactivityState,
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
