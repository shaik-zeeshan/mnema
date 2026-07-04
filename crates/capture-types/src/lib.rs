mod conversation;
mod inactivity;
mod logs;
mod microphone;
mod recording;
mod session;
mod usage_charts;
mod user_context;

use serde::Serialize;

pub use capture_metadata::{BrowserUrlMode, ExcludedAppEntry, MetadataSettings, PrivacySettings};
pub use conversation::*;
pub use inactivity::*;
pub use logs::*;
pub use microphone::*;
pub use recording::*;
pub use session::*;
pub use usage_charts::*;
pub use user_context::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureErrorResponse {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_session_meta_serializes_camel_case() {
        let meta = SourceSessionMeta {
            session_id: "sess-abc".to_string(),
            started_at_unix_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_value(&meta).expect("serialize");
        assert_eq!(json["sessionId"], "sess-abc");
        assert_eq!(json["startedAtUnixMs"], 1_700_000_000_000u64);
    }

    #[test]
    fn source_sessions_partial_population_serializes_nulls() {
        let sessions = SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "scr-1".to_string(),
                started_at_unix_ms: 1000,
            }),
            microphone: None,
            system_audio: None,
        };
        let json = serde_json::to_value(&sessions).expect("serialize");
        assert!(json["screen"].is_object());
        assert!(json["microphone"].is_null());
        assert!(json["systemAudio"].is_null());
    }

    #[test]
    fn native_capture_session_includes_source_sessions_field() {
        let session = NativeCaptureSession {
            is_running: true,
            is_inactivity_paused: false,
            is_user_paused: false,
            is_low_disk_suspended: false,
            requested_sources: None,
            output_files: None,
            source_sessions: Some(SourceSessions {
                screen: Some(SourceSessionMeta {
                    session_id: "scr-1".to_string(),
                    started_at_unix_ms: 1000,
                }),
                microphone: Some(SourceSessionMeta {
                    session_id: "mic-1".to_string(),
                    started_at_unix_ms: 1001,
                }),
                system_audio: None,
            }),
        };
        let json = serde_json::to_value(&session).expect("serialize");
        assert_eq!(json["sourceSessions"]["screen"]["sessionId"], "scr-1");
        assert_eq!(json["sourceSessions"]["microphone"]["sessionId"], "mic-1");
        assert!(json["sourceSessions"]["systemAudio"].is_null());
        assert_eq!(json["isLowDiskSuspended"], false);
        assert!(json.get("sessionId").is_none());
        assert!(json.get("startedAtUnixMs").is_none());
    }

    #[test]
    fn recording_settings_deserialize_defaults_audio_sensitivity_vad_and_supports_alias_mode_field()
    {
        let settings: RecordingSettings = serde_json::from_str(
            r#"{
                "captureScreen": true,
                "captureMicrophone": true,
                "captureSystemAudio": true,
                "segmentDurationSeconds": 60,
                "screenFrameRate": 30,
                "screenResolution": { "mode": "preset", "preset": "original" },
                "videoBitrate": { "mode": "preset", "preset": "medium" },
                "saveDirectory": "/tmp",
                "autoStart": false,
                "pauseCaptureOnInactivity": true,
                "idleTimeoutSeconds": 10,
                "inactivityActivityMode": "system_input_or_screen_or_audio"
            }"#,
        )
        .expect("settings should deserialize");

        assert_eq!(
            settings.microphone_activity_sensitivity,
            default_microphone_activity_sensitivity()
        );
        assert_eq!(
            settings.system_audio_activity_sensitivity,
            default_system_audio_activity_sensitivity()
        );
        assert_eq!(
            settings.microphone_vad_adapter,
            default_microphone_vad_adapter()
        );
        assert_eq!(
            settings.native_capture_debug_logging_enabled,
            default_native_capture_debug_logging_enabled()
        );
        assert_eq!(
            settings.developer_options_enabled,
            default_developer_options_enabled()
        );
        assert_eq!(
            settings.preview_cache_ttl_seconds,
            default_preview_cache_ttl_seconds()
        );
        assert_eq!(
            settings.follow_timeline_live,
            default_follow_timeline_live()
        );
        assert_eq!(settings.retention_policy, default_retention_policy());
        assert_eq!(settings.appearance, default_appearance());
        assert_eq!(settings.ocr, default_ocr_settings());
        assert_eq!(
            settings.inactivity_activity_mode,
            InactivityActivityMode::SystemInputOrScreenOrAudio
        );
    }

    #[test]
    fn recording_settings_serializes_audio_speech_detection_and_omits_legacy_vad_adapter() {
        let settings = RecordingSettings {
            capture_screen: true,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            native_capture_debug_logging_enabled: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: default_retention_policy(),
            appearance: default_appearance(),
            ocr: default_ocr_settings(),
            transcription: default_audio_transcription_settings(),
            speaker_analysis: default_speaker_analysis_settings(),
            audio_speech_detection: default_audio_speech_detection_settings(),
            metadata: default_metadata_settings(),
            privacy: default_privacy_settings(),
            access: AccessSettings::default(),
            ai_runtime: AiRuntimeSettings::default(),
            user_context: UserContextSettings::default(),
            semantic_search: default_semantic_search_settings(),
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            microphone_activity_sensitivity: 50,
            system_audio_activity_sensitivity: 50,
            microphone_vad_adapter: MicrophoneVadAdapter::Webrtc,
            inactivity_activity_mode: InactivityActivityMode::SystemInputOrScreen,
        };

        let json = serde_json::to_value(&settings).expect("settings should serialize");

        assert_eq!(json["access"]["askAiEnabled"], false);
        assert_eq!(json["audioSpeechDetection"]["detector"], "silero");
        assert!(json.get("microphoneVadAdapter").is_none());
    }

    #[test]
    fn recording_settings_deserializes_default_transcription_settings_when_missing() {
        let settings: RecordingSettings = serde_json::from_str(
            r#"{
                "captureScreen": true,
                "captureMicrophone": true,
                "captureSystemAudio": false,
                "segmentDurationSeconds": 60,
                "screenFrameRate": 30,
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
        .expect("settings should deserialize");

        assert_eq!(
            settings.transcription,
            default_audio_transcription_settings()
        );
        // The Semantic Search Model selection is default-on (English nomic tier)
        // when absent — older settings files have no `semanticSearch` field.
        assert_eq!(
            settings.semantic_search,
            default_semantic_search_settings()
        );
        assert!(settings.semantic_search.enabled);
        assert_eq!(
            settings.semantic_search.model_id.as_deref(),
            Some("nomic-embed-text-v1.5")
        );
    }

    #[test]
    fn recording_settings_round_trips_semantic_search_selection() {
        // A minimal settings JSON fills every optional field from its default;
        // we then override only the semantic search selection.
        let mut settings: RecordingSettings = serde_json::from_str(
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
        .expect("settings should deserialize");
        settings.semantic_search.model_id = Some("bge-m3".to_string());
        let json = serde_json::to_value(&settings).expect("serialize");
        assert_eq!(json["semanticSearch"]["modelId"], "bge-m3");
        assert_eq!(json["semanticSearch"]["provider"], "local");
        let back: RecordingSettings =
            serde_json::from_value(json).expect("round-trips back");
        assert_eq!(back.semantic_search, settings.semantic_search);
    }

    #[test]
    fn update_semantic_search_request_distinguishes_absent_null_and_set_model() {
        // Absent model_id => None (leave unchanged).
        let absent: UpdateSemanticSearchSettingsRequest =
            serde_json::from_str(r#"{ "enabled": true }"#).expect("parse");
        assert_eq!(absent.model_id, None);
        assert_eq!(absent.enabled, Some(true));

        // Explicit null => Some(None) (clear the selection).
        let cleared: UpdateSemanticSearchSettingsRequest =
            serde_json::from_str(r#"{ "modelId": null }"#).expect("parse");
        assert_eq!(cleared.model_id, Some(None));

        // A model id => Some(Some(id)).
        let set: UpdateSemanticSearchSettingsRequest =
            serde_json::from_str(r#"{ "modelId": "bge-m3" }"#).expect("parse");
        assert_eq!(set.model_id, Some(Some("bge-m3".to_string())));
    }

    #[test]
    fn update_recording_settings_request_deserializes_explicit_transcription_settings() {
        let request: UpdateRecordingSettingsRequest = serde_json::from_str(
            r#"{
                "captureScreen": true,
                "captureMicrophone": true,
                "captureSystemAudio": false,
                "segmentDurationSeconds": 60,
                "screenFrameRate": 30,
                "screenResolution": { "mode": "preset", "preset": "original" },
                "videoBitrate": { "mode": "preset", "preset": "medium" },
                "saveDirectory": "/tmp",
                "autoStart": false,
                "pauseCaptureOnInactivity": true,
                "idleTimeoutSeconds": 10,
                "microphoneActivitySensitivity": 50,
                "systemAudioActivitySensitivity": 50,
                "transcription": {
                    "enabled": true,
                    "provider": "parakeet",
                    "modelId": "parakeet-tdt-0.6b-v3-onnx",
                    "language": "en"
                },
                "activityMode": "system_input_or_screen"
            }"#,
        )
        .expect("request should deserialize");

        assert!(request.transcription.enabled);
        assert_eq!(
            request.transcription.provider,
            AudioTranscriptionProvider::Parakeet
        );
        assert_eq!(
            request.transcription.model_id.as_deref(),
            Some("parakeet-tdt-0.6b-v3-onnx")
        );
        assert_eq!(request.transcription.language, "en");
    }

    #[test]
    fn update_recording_settings_request_deserializes_explicit_microphone_vad_adapter() {
        let request: UpdateRecordingSettingsRequest = serde_json::from_str(
            r#"{
                "captureScreen": true,
                "captureMicrophone": true,
                "captureSystemAudio": false,
                "segmentDurationSeconds": 60,
                "screenFrameRate": 30,
                "screenResolution": { "mode": "preset", "preset": "original" },
                "videoBitrate": { "mode": "preset", "preset": "medium" },
                "saveDirectory": "/tmp",
                "autoStart": false,
                "pauseCaptureOnInactivity": true,
                "idleTimeoutSeconds": 10,
                "microphoneActivitySensitivity": 50,
                "systemAudioActivitySensitivity": 50,
                "microphoneVadAdapter": "off",
                "activityMode": "system_input_or_screen"
            }"#,
        )
        .expect("request should deserialize");

        assert_eq!(request.microphone_vad_adapter, MicrophoneVadAdapter::Off);
    }

    #[test]
    fn update_recording_settings_request_deserialize_defaults_debug_logging_flag() {
        let request: UpdateRecordingSettingsRequest = serde_json::from_str(
            r#"{
                "captureScreen": true,
                "captureMicrophone": false,
                "captureSystemAudio": false,
                "segmentDurationSeconds": 60,
                "screenFrameRate": 30,
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
        .expect("request should deserialize");

        assert_eq!(
            request.native_capture_debug_logging_enabled,
            default_native_capture_debug_logging_enabled()
        );
        assert_eq!(
            request.developer_options_enabled,
            default_developer_options_enabled()
        );
        assert_eq!(
            request.preview_cache_ttl_seconds,
            default_preview_cache_ttl_seconds()
        );
        assert_eq!(request.follow_timeline_live, default_follow_timeline_live());
        assert_eq!(request.appearance, default_appearance());
        assert_eq!(request.ocr, default_ocr_settings());
        assert_eq!(
            request.transcription,
            default_audio_transcription_settings()
        );
        assert_eq!(
            request.microphone_vad_adapter,
            default_microphone_vad_adapter()
        );
    }

    #[test]
    fn ai_runtime_settings_legacy_engine_shape_migrates_to_providers() {
        // A legacy engine-centric file (default engine + additionalEngines)
        // must deserialize into the provider list, with the old default
        // engine's {provider, model} becoming the global default model
        // (ADR 0034 migration is deserialization-level).
        let settings: AiRuntimeSettings = serde_json::from_str(
            r#"{
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
            }"#,
        )
        .expect("legacy ai_runtime settings should deserialize");

        assert!(settings.enabled);
        assert_eq!(
            settings.providers,
            vec![
                AiProviderConfig {
                    id: "anthropic".to_string(),
                    kind: AiProviderKind::Anthropic,
                    label: String::new(),
                    base_url: String::new(),
                },
                AiProviderConfig {
                    id: "ollama".to_string(),
                    kind: AiProviderKind::Ollama,
                    label: String::new(),
                    base_url: "http://localhost:11434".to_string(),
                },
            ]
        );
        assert_eq!(
            settings.default_model,
            Some(AiEngineRef {
                provider: "anthropic".to_string(),
                model: "claude-haiku-4-5".to_string(),
            })
        );
    }

    #[test]
    fn ai_runtime_settings_legacy_local_engine_migrates_to_local_provider() {
        // A legacy local default engine maps to its local-kind provider, with
        // the endpoint carried as the provider's baseUrl.
        let settings: AiRuntimeSettings = serde_json::from_str(
            r#"{
                "enabled": true,
                "engineKind": "local",
                "cloudProvider": "anthropic",
                "cloudModel": "claude-haiku-4-5",
                "cloudBaseUrl": "",
                "localKind": "ollama",
                "localEndpoint": "http://localhost:11434",
                "localModel": "llama3.2"
            }"#,
        )
        .expect("legacy local ai_runtime settings should deserialize");

        assert_eq!(
            settings.providers,
            vec![AiProviderConfig {
                id: "ollama".to_string(),
                kind: AiProviderKind::Ollama,
                label: String::new(),
                base_url: "http://localhost:11434".to_string(),
            }]
        );
        assert_eq!(
            settings.default_model,
            Some(AiEngineRef {
                provider: "ollama".to_string(),
                model: "llama3.2".to_string(),
            })
        );
    }

    #[test]
    fn ai_runtime_settings_new_shape_round_trips() {
        let settings = AiRuntimeSettings {
            enabled: true,
            providers: vec![
                AiProviderConfig {
                    id: "anthropic".to_string(),
                    kind: AiProviderKind::Anthropic,
                    label: String::new(),
                    base_url: String::new(),
                },
                AiProviderConfig {
                    id: "openai_compatible".to_string(),
                    kind: AiProviderKind::OpenaiCompatible,
                    label: "Fireworks".to_string(),
                    base_url: "https://api.example.com/v1".to_string(),
                },
            ],
            default_model: Some(AiEngineRef {
                provider: "anthropic".to_string(),
                model: "claude-haiku-4-5".to_string(),
            }),
        };

        let json = serde_json::to_value(&settings).expect("serialize");
        assert_eq!(json["providers"][0]["kind"], "anthropic");
        assert_eq!(json["providers"][0]["id"], "anthropic");
        assert_eq!(json["providers"][1]["id"], "openai_compatible");
        assert_eq!(json["providers"][1]["label"], "Fireworks");
        assert_eq!(json["providers"][1]["kind"], "openai_compatible");
        assert_eq!(json["providers"][1]["baseUrl"], "https://api.example.com/v1");
        assert_eq!(json["defaultModel"]["provider"], "anthropic");
        assert_eq!(json["defaultModel"]["model"], "claude-haiku-4-5");
        // Saves write ONLY the new shape — no legacy engine-centric keys.
        assert!(json.get("engineKind").is_none());
        assert!(json.get("additionalEngines").is_none());

        let round: AiRuntimeSettings =
            serde_json::from_value(json).expect("deserialize round-trip");
        assert_eq!(round, settings);
    }

    #[test]
    fn ai_runtime_settings_empty_object_deserializes_to_defaults() {
        // Neither shape present (e.g. `#[serde(default)]`-adjacent partials):
        // no providers, no default model.
        let settings: AiRuntimeSettings =
            serde_json::from_str(r#"{ "enabled": true }"#).expect("deserialize");
        assert!(settings.enabled);
        assert!(settings.providers.is_empty());
        assert!(settings.default_model.is_none());
    }

    #[test]
    fn update_ai_runtime_settings_request_default_model_is_double_option() {
        // Absent → leave unchanged.
        let request: UpdateAiRuntimeSettingsRequest =
            serde_json::from_str(r#"{ "enabled": true }"#).expect("deserialize");
        assert_eq!(request.default_model, None);

        // Explicit null → clear.
        let request: UpdateAiRuntimeSettingsRequest =
            serde_json::from_str(r#"{ "defaultModel": null }"#).expect("deserialize");
        assert_eq!(request.default_model, Some(None));

        // Object → set.
        let request: UpdateAiRuntimeSettingsRequest = serde_json::from_str(
            r#"{ "defaultModel": { "provider": "ollama", "model": "llama3.2" } }"#,
        )
        .expect("deserialize");
        assert_eq!(
            request.default_model,
            Some(Some(AiEngineRef {
                provider: "ollama".to_string(),
                model: "llama3.2".to_string(),
            }))
        );
    }

    #[test]
    fn user_context_settings_enabled_defaults_false_when_missing() {
        // Legacy persisted UserContextSettings (no `enabled`) must default the
        // continuous-derivation opt-in to OFF.
        let settings: UserContextSettings = serde_json::from_str(
            r#"{
                "derivationBudgetTier": "balanced",
                "backfillWindowDays": 30,
                "backfillGoDeeper": false
            }"#,
        )
        .expect("legacy user_context settings should deserialize");

        assert!(!settings.enabled);
        assert!(!UserContextSettings::default().enabled);
    }

    #[test]
    fn user_context_settings_round_trips_enabled() {
        let settings = UserContextSettings {
            enabled: true,
            ..UserContextSettings::default()
        };
        let json = serde_json::to_value(&settings).expect("serialize");
        assert_eq!(json["enabled"], true);
        let round: UserContextSettings =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(round, settings);
    }

    #[test]
    fn update_user_context_settings_request_deserializes_enabled() {
        let request: UpdateUserContextSettingsRequest =
            serde_json::from_str(r#"{ "enabled": true }"#).expect("request should deserialize");
        assert_eq!(request.enabled, Some(true));
    }

    #[test]
    fn recording_settings_domain_response_serializes_domain_as_snake_case() {
        let response = RecordingSettingsDomainUpdateResponse {
            domain: SettingsOwnershipDomain::AppPrivacyExclusion,
            settings: RecordingSettings {
                capture_screen: true,
                capture_microphone: false,
                capture_system_audio: false,
                segment_duration_seconds: 60,
                screen_frame_rate: 30.0,
                screen_resolution: ScreenResolution::Preset {
                    preset: ScreenResolutionPreset::Original,
                },
                video_bitrate: default_video_bitrate(),
                save_directory: "/tmp".to_string(),
                auto_start: false,
                native_capture_debug_logging_enabled: false,
                developer_options_enabled: false,
                preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
                follow_timeline_live: false,
                retention_policy: default_retention_policy(),
                appearance: default_appearance(),
                ocr: default_ocr_settings(),
                transcription: default_audio_transcription_settings(),
                speaker_analysis: default_speaker_analysis_settings(),
                audio_speech_detection: default_audio_speech_detection_settings(),
                metadata: default_metadata_settings(),
                privacy: default_privacy_settings(),
                access: AccessSettings::default(),
                ai_runtime: AiRuntimeSettings::default(),
                user_context: UserContextSettings::default(),
                semantic_search: default_semantic_search_settings(),
                pause_capture_on_inactivity: true,
                idle_timeout_seconds: 10,
                microphone_activity_sensitivity: 50,
                system_audio_activity_sensitivity: 50,
                microphone_vad_adapter: default_microphone_vad_adapter(),
                inactivity_activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
            },
        };

        let json = serde_json::to_value(&response).expect("response should serialize");

        assert_eq!(json["domain"], "app_privacy_exclusion");
        assert!(json["settings"].is_object());
    }
}
