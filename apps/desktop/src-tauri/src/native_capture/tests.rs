use super::activity::{
    current_activity_snapshot, idle_debug_activity_sources, lock_runtime_for_idle_debug,
};
use super::describe_recording_settings_changes;
use super::microphone::microphone_auto_disconnect_transition_failed_event;
#[cfg(target_os = "macos")]
use super::microphone::{
    next_microphone_output_file_for_runtime, should_move_microphone_capture_to_waiting_state,
    should_reconnect_waiting_microphone_session,
};
use super::runtime::{
    mark_runtime_session_stopped, should_recover_from_segment_finalize_error,
    should_rotate_segment, stopped_session_from_runtime, validate_start_request,
    NativeCaptureRuntime,
};
use super::segments::{
    flush_frame_artifacts, try_forward_frame_artifact, FrameArtifactForwardingResult,
    FrameArtifactMessage,
};
#[cfg(target_os = "macos")]
use super::segments::{
    handle_inactivity_resume_error, resume_runtime_from_inactivity_with_start_segment,
    StartedSegmentState,
};
use crate::native_capture_inactivity::{ActivityPolicyEvaluation, InactivityState};
use crate::native_capture_output::set_current_microphone_output_file;
use crate::native_capture_settings::{
    compute_effective_screen_bitrate_bps, validate_recording_settings,
    validate_recording_settings_with_resolution_support,
};
#[cfg(target_os = "macos")]
use capture_runtime::{CaptureClock, RuntimeSignal, SegmentPlanner, SegmentSchedule};
use capture_runtime::{RuntimeController, RuntimeState};
use capture_types::{
    default_inactivity_activity_mode, default_video_bitrate, CaptureErrorResponse,
    CaptureOutputFiles, CaptureSources, CaptureSupportResponse, InactivityActivityMode,
    MicrophoneControllerState, MicrophoneDisconnectPolicy, MicrophonePreference,
    MicrophonePreferenceMode, RecordingSettings, ScreenResolution, ScreenResolutionPreset,
    StartNativeCaptureRequest, UpdateRecordingSettingsRequest, VideoBitrateMode,
    VideoBitratePreset, VideoBitrateSettings,
};
use tokio::sync::mpsc;

fn recording_settings_fixture() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    }
}

#[cfg(target_os = "macos")]
fn running_runtime_controller() -> RuntimeController {
    let mut controller = RuntimeController::default();
    controller
        .apply(RuntimeSignal::StartRequested)
        .expect("idle runtime should start");
    controller
        .apply(RuntimeSignal::SourcesReady)
        .expect("starting runtime should become running");
    controller
}

#[cfg(target_os = "macos")]
fn paused_runtime_fixture() -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-resume",
        )),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
fn resumed_segment_state_fixture(screen_file: String) -> StartedSegmentState {
    (
        CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        },
        Some(screen_file),
        None,
        None,
        None,
        None,
    )
}

#[test]
fn validate_start_request_rejects_system_audio_when_not_supported() {
    let request = StartNativeCaptureRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: true,
    };
    let support = CaptureSupportResponse {
        platform: "macos".to_string(),
        native_capture_supported: true,
        supported_sources: CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        },
    };

    let error = validate_start_request(&request, &support).expect_err("must reject system audio");
    assert_eq!(error.code, "system_audio_unsupported");
}

#[test]
fn describe_recording_settings_changes_returns_empty_for_identical_settings() {
    let settings = recording_settings_fixture();

    assert!(describe_recording_settings_changes(&settings, &settings).is_empty());
}

#[test]
fn describe_recording_settings_changes_lists_high_signal_differences() {
    let previous = recording_settings_fixture();
    let next = RecordingSettings {
        capture_microphone: true,
        save_directory: "/tmp/updated".to_string(),
        auto_start: true,
        native_capture_debug_logging_enabled: true,
        segment_duration_seconds: 120,
        screen_frame_rate: 24,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P720,
        },
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Custom,
            preset: None,
            custom_mbps: Some(8),
        },
        pause_capture_on_inactivity: false,
        idle_timeout_seconds: 30,
        audio_activity_sensitivity: 75,
        inactivity_activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
        ..previous.clone()
    };

    let changes = describe_recording_settings_changes(&previous, &next);

    assert!(changes.contains(&"sources screen=true, microphone=false, system_audio=false -> screen=true, microphone=true, system_audio=false".to_string()));
    assert!(!changes
        .iter()
        .any(|change| change.contains("save_directory")));
    assert!(changes.contains(&"auto_start false -> true".to_string()));
    assert!(changes.contains(&"debug_logging false -> true".to_string()));
    assert!(changes.contains(&"segment_duration_seconds 60 -> 120".to_string()));
    assert!(changes.contains(&"screen_frame_rate 30 -> 24".to_string()));
    assert!(changes.contains(&"screen_resolution original -> 720p".to_string()));
    assert!(changes.contains(&"video_bitrate preset:medium -> custom:8mbps".to_string()));
    assert!(changes.contains(&"pause_on_inactivity true -> false".to_string()));
    assert!(changes.contains(&"idle_timeout_seconds 10 -> 30".to_string()));
    assert!(changes.contains(&"audio_activity_sensitivity 50 -> 75".to_string()));
    assert!(changes.contains(
        &"activity_mode system_input_or_screen -> system_input_or_screen_or_audio".to_string()
    ));
}

#[test]
fn validate_recording_settings_rejects_all_sources_disabled() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: false,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("all sources disabled must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(error.message, "At least one capture source must be enabled");
}

#[test]
fn validate_recording_settings_rejects_system_audio_without_screen() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: false,
        capture_microphone: true,
        capture_system_audio: true,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("system audio without screen must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        "System audio capture requires screen capture"
    );
}

#[test]
fn validate_recording_settings_allows_storing_resolution_when_screen_disabled() {
    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: false,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Custom {
                width: 1280,
                height: 720,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            native_capture_debug_logging_enabled: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("resolution settings should still be storable");

    assert_eq!(
        settings.screen_resolution,
        ScreenResolution::Custom {
            width: 1280,
            height: 720,
        }
    );
}

#[test]
fn validate_recording_settings_allows_non_original_resolution_when_screen_disabled_on_fallback_backend(
) {
    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: false,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            native_capture_debug_logging_enabled: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        false,
    )
    .expect("resolution should be allowed when screen capture is disabled");

    assert_eq!(
        settings.screen_resolution,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P720,
        }
    );
}

#[test]
fn validate_recording_settings_rejects_non_original_resolution_when_screen_enabled_on_fallback_backend(
) {
    let error = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            native_capture_debug_logging_enabled: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        false,
    )
    .expect_err("fallback backend must reject non-original resolution when screen is enabled");

    assert_eq!(error.code, "screen_resolution_unsupported");
}

#[test]
fn validate_recording_settings_rejects_too_small_custom_resolution() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Custom {
            width: 8,
            height: 8,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("too small resolution should be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
}

#[test]
fn validate_recording_settings_defaults_preset_bitrate_when_preset_value_missing() {
    let settings = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Preset,
            preset: None,
            custom_mbps: Some(12),
        },
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect("preset mode should normalize bitrate values");

    assert_eq!(settings.video_bitrate.mode, VideoBitrateMode::Preset);
    assert_eq!(
        settings.video_bitrate.preset,
        Some(VideoBitratePreset::Medium)
    );
    assert_eq!(settings.video_bitrate.custom_mbps, None);
}

#[test]
fn validate_recording_settings_rejects_custom_bitrate_out_of_range() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Custom,
            preset: Some(VideoBitratePreset::High),
            custom_mbps: Some(41),
        },
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("custom bitrate above max should be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        "videoBitrate.customMbps must be between 1 and 40"
    );
}

#[test]
fn validate_recording_settings_accepts_audio_activity_mode_and_sensitivity() {
    let settings = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: true,
        capture_system_audio: true,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 75,
        inactivity_activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
    })
    .expect("audio-aware inactivity settings should be valid");

    assert_eq!(settings.audio_activity_sensitivity, 75);
    assert_eq!(
        settings.inactivity_activity_mode,
        InactivityActivityMode::SystemInputOrScreenOrAudio
    );
}

#[test]
fn validate_recording_settings_preserves_native_capture_debug_logging_flag() {
    let settings = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: true,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect("debug logging flag should round-trip through validation");

    assert!(settings.native_capture_debug_logging_enabled);
}

#[test]
fn validate_recording_settings_rejects_audio_activity_sensitivity_above_max() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 101,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("sensitivity above max must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        "audioActivitySensitivity must be between 0 and 100"
    );
}

#[test]
fn compute_effective_screen_bitrate_uses_preset_formula() {
    let settings = RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P720,
        },
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Preset,
            preset: Some(VideoBitratePreset::Medium),
            custom_mbps: None,
        },
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    };

    let bitrate = compute_effective_screen_bitrate_bps(&settings)
        .expect("screen capture should produce a bitrate");

    assert_eq!(bitrate, 2_750_000);
}

#[test]
fn compute_effective_screen_bitrate_uses_custom_value() {
    let settings = RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Custom,
            preset: Some(VideoBitratePreset::Low),
            custom_mbps: Some(7),
        },
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    };

    let bitrate = compute_effective_screen_bitrate_bps(&settings)
        .expect("screen capture should produce a bitrate");

    assert_eq!(bitrate, 7_000_000);
}

#[test]
fn compute_effective_screen_bitrate_none_when_screen_disabled() {
    let settings = RecordingSettings {
        capture_screen: false,
        capture_microphone: true,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P1080,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        native_capture_debug_logging_enabled: false,
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        audio_activity_sensitivity: 50,
        inactivity_activity_mode: default_inactivity_activity_mode(),
    };

    assert_eq!(compute_effective_screen_bitrate_bps(&settings), None);
}

#[test]
fn mark_runtime_session_stopped_preserves_session_metadata() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        session_id: Some("session-1".to_string()),
        started_at_unix_ms: Some(123),
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
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
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        frame_artifact_tx: None,
        #[cfg(target_os = "macos")]
        recording_file: Some("/tmp/screen.mov".to_string()),
        #[cfg(target_os = "macos")]
        microphone_recording_file: Some("/tmp/microphone.mov".to_string()),
        #[cfg(target_os = "macos")]
        system_audio_recording_file: None,
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(!runtime.is_running);
    assert_eq!(runtime.session_id, Some("session-1".to_string()));
    assert_eq!(runtime.started_at_unix_ms, Some(123));
    assert!(runtime.requested_sources.is_some());
    assert!(runtime.output_files.is_some());
    assert!(runtime.frame_artifact_tx.is_none());
}

#[test]
fn stopped_session_from_runtime_preserves_finalized_metadata() {
    let runtime = NativeCaptureRuntime {
        is_running: true,
        session_id: Some("session-1".to_string()),
        started_at_unix_ms: Some(123),
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        current_segment_output_files: None,
        current_segment_index: 1,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        frame_artifact_tx: None,
        #[cfg(target_os = "macos")]
        recording_file: None,
        #[cfg(target_os = "macos")]
        microphone_recording_file: None,
        #[cfg(target_os = "macos")]
        system_audio_recording_file: None,
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
    };

    let session = stopped_session_from_runtime(&runtime);

    assert!(!session.is_running);
    assert_eq!(session.session_id, Some("session-1".to_string()));
    assert_eq!(session.started_at_unix_ms, Some(123));
    assert!(session
        .requested_sources
        .as_ref()
        .is_some_and(|sources| { sources.screen && sources.microphone && sources.system_audio }));
}

#[test]
fn current_activity_snapshot_marks_audio_sources_enabled_from_requested_sources() {
    let runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        ..Default::default()
    };

    let snapshot = current_activity_snapshot(&runtime);

    assert!(snapshot.microphone_activity.enabled);
    assert!(snapshot.system_audio_activity.enabled);
}

#[test]
fn idle_debug_activity_sources_include_audio_fields() {
    let policy = ActivityPolicyEvaluation {
        effective_idle: crate::native_capture_inactivity::EffectiveIdle {
            source: crate::native_capture_inactivity::ActivitySourceKind::MicrophoneCapture,
            idle_ms: 250,
        },
        sources: vec![crate::native_capture_inactivity::ActivitySourceSample {
            kind: crate::native_capture_inactivity::ActivitySourceKind::MicrophoneCapture,
            enabled: true,
            available: true,
            idle_ms: Some(250),
            latest_normalized_level: Some(0.35),
            activity_threshold: Some(0.4),
        }],
    };

    let sources = idle_debug_activity_sources(&policy);

    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].kind, "microphone_capture");
    assert!(sources[0].enabled);
    assert!(sources[0].available);
    assert_eq!(sources[0].idle_ms, Some(250));
    assert_eq!(sources[0].latest_normalized_level, Some(0.35));
    assert_eq!(sources[0].activity_threshold, Some(0.4));
    assert!(sources[0].selected);
}

#[test]
fn lock_runtime_for_idle_debug_recovers_poisoned_state() {
    let state = std::sync::Mutex::new(NativeCaptureRuntime {
        is_running: true,
        session_id: Some("session-1".to_string()),
        ..Default::default()
    });

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _runtime = state.lock().expect("state should lock before poisoning");
        panic!("poison native capture state");
    }));

    assert!(state.is_poisoned());

    let runtime = lock_runtime_for_idle_debug(&state);

    assert!(runtime.is_running);
    assert_eq!(runtime.session_id.as_deref(), Some("session-1"));
}

#[cfg(target_os = "macos")]
#[test]
fn should_reconnect_waiting_microphone_session_when_device_returns() {
    let runtime = NativeCaptureRuntime {
        is_running: true,
        session_id: Some("session-1".to_string()),
        started_at_unix_ms: Some(123),
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
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
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        frame_artifact_tx: None,
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
        system_audio_recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
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
        session_id: Some("session-1".to_string()),
        started_at_unix_ms: Some(123),
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
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
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        frame_artifact_tx: None,
        recording_file: None,
        microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
        system_audio_recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
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
fn next_microphone_output_file_for_runtime_uses_new_segment_name() {
    let runtime = NativeCaptureRuntime {
        is_running: true,
        session_id: Some("session-1".to_string()),
        started_at_unix_ms: Some(123),
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
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
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        frame_artifact_tx: None,
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
        system_audio_recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
    };

    let path = next_microphone_output_file_for_runtime(&runtime)
        .expect("should build next microphone segment path");

    assert!(path.starts_with("/tmp/microphone-"));
    assert!(path.ends_with(".m4a"));
    assert_ne!(path, "/tmp/microphone.m4a");
}

#[test]
fn set_current_microphone_output_file_tracks_all_segments() {
    let mut output_files = CaptureOutputFiles {
        screen_file: Some("/tmp/screen.mov".to_string()),
        screen_files: vec!["/tmp/screen.mov".to_string()],
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    set_current_microphone_output_file(&mut output_files, "/tmp/microphone-1.m4a".to_string());
    set_current_microphone_output_file(&mut output_files, "/tmp/microphone-2.m4a".to_string());

    assert_eq!(
        output_files.microphone_file,
        Some("/tmp/microphone-2.m4a".to_string())
    );
    assert_eq!(
        output_files.microphone_files,
        vec![
            "/tmp/microphone-1.m4a".to_string(),
            "/tmp/microphone-2.m4a".to_string()
        ]
    );
}

#[test]
fn should_rotate_segment_only_after_boundary_crossing() {
    assert!(!should_rotate_segment(1, 1));
    assert!(should_rotate_segment(1, 2));
    assert!(should_rotate_segment(3, 5));
}

#[test]
fn should_recover_from_segment_finalize_error_accepts_wrapped_missing_screen_output() {
    let error = capture_writers::aggregate_output_processing_failures(vec![
        "screen output missing: expected screen recording file at /tmp/screen.mov".to_string(),
    ])
    .expect_err("single missing screen output failure should aggregate");

    assert!(should_recover_from_segment_finalize_error(&error));
}

#[test]
fn should_recover_from_segment_finalize_error_rejects_missing_screen_output_with_extra_failures() {
    let error = capture_writers::aggregate_output_processing_failures(vec![
        "screen output missing: expected screen recording file at /tmp/screen.mov".to_string(),
        "system audio output conversion failed: missing source recording".to_string(),
    ])
    .expect_err("multiple output failures should aggregate");

    assert!(!should_recover_from_segment_finalize_error(&error));
}

#[test]
fn should_recover_from_segment_finalize_error_accepts_missing_screen_output_without_path() {
    let error = capture_writers::aggregate_output_processing_failures(vec![
        "screen output missing: expected screen recording file".to_string(),
    ])
    .expect_err("single missing screen output failure should aggregate");

    assert!(should_recover_from_segment_finalize_error(&error));
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

#[test]
fn mark_runtime_session_stopped_clears_frame_artifact_worker() {
    let (tx, _rx) = mpsc::channel::<FrameArtifactMessage>(1);
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        frame_artifact_tx: Some(tx),
        ..Default::default()
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(runtime.frame_artifact_tx.is_none());
}

#[test]
fn try_forward_frame_artifact_enqueues_when_capacity_available() {
    let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(1);

    let result = try_forward_frame_artifact(
        &tx,
        capture_screen::ScreenFrameArtifact {
            file_path: "/tmp/frame-1.png".to_string(),
            captured_at_unix_ms: 1,
            width: Some(100),
            height: Some(100),
            content_fingerprint: Some(7),
        },
    );

    assert_eq!(result, FrameArtifactForwardingResult::Enqueued);

    let queued = rx
        .try_recv()
        .expect("frame should be queued")
        .unwrap_artifact();
    assert_eq!(queued.file_path, "/tmp/frame-1.png");
}

#[test]
fn try_forward_frame_artifact_enqueues_multiple_frames_without_dropping() {
    let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(2);

    let first = try_forward_frame_artifact(
        &tx,
        capture_screen::ScreenFrameArtifact {
            file_path: "/tmp/frame-1.png".to_string(),
            captured_at_unix_ms: 1,
            width: None,
            height: None,
            content_fingerprint: None,
        },
    );
    let second = try_forward_frame_artifact(
        &tx,
        capture_screen::ScreenFrameArtifact {
            file_path: "/tmp/frame-2.png".to_string(),
            captured_at_unix_ms: 2,
            width: None,
            height: None,
            content_fingerprint: None,
        },
    );

    assert_eq!(first, FrameArtifactForwardingResult::Enqueued);
    assert_eq!(second, FrameArtifactForwardingResult::Enqueued);

    let first = rx
        .try_recv()
        .expect("first frame should reach worker queue")
        .unwrap_artifact();
    let second = rx
        .try_recv()
        .expect("second frame should reach worker queue")
        .unwrap_artifact();
    assert_eq!(first.file_path, "/tmp/frame-1.png");
    assert_eq!(second.file_path, "/tmp/frame-2.png");
}

#[test]
fn try_forward_frame_artifact_waits_for_capacity_without_dropping_frames() {
    let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(1);

    let first = try_forward_frame_artifact(
        &tx,
        capture_screen::ScreenFrameArtifact {
            file_path: "/tmp/frame-1.png".to_string(),
            captured_at_unix_ms: 1,
            width: None,
            height: None,
            content_fingerprint: None,
        },
    );

    assert_eq!(first, FrameArtifactForwardingResult::Enqueued);

    let sender = std::thread::spawn(move || {
        try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-2.png".to_string(),
                captured_at_unix_ms: 2,
                width: None,
                height: None,
                content_fingerprint: None,
            },
        )
    });

    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !sender.is_finished(),
        "second send should wait for queue capacity"
    );

    let queued = tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("first frame should arrive before timeout")
            .expect("first frame should be queued")
    })
    .unwrap_artifact();
    assert_eq!(queued.file_path, "/tmp/frame-1.png");

    let second = sender.join().expect("sender thread should exit cleanly");
    assert_eq!(second, FrameArtifactForwardingResult::Enqueued);

    let deferred = tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("second frame should arrive before timeout")
            .expect("second frame should be queued")
    })
    .unwrap_artifact();
    assert_eq!(deferred.file_path, "/tmp/frame-2.png");
}

#[test]
fn try_forward_frame_artifact_reports_closed_receiver() {
    let (tx, rx) = mpsc::channel::<FrameArtifactMessage>(1);
    drop(rx);

    let result = try_forward_frame_artifact(
        &tx,
        capture_screen::ScreenFrameArtifact {
            file_path: "/tmp/frame-1.png".to_string(),
            captured_at_unix_ms: 1,
            width: None,
            height: None,
            content_fingerprint: None,
        },
    );

    assert_eq!(result, FrameArtifactForwardingResult::ReceiverClosed);
}

#[test]
fn flush_frame_artifacts_waits_for_all_queued_items() {
    let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(4);

    // Enqueue two frame artifacts before the flush.
    for i in 1..=2 {
        tx.try_send(FrameArtifactMessage::Artifact(
            capture_screen::ScreenFrameArtifact {
                file_path: format!("/tmp/frame-{i}.png"),
                captured_at_unix_ms: i,
                width: None,
                height: None,
                content_fingerprint: None,
            },
        ))
        .expect("channel should have capacity");
    }

    // Track which artifacts the consumer sees before the flush ack.
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_for_consumer = std::sync::Arc::clone(&seen);

    // Consumer task: drain messages, record artifacts, ack flushes.
    let consumer = std::thread::spawn(move || {
        tauri::async_runtime::block_on(async {
            while let Some(message) = rx.recv().await {
                match message {
                    FrameArtifactMessage::Artifact(artifact) => {
                        // Simulate some processing latency.
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        seen_for_consumer
                            .lock()
                            .expect("seen state should lock")
                            .push(artifact.file_path);
                    }
                    FrameArtifactMessage::Flush(response_tx) => {
                        let _ = response_tx.send(());
                    }
                }
            }
        });
    });

    // Flush from another thread (simulating the segment loop / stop path).
    flush_frame_artifacts(&tx);

    // After flush returns, all prior artifacts must have been processed.
    let processed = seen.lock().expect("seen state should lock").clone();
    assert_eq!(
        processed,
        vec![
            "/tmp/frame-1.png".to_string(),
            "/tmp/frame-2.png".to_string()
        ],
        "flush must drain all artifacts enqueued before the barrier"
    );

    // Drop the sender so the consumer exits.
    drop(tx);
    consumer
        .join()
        .expect("consumer thread should exit cleanly");
}

#[test]
fn flush_frame_artifacts_is_noop_when_channel_closed() {
    let (tx, rx) = mpsc::channel::<FrameArtifactMessage>(1);
    drop(rx);

    // Must not hang or panic when the receiver is already gone.
    flush_frame_artifacts(&tx);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_transient_failure_keeps_runtime_paused_for_retry() {
    let mut runtime = paused_runtime_fixture();

    let error =
        resume_runtime_from_inactivity_with_start_segment(&mut runtime, |_, _, _, _, _, _, _| {
            Err(CaptureErrorResponse {
                code: "capture_stream_start_failed".to_string(),
                message: "temporary startup failure".to_string(),
            })
        })
        .expect_err("transient resume failure should bubble to retry handler");

    assert!(!handle_inactivity_resume_error(&mut runtime, error));
    assert!(runtime.is_running);
    assert_eq!(runtime.runtime_state, RuntimeState::Running);
    assert!(runtime.inactivity.is_paused);
    assert_eq!(runtime.current_segment_index, 1);
    assert!(runtime.current_segment_output_files.is_none());
    assert!(runtime.recording_file.is_none());
    assert!(runtime.segment_planner.is_some());
    assert!(runtime.segment_schedule.is_some());
    assert!(runtime.capture_clock.is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_retry_success_clears_paused_state_and_restores_segment_outputs() {
    let mut runtime = paused_runtime_fixture();

    let error =
        resume_runtime_from_inactivity_with_start_segment(&mut runtime, |_, _, _, _, _, _, _| {
            Err(CaptureErrorResponse {
                code: "capture_stream_start_failed".to_string(),
                message: "temporary startup failure".to_string(),
            })
        })
        .expect_err("first resume attempt should fail transiently");
    assert!(!handle_inactivity_resume_error(&mut runtime, error));

    let expected_screen_file =
        "/tmp/native-capture-tests/native-session-resume-segment-0002/screen.mov".to_string();

    resume_runtime_from_inactivity_with_start_segment(
        &mut runtime,
        |segment_dir, sources, frame_rate, resolution, bitrate, microphone_device_id, frame_tx| {
            assert_eq!(
                sources,
                &CaptureSources {
                    screen: true,
                    microphone: false,
                    system_audio: false,
                }
            );
            assert_eq!(frame_rate, 30);
            assert_eq!(resolution, &ScreenResolution::default());
            assert_eq!(bitrate, None);
            assert_eq!(microphone_device_id, None);
            assert!(frame_tx.is_none());
            assert_eq!(
                segment_dir.file_name().and_then(|name| name.to_str()),
                Some("native-session-resume-segment-0002")
            );

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("later resume retry should succeed");

    assert!(runtime.is_running);
    assert_eq!(runtime.runtime_state, RuntimeState::Running);
    assert!(!runtime.inactivity.is_paused);
    assert_eq!(runtime.current_segment_index, 2);
    assert_eq!(runtime.recording_file, Some(expected_screen_file.clone()));
    let segment_outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("resume should restore current segment outputs");
    let expected_screen_files = vec![expected_screen_file.clone()];
    assert_eq!(segment_outputs.screen_file, Some(expected_screen_file));
    assert_eq!(&segment_outputs.screen_files, &expected_screen_files);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_invalid_runtime_state_marks_runtime_failed() {
    let mut runtime = paused_runtime_fixture();
    runtime.segment_planner = None;

    let error =
        resume_runtime_from_inactivity_with_start_segment(&mut runtime, |_, _, _, _, _, _, _| {
            unreachable!("invalid runtime state should fail before restart")
        })
        .expect_err("missing planner should fail loudly");

    assert!(handle_inactivity_resume_error(&mut runtime, error));
    assert!(!runtime.is_running);
    assert_eq!(runtime.runtime_state, RuntimeState::Failed);
    assert!(!runtime.inactivity.is_paused);
}
