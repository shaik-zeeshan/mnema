//! The microphone controller: its preferences, the waited-for-device rules and
//! the one restart path. The device-change notifier and the reconnect, which
//! drive `NativeCaptureState`, are [`controller`].

use capture_microphone as microphone_capture;
use capture_types::{
    CaptureErrorResponse, CaptureSources, MicrophoneControllerState, MicrophoneDisconnectPolicy,
    MicrophonePreference, MicrophonePreferenceMode,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};

use super::runtime::{
    ensure_microphone_planner_for_runtime, now_unix_ms, refresh_runtime_planner_dates,
    NativeCaptureRuntime,
};
use crate::native_capture_output::set_current_microphone_output_file;

#[derive(Debug, Clone)]
pub struct MicrophoneControllerPreferencesRuntime {
    pub preference: MicrophonePreference,
    pub disconnect_policy: MicrophoneDisconnectPolicy,
}

impl Default for MicrophoneControllerPreferencesRuntime {
    fn default() -> Self {
        Self {
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::Default,
                device_id: None,
            },
            disconnect_policy: MicrophoneDisconnectPolicy::FallbackToDefault,
        }
    }
}

/// One preference, shared by app-core and the host (both manage the same `Arc`).
pub type MicrophoneControllerPreferencesState = Arc<Mutex<MicrophoneControllerPreferencesRuntime>>;
pub type MicrophoneDeviceChangeNotifierState =
    Mutex<Option<microphone_capture::MicrophoneDeviceChangeNotifier>>;

pub const MICROPHONE_CONTROLLER_CHANGED_EVENT: &str = "microphone_controller_changed";
pub const MICROPHONE_AUTO_DISCONNECT_TRANSITION_FAILED_EVENT: &str =
    "microphone_auto_disconnect_transition_failed";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneAutoDisconnectTransitionFailedEvent {
    pub context: String,
    pub code: String,
    pub message: String,
}

pub fn microphone_auto_disconnect_transition_failed_event(
    error: &CaptureErrorResponse,
) -> MicrophoneAutoDisconnectTransitionFailedEvent {
    MicrophoneAutoDisconnectTransitionFailedEvent {
        context: "stop_before_wait_for_same_device".to_string(),
        code: error.code.clone(),
        message: error.message.clone(),
    }
}

pub fn validate_microphone_preference(
    preference: MicrophonePreference,
) -> Result<MicrophonePreference, CaptureErrorResponse> {
    if preference.mode != MicrophonePreferenceMode::SpecificDevice {
        return Ok(preference);
    }

    let device_id = preference
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CaptureErrorResponse {
            code: "microphone_specific_device_id_required".to_string(),
            message: "A non-empty device_id is required when preference.mode is specific_device"
                .to_string(),
        })?;

    Ok(MicrophonePreference {
        mode: preference.mode,
        device_id: Some(device_id),
    })
}

pub fn emit_microphone_controller_changed(core: &crate::AppCore, state: MicrophoneControllerState) {
    core.host()
        .emit(crate::CoreEvent::MicrophoneControllerChanged(state));
}

pub fn emit_microphone_auto_disconnect_transition_failed(
    core: &crate::AppCore,
    error: &CaptureErrorResponse,
) {
    let payload = microphone_auto_disconnect_transition_failed_event(error);
    core.host()
        .emit(crate::CoreEvent::MicrophoneAutoDisconnectTransitionFailed(
            payload,
        ));
}

pub fn should_wait_for_same_microphone_device(state: &MicrophoneControllerState) -> bool {
    state.preference.mode == MicrophonePreferenceMode::SpecificDevice
        && state.disconnect_policy == MicrophoneDisconnectPolicy::WaitForSameDevice
        && state.preference.device_id.is_some()
        && state.effective_device.is_none()
}

#[cfg(target_os = "macos")]
pub fn should_move_microphone_capture_to_waiting_state(
    runtime_is_running: bool,
    requested_sources: Option<&CaptureSources>,
    has_active_microphone_session: bool,
    state: &MicrophoneControllerState,
) -> bool {
    runtime_is_running
        && requested_sources.is_some_and(|sources| sources.microphone)
        && has_active_microphone_session
        && should_wait_for_same_microphone_device(state)
}

#[cfg(target_os = "macos")]
pub fn next_microphone_output_file_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Result<String, CaptureErrorResponse> {
    let planner = runtime
        .microphone_planner
        .as_ref()
        .ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture microphone planner missing while reconnecting microphone".to_string(),
        })?;
    let audio_dir = planner.audio_dir();
    std::fs::create_dir_all(&audio_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture audio directory: {error}"),
    })?;

    Ok(planner
        .microphone_reconnect_file(runtime.current_segment_index, now_unix_ms())
        .to_string_lossy()
        .to_string())
}

/// A paused microphone is left to the tick, which reopens it once the device is
/// back ([`super::microphone_away::reopen_unwakeable_microphone`]).
#[cfg(target_os = "macos")]
pub fn should_reconnect_waiting_microphone_session(
    runtime: &NativeCaptureRuntime,
    state: &MicrophoneControllerState,
) -> bool {
    super::microphone_away::microphone_free_to_record(runtime)
        && !runtime.inactivity.is_microphone_paused()
        && runtime.active_microphone_session.is_none()
        && state.preference.mode == MicrophonePreferenceMode::SpecificDevice
        && state.disconnect_policy == MicrophoneDisconnectPolicy::WaitForSameDevice
        && state.preference.device_id.is_some()
        && state.effective_device.is_some()
}

/// Start a fresh microphone session for a still-running capture runtime and wire
/// its output file into the current segment.
///
/// The one microphone restart path: used both when a waited-for device reappears
/// and when a bounded out-of-band recording (voice enrollment) hands the
/// microphone back. Screen and the system-audio tap are untouched — neither
/// shares the device.
#[cfg(target_os = "macos")]
pub fn restart_microphone_session_for_runtime(
    runtime: &mut NativeCaptureRuntime,
    device_id: Option<&str>,
    reason: &str,
) -> Result<(), CaptureErrorResponse> {
    ensure_microphone_planner_for_runtime(runtime, reason)?;
    refresh_runtime_planner_dates(runtime);

    let microphone_recording_file = next_microphone_output_file_for_runtime(runtime)?;
    let session =
        microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
            &microphone_recording_file,
            device_id,
        )?;

    runtime.active_microphone_session = Some(session);
    runtime.microphone_recording_file = Some(microphone_recording_file.clone());
    if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
        set_current_microphone_output_file(output_files, microphone_recording_file);
    }

    Ok(())
}

pub fn resolve_capture_microphone_device_id(state: &MicrophoneControllerState) -> Option<String> {
    state.effective_device.as_ref().and_then(|device| {
        if device.is_default {
            None
        } else {
            Some(device.id.clone())
        }
    })
}

pub mod bounded_clip;
pub mod controller;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_capture_inactivity::InactivityState;
    use capture_runtime::{RuntimeController, RuntimeState, SegmentPlanner};
    use capture_types::{CaptureOutputFiles, ScreenResolution, ScreenResolutionPreset};

    #[cfg(target_os = "macos")]
    #[test]
    fn should_reconnect_waiting_microphone_session_when_device_returns() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            current_segment_sources: None,
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                screen_files: vec!["/tmp/screen.mov".to_string()],
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 5.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
            microphone_planner: None,
            system_audio_planner: None,
            frame_artifact_tx: None,
            recording_file: Some("/tmp/screen.mov".to_string()),
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
            active_system_audio_session: None,
            capture_suspension: None,
            #[cfg(target_os = "macos")]
            free_space_probe: None,
            away_probe: None,
            test_microphone_device: None,
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
            user_capture_paused: false,
            microphone_away_hold: false,
            microphone_waiting_for_device: false,
            microphone_lent_out: false,
            microphone_vad: Default::default(),
            source_sessions: None,
            recording_started_at_unix_ms: None,
        };
        let state = MicrophoneControllerState {
            devices: vec![capture_types::MicrophoneDevice {
                id: "mic-1".to_string(),
                name: "Mic 1".to_string(),
                is_default: false,
            }],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: Some(capture_types::MicrophoneDevice {
                id: "mic-1".to_string(),
                name: "Mic 1".to_string(),
                is_default: false,
            }),
        };

        assert!(should_reconnect_waiting_microphone_session(
            &runtime, &state
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_not_reconnect_waiting_microphone_session_while_device_missing() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            }),
            current_segment_sources: None,
            output_files: Some(CaptureOutputFiles {
                screen_file: None,
                screen_files: Vec::new(),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 5.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
            microphone_planner: None,
            system_audio_planner: None,
            frame_artifact_tx: None,
            recording_file: None,
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
            active_system_audio_session: None,
            capture_suspension: None,
            #[cfg(target_os = "macos")]
            free_space_probe: None,
            away_probe: None,
            test_microphone_device: None,
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
            user_capture_paused: false,
            microphone_away_hold: false,
            microphone_waiting_for_device: false,
            microphone_lent_out: false,
            microphone_vad: Default::default(),
            source_sessions: None,
            recording_started_at_unix_ms: None,
        };
        let state = MicrophoneControllerState {
            devices: vec![],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: None,
        };

        assert!(!should_reconnect_waiting_microphone_session(
            &runtime, &state
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_move_microphone_capture_to_waiting_state_when_selected_device_missing() {
        let state = MicrophoneControllerState {
            devices: vec![],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: None,
        };

        assert!(should_move_microphone_capture_to_waiting_state(
            true,
            Some(&CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            true,
            &state,
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn next_microphone_output_file_for_runtime_uses_flat_audio_session_directory() {
        let save_root_dir = std::env::temp_dir()
            .join("native-capture-microphone-path-tests")
            .to_string_lossy()
            .to_string();
        let runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            current_segment_sources: None,
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/finalized-screen/screen.mov".to_string()),
                screen_files: vec!["/tmp/finalized-screen/screen.mov".to_string()],
                microphone_file: Some("/tmp/finalized-screen/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/finalized-screen/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/current-screen/screen.mov".to_string()),
                screen_files: vec!["/tmp/current-screen/screen.mov".to_string()],
                microphone_file: Some("/tmp/current-screen/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/current-screen/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_index: 3,
            screen_frame_rate: 5.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: Some(SegmentPlanner::with_date_prefix(
                save_root_dir.clone(),
                "session-1",
                "2026/04/16",
            )),
            microphone_planner: Some(SegmentPlanner::with_date_prefix(
                save_root_dir.clone(),
                "session-1",
                "2026/04/16",
            )),
            system_audio_planner: None,
            frame_artifact_tx: None,
            recording_file: Some("/tmp/current-screen/screen.mov".to_string()),
            microphone_recording_file: Some("/tmp/current-screen/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
            active_system_audio_session: None,
            capture_suspension: None,
            #[cfg(target_os = "macos")]
            free_space_probe: None,
            away_probe: None,
            test_microphone_device: None,
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
            user_capture_paused: false,
            microphone_away_hold: false,
            microphone_waiting_for_device: false,
            microphone_lent_out: false,
            microphone_vad: Default::default(),
            source_sessions: None,
            recording_started_at_unix_ms: None,
        };

        let path = next_microphone_output_file_for_runtime(&runtime)
            .expect("should build next microphone segment path");
        let output_path = std::path::PathBuf::from(&path);
        let expected_audio_dir = std::path::Path::new(&save_root_dir).join("2026/04/16/audio");

        assert_eq!(output_path.parent(), Some(expected_audio_dir.as_path()));
        assert!(output_path
            .file_name()
            .expect("microphone reconnect path should have file name")
            .to_string_lossy()
            .starts_with("session-1-segment-0003-"));
        assert!(path.ends_with(".m4a"));
        assert!(!path.starts_with("/tmp/current-screen/"));
        assert!(!path.starts_with("/tmp/finalized-screen/"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn next_microphone_output_file_for_runtime_requires_segment_planner() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/finalized-screen/screen.mov".to_string()),
                screen_files: vec!["/tmp/finalized-screen/screen.mov".to_string()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/current-screen/screen.mov".to_string()),
                screen_files: vec!["/tmp/current-screen/screen.mov".to_string()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_index: 1,
            recording_file: Some("/tmp/current-screen/screen.mov".to_string()),
            ..Default::default()
        };

        let error = next_microphone_output_file_for_runtime(&runtime)
            .expect_err("planner should be required for microphone reconnect path planning");

        assert_eq!(error.code, "invalid_runtime_state");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn next_microphone_output_file_for_runtime_uses_microphone_planner_session() {
        let save_root_dir = "/tmp/native-capture-independent-source-sessions".to_string();
        let runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            }),
            current_segment_index: 3,
            segment_planner: Some(SegmentPlanner::with_date_prefix(
                &save_root_dir,
                "screen-session",
                "2026/04/16",
            )),
            microphone_planner: Some(SegmentPlanner::with_date_prefix(
                &save_root_dir,
                "microphone-session",
                "2026/04/16",
            )),
            system_audio_planner: Some(SegmentPlanner::with_date_prefix(
                &save_root_dir,
                "system-audio-session",
                "2026/04/16",
            )),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/finalized-screen/screen.mov".to_string()),
                screen_files: vec!["/tmp/finalized-screen/screen.mov".to_string()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/current-screen/screen.mov".to_string()),
                screen_files: vec!["/tmp/current-screen/screen.mov".to_string()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            recording_file: Some("/tmp/current-screen/screen.mov".to_string()),
            ..Default::default()
        };

        let path = next_microphone_output_file_for_runtime(&runtime)
            .expect("should build next microphone segment path");
        let output_path = std::path::PathBuf::from(&path);
        let expected_audio_dir = std::path::Path::new(&save_root_dir).join("2026/04/16/audio");

        assert_eq!(output_path.parent(), Some(expected_audio_dir.as_path()));
        assert!(path.contains("audio/microphone-session-segment-0003-"));
        assert!(!path.contains("screen-session-segment"));
        assert!(!path.contains("system-audio-session-segment"));
    }

    #[test]
    fn microphone_auto_disconnect_transition_failed_event_has_expected_payload() {
        let error = CaptureErrorResponse {
            code: "microphone_stop_failed".to_string(),
            message: "stop failed".to_string(),
        };

        let payload = microphone_auto_disconnect_transition_failed_event(&error);

        assert_eq!(payload.context, "stop_before_wait_for_same_device");
        assert_eq!(payload.code, "microphone_stop_failed");
        assert_eq!(payload.message, "stop failed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_waited_for_device_does_not_reconnect_a_lent_out_microphone() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            }),
            microphone_lent_out: true,
            ..Default::default()
        };
        let device = capture_types::MicrophoneDevice {
            id: "mic-1".to_string(),
            name: "Mic 1".to_string(),
            is_default: false,
        };
        let state = MicrophoneControllerState {
            devices: vec![device.clone()],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: Some(device),
        };

        assert!(!should_reconnect_waiting_microphone_session(
            &runtime, &state
        ));
    }

    // The emits that used to go straight to the Tauri handle reach the host as
    // typed core events carrying the same payloads.
    #[test]
    fn microphone_emits_reach_the_host_as_typed_core_events() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = std::sync::Arc::new(crate::test_support::FakeHost::default());
        let core = crate::test_support::core_in(host.clone(), dir.path());
        let state = MicrophoneControllerState {
            devices: vec![],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::Default,
                device_id: None,
            },
            disconnect_policy: MicrophoneDisconnectPolicy::FallbackToDefault,
            effective_device: None,
        };
        let error = CaptureErrorResponse {
            code: "microphone_stop_failed".to_string(),
            message: "stop failed".to_string(),
        };

        emit_microphone_controller_changed(&core, state.clone());
        emit_microphone_auto_disconnect_transition_failed(&core, &error);

        assert_eq!(
            *host.events.lock().unwrap(),
            vec![
                crate::CoreEvent::MicrophoneControllerChanged(state),
                crate::CoreEvent::MicrophoneAutoDisconnectTransitionFailed(
                    microphone_auto_disconnect_transition_failed_event(&error)
                ),
            ]
        );
    }
}
