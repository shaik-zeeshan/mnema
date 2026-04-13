use crate::native_capture_inactivity::{
    ActivityPolicyEvaluation, ActivitySnapshot, AudioActivitySourceState,
};
use capture_microphone as microphone_capture;
use serde::Serialize;

use super::runtime::{now_monotonic_marker_ms, NativeCaptureRuntime, NativeCaptureState};

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
    pub audio_activity_sensitivity: u8,
    pub audio_activity_threshold: f32,
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
    pub activity_sources: Vec<IdleDebugActivitySource>,
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
    #[cfg(target_os = "macos")]
    capture_screen::poll_screen_activity();

    ActivitySnapshot {
        system_input_idle_ms: current_system_idle_ms(),
        screen_activity_idle_ms: capture_screen::screen_activity_idle_ms(),
        microphone_activity: AudioActivitySourceState {
            enabled: capture_source_requested(runtime, |sources| sources.microphone),
            idle_ms: microphone_capture::microphone_activity_idle_ms(),
            latest_normalized_level: microphone_capture::microphone_activity_level(),
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: capture_source_requested(runtime, |sources| sources.system_audio),
            idle_ms: capture_screen::system_audio_activity_idle_ms(),
            latest_normalized_level: capture_screen::system_audio_activity_level(),
        },
    }
}

pub(super) fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    let mut runtime = state.lock().expect("native capture state poisoned");
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
    let activity_snapshot = current_activity_snapshot(&runtime);
    let policy = runtime
        .inactivity
        .evaluate_policy_for_snapshot(now, activity_snapshot);
    let effective_idle = policy.effective_idle;
    let microphone_activity_enabled = activity_snapshot.microphone_activity.enabled;
    let system_audio_activity_enabled = activity_snapshot.system_audio_activity.enabled;
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
        audio_activity_sensitivity: runtime.inactivity.audio_activity_sensitivity,
        audio_activity_threshold: runtime.inactivity.audio_activity_threshold(),
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
        activity_sources: idle_debug_activity_sources(&policy),
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
