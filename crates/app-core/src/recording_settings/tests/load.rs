use super::*;

#[test]
fn load_recording_settings_from_path_returns_none_for_missing_file() {
    let dir = TestDir::new("missing");

    assert!(load_recording_settings_from_path(&dir.path().join("missing.json")).is_none());
}

#[test]
fn load_recording_settings_from_path_returns_none_for_invalid_file() {
    let dir = TestDir::new("invalid");
    let path = dir.path().join("recording-settings.json");
    fs::write(&path, "not valid json").expect("invalid file should write");

    assert!(load_recording_settings_from_path(&path).is_none());
}

#[test]
fn load_recording_settings_from_path_or_default_uses_defaults_for_missing_file() {
    let dir = TestDir::new("missing-default");

    assert_eq!(
        load_recording_settings_from_path_or_default(&dir.path().join("missing.json"))
            .save_directory,
        default_recording_settings().save_directory
    );
}

#[test]
fn load_recording_settings_from_path_or_default_uses_defaults_for_invalid_file() {
    let dir = TestDir::new("invalid-default");
    let path = dir.path().join("recording-settings.json");
    fs::write(&path, "not valid json").expect("invalid file should write");

    assert_eq!(
        load_recording_settings_from_path_or_default(&path).save_directory,
        default_recording_settings().save_directory
    );
}

#[test]
fn default_recording_settings_disable_ask_ai_access() {
    assert!(!default_recording_settings().access.ask_ai_enabled);
    assert!(!default_recording_settings().access.ask_ai_web_fetch_enabled);
}

#[test]
fn default_recording_settings_disable_developer_options() {
    assert!(!default_recording_settings().developer_options_enabled);
}

#[test]
fn default_recording_settings_use_default_microphone_vad_adapter() {
    assert_eq!(
        default_recording_settings().microphone_vad_adapter,
        capture_types::default_microphone_vad_adapter()
    );
}

#[test]
fn load_recording_settings_from_path_preserves_developer_options_flag() {
    let dir = TestDir::new("developer-options-enabled");
    let path = dir.path().join("recording-settings.json");
    let mut settings = default_recording_settings();
    settings.developer_options_enabled = true;

    fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings should serialize"),
    )
    .expect("settings file should write");

    let loaded = load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert!(loaded.developer_options_enabled);
}

#[test]
fn load_recording_settings_from_path_preserves_ask_ai_access_flag() {
    let dir = TestDir::new("ask-ai-enabled");
    let path = dir.path().join("recording-settings.json");
    let mut settings = default_recording_settings();
    settings.access.ask_ai_enabled = true;

    fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings should serialize"),
    )
    .expect("settings file should write");

    let loaded = load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert!(loaded.access.ask_ai_enabled);
}

#[test]
fn load_recording_settings_from_path_migrates_legacy_microphone_vad_adapter_to_shared_detector() {
    let dir = TestDir::new("microphone-vad-adapter");
    let path = dir.path().join("recording-settings.json");

    fs::write(
        &path,
        r#"{
            "captureScreen": true,
            "captureMicrophone": true,
            "captureSystemAudio": false,
            "segmentDurationSeconds": 60,
            "screenFrameRate": 1,
            "screenResolution": { "mode": "preset", "preset": "original" },
            "videoBitrate": { "mode": "preset", "preset": "medium" },
            "saveDirectory": "/tmp",
            "autoStart": false,
            "pauseCaptureOnInactivity": true,
            "idleTimeoutSeconds": 10,
            "microphoneActivitySensitivity": 50,
            "systemAudioActivitySensitivity": 50,
            "microphoneVadAdapter": "webrtc",
            "activityMode": "system_input_or_screen"
        }"#,
    )
    .expect("settings file should write");

    let loaded = load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert_eq!(
        loaded.audio_speech_detection.detector,
        capture_types::AudioSpeechDetector::Webrtc
    );
    assert_eq!(
        loaded.microphone_vad_adapter,
        capture_types::AudioSpeechDetector::Webrtc
    );
}

#[test]
fn load_recording_settings_from_path_preserves_saved_resolution_when_backend_unsupported() {
    let dir = TestDir::new("non-original-resolution");
    let path = dir.path().join("recording-settings.json");
    let mut settings = default_recording_settings();
    settings.save_directory = "/tmp/custom-mnema".to_string();
    settings.auto_start = true;
    settings.screen_resolution = ScreenResolution::Preset {
        preset: ScreenResolutionPreset::P720,
    };

    fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings should serialize"),
    )
    .expect("settings file should write");

    let loaded = load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert_eq!(loaded.save_directory, "/tmp/custom-mnema");
    assert!(loaded.auto_start);
    assert_eq!(
        loaded.screen_resolution,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P720
        }
    );
}

#[test]
fn load_recording_settings_from_path_migrates_legacy_engine_centric_ai_runtime() {
    // ADR 0034: an existing recording-settings.json with the old
    // engine-centric aiRuntime block loads into the provider-centric shape
    // (deserialization-level migration; the next save writes only the new
    // shape).
    let dir = TestDir::new("legacy-ai-runtime");
    let path = dir.path().join("recording-settings.json");

    fs::write(
        &path,
        r#"{
            "captureScreen": true,
            "captureMicrophone": false,
            "captureSystemAudio": false,
            "segmentDurationSeconds": 60,
            "screenFrameRate": 1,
            "screenResolution": { "mode": "preset", "preset": "original" },
            "videoBitrate": { "mode": "preset", "preset": "medium" },
            "saveDirectory": "/tmp",
            "autoStart": false,
            "pauseCaptureOnInactivity": true,
            "idleTimeoutSeconds": 10,
            "microphoneActivitySensitivity": 50,
            "systemAudioActivitySensitivity": 50,
            "activityMode": "system_input_or_screen",
            "aiRuntime": {
                "enabled": true,
                "engineKind": "cloud",
                "cloudProvider": "anthropic",
                "cloudModel": "claude-haiku-4-5",
                "cloudBaseUrl": "",
                "localKind": "ollama",
                "localEndpoint": "http://localhost:11434",
                "localModel": "",
                "additionalEngines": [
                    {
                        "engineKind": "local",
                        "cloudProvider": "openai",
                        "cloudModel": "",
                        "cloudBaseUrl": "",
                        "localKind": "ollama",
                        "localEndpoint": "http://localhost:11434",
                        "localModel": "llama3.2"
                    }
                ]
            }
        }"#,
    )
    .expect("settings file should write");

    let loaded = load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert!(loaded.ai_runtime.enabled);
    assert_eq!(
        loaded.ai_runtime.providers,
        vec![
            capture_types::AiProviderConfig {
                id: "anthropic".to_string(),
                kind: capture_types::AiProviderKind::Anthropic,
                label: String::new(),
                base_url: String::new(),
            },
            capture_types::AiProviderConfig {
                id: "ollama".to_string(),
                kind: capture_types::AiProviderKind::Ollama,
                label: String::new(),
                base_url: "http://localhost:11434".to_string(),
            },
        ]
    );
    assert_eq!(
        loaded.ai_runtime.default_model,
        Some(capture_types::AiEngineRef {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        })
    );
}

#[test]
fn default_recording_settings_include_preview_cache_ttl() {
    assert_eq!(
        default_recording_settings().preview_cache_ttl_seconds,
        default_preview_cache_ttl_seconds()
    );
}

#[test]
fn default_recording_settings_capture_screen_at_half_fps() {
    assert_eq!(default_recording_settings().screen_frame_rate, 0.5);
}

#[test]
fn load_recording_settings_from_path_defaults_preview_cache_ttl_when_missing() {
    let dir = TestDir::new("preview-cache-ttl-default");
    let path = dir.path().join("recording-settings.json");

    fs::write(
        &path,
        r#"{
            "captureScreen": true,
            "captureMicrophone": false,
            "captureSystemAudio": false,
            "segmentDurationSeconds": 60,
            "screenFrameRate": 1,
            "screenResolution": { "mode": "preset", "preset": "original" },
            "videoBitrate": { "mode": "preset", "preset": "medium" },
            "saveDirectory": "/tmp",
            "autoStart": false,
            "pauseCaptureOnInactivity": true,
            "idleTimeoutSeconds": 10,
            "microphoneActivitySensitivity": 50,
            "systemAudioActivitySensitivity": 50,
            "activityMode": "system_input_or_screen"
        }"#,
    )
    .expect("settings file should write");

    let loaded = load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert_eq!(
        loaded.preview_cache_ttl_seconds,
        default_preview_cache_ttl_seconds()
    );
    assert_eq!(loaded.follow_timeline_live, default_follow_timeline_live());
    assert_eq!(loaded.appearance, default_appearance());
    assert_eq!(loaded.ocr, default_ocr_settings());
    assert_eq!(loaded.access, AccessSettings::default());
    assert_eq!(loaded.transcription, default_audio_transcription_settings());
}

#[test]
fn settings_persist_and_load_through_the_core_config_dir() {
    let dir = TestDir::new("core-config");
    let core = crate::test_support::core_in(Default::default(), dir.path());
    assert_eq!(
        recording_settings_file_path(&core),
        dir.path().join("config").join("recording-settings.json")
    );

    let state = RecordingSettingsState::default();
    assert_eq!(
        initialize_recording_settings_state_from_disk(&core, &state).source,
        "defaults"
    );

    let mut settings = default_recording_settings();
    settings.auto_start = !settings.auto_start;
    persist_recording_settings(&core, &settings).expect("settings persist");

    let loaded = initialize_recording_settings_state_from_disk(&core, &state);
    assert_eq!(loaded.source, "disk");
    assert_eq!(current_recording_settings(&state), settings);
    assert_eq!(load_recording_settings_or_default(&core), settings);
}
