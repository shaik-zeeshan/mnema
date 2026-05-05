mod inactivity;
mod logs;
mod microphone;
mod recording;
mod session;

use serde::Serialize;

pub use inactivity::*;
pub use logs::*;
pub use microphone::*;
pub use recording::*;
pub use session::*;

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
        assert!(json.get("sessionId").is_none());
        assert!(json.get("startedAtUnixMs").is_none());
    }

    #[test]
    fn recording_settings_deserialize_defaults_audio_sensitivity_and_supports_alias_mode_field() {
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
        assert_eq!(settings.follow_timeline_live, default_follow_timeline_live());
        assert_eq!(settings.appearance, default_appearance());
        assert_eq!(settings.ocr, default_ocr_settings());
        assert_eq!(
            settings.inactivity_activity_mode,
            InactivityActivityMode::SystemInputOrScreenOrAudio
        );
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
    }
}
