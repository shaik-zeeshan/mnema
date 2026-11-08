use super::*;

#[test]
fn validate_recording_settings_rejects_all_sources_disabled() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: false,
        capture_microphone: false,
        capture_system_audio: false,
        ..update_recording_settings_request_fixture()
    })
    .expect_err("all sources disabled must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(error.message, "At least one capture source must be enabled");
}

#[test]
fn validate_recording_settings_allows_system_audio_without_screen() {
    let settings = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: false,
        capture_microphone: false,
        capture_system_audio: true,
        ..update_recording_settings_request_fixture()
    })
    .expect("system audio runs on its own tap, so it needs no screen capture");

    assert!(!settings.capture_screen);
    assert!(!settings.capture_microphone);
    assert!(settings.capture_system_audio);
}

#[test]
fn validate_recording_settings_allows_storing_resolution_when_screen_disabled() {
    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: false,
            capture_microphone: true,
            capture_system_audio: false,
            screen_resolution: ScreenResolution::Custom {
                width: 1280,
                height: 720,
            },
            ..update_recording_settings_request_fixture()
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
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            ..update_recording_settings_request_fixture()
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
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            ..update_recording_settings_request_fixture()
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
        screen_resolution: ScreenResolution::Custom {
            width: 8,
            height: 8,
        },
        ..update_recording_settings_request_fixture()
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
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Preset,
            preset: None,
            custom_mbps: Some(12),
        },
        ..update_recording_settings_request_fixture()
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
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Custom,
            preset: Some(VideoBitratePreset::High),
            custom_mbps: Some(41),
        },
        ..update_recording_settings_request_fixture()
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
        microphone_activity_sensitivity: 75,
        system_audio_activity_sensitivity: 75,
        inactivity_activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
        ..update_recording_settings_request_fixture()
    })
    .expect("audio-aware inactivity settings should be valid");

    assert_eq!(settings.microphone_activity_sensitivity, 75);
    assert_eq!(settings.system_audio_activity_sensitivity, 75);
    assert_eq!(
        settings.inactivity_activity_mode,
        InactivityActivityMode::SystemInputOrScreenOrAudio
    );
}

#[test]
fn validate_recording_settings_preserves_developer_options_flag() {
    let settings = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        developer_options_enabled: true,
        ..update_recording_settings_request_fixture()
    })
    .expect("developer options flag should round-trip through validation");

    assert!(settings.developer_options_enabled);
}

#[test]
fn validate_recording_settings_rejects_audio_activity_sensitivity_above_max() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        microphone_activity_sensitivity: 101,
        ..update_recording_settings_request_fixture()
    })
    .expect_err("sensitivity above max must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        "microphoneActivitySensitivity must be between 0 and 100"
    );
}

#[test]
fn validate_recording_settings_accepts_frame_rate_bounds() {
    // 1/60 fps = one snapshot per minute, the new floor.
    for rate in [1.0 / 60.0, 0.5, 10.0] {
        validate_recording_settings(UpdateRecordingSettingsRequest {
            screen_frame_rate: rate,
            ..update_recording_settings_request_fixture()
        })
        .unwrap_or_else(|error| panic!("frame rate {rate} must be accepted: {error:?}"));
    }
}

#[test]
fn validate_recording_settings_rejects_out_of_range_frame_rates() {
    for rate in [1.0 / 61.0, 10.1, 0.0, -1.0, 30.0, 120.0, f64::NAN] {
        let error = validate_recording_settings(UpdateRecordingSettingsRequest {
            screen_frame_rate: rate,
            ..update_recording_settings_request_fixture()
        })
        .expect_err(&format!("frame rate {rate} must be rejected"));

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(
            error.message,
            "screenFrameRate must be between 1/60 (one snapshot per minute) and 10"
        );
    }
}

#[test]
fn compute_effective_screen_bitrate_uses_preset_formula() {
    let settings = RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        // 1920 * 1080 * 4.5 * 0.10 = 933,120 rounds to 1,000,000; an integer-truncated
        // rate (4.0) would round to 750,000, so this pins the fractional f64 path.
        screen_frame_rate: 4.5,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P1080,
        },
        video_bitrate: VideoBitrateSettings {
            mode: VideoBitrateMode::Preset,
            preset: Some(VideoBitratePreset::Medium),
            custom_mbps: None,
        },
        save_directory: "/tmp".to_string(),
        auto_start: false,
        developer_options_enabled: false,
        preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
        follow_timeline_live: false,
        retention_policy: default_retention_policy(),
        appearance: default_appearance(),
        time_format: Default::default(),
        ocr: default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: default_speaker_analysis_settings(),
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: AccessSettings::default(),
        ai_runtime: AiRuntimeSettings::default(),
        user_context: UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        microphone_vad_adapter: default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    };

    let bitrate = compute_effective_screen_bitrate_bps(&settings)
        .expect("screen capture should produce a bitrate");

    assert_eq!(bitrate, 1_000_000);
}

#[test]
fn compute_effective_screen_bitrate_uses_custom_value() {
    let settings = RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 5.0,
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
        developer_options_enabled: false,
        preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
        follow_timeline_live: false,
        retention_policy: default_retention_policy(),
        appearance: default_appearance(),
        time_format: Default::default(),
        ocr: default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: default_speaker_analysis_settings(),
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: AccessSettings::default(),
        ai_runtime: AiRuntimeSettings::default(),
        user_context: UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        microphone_vad_adapter: default_microphone_vad_adapter(),
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
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P1080,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        developer_options_enabled: false,
        preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
        follow_timeline_live: false,
        retention_policy: default_retention_policy(),
        appearance: default_appearance(),
        time_format: Default::default(),
        ocr: default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: default_speaker_analysis_settings(),
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: AccessSettings::default(),
        ai_runtime: AiRuntimeSettings::default(),
        user_context: UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        microphone_vad_adapter: default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    };

    assert_eq!(compute_effective_screen_bitrate_bps(&settings), None);
}
