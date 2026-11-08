//! The root's own tests. The segment, lifecycle and microphone tests moved to
//! `app_core::native_capture::tests` with the code they exercise.

use super::describe_recording_settings_changes;
use super::{
    audio_transcription_unavailable_notification, ocr_unavailable_notification,
    recording_requires_speech_detector, should_warn_audio_transcription_unavailable_at_start,
    should_warn_audio_transcription_unavailable_at_startup, should_warn_ocr_unavailable_at_start,
    should_warn_ocr_unavailable_at_startup,
};
use app_core::native_capture::app_notifications::{AppNotificationAction, AppNotificationsRuntime};
#[cfg(target_os = "macos")]
use app_core::native_capture::lifecycle::{NativeCaptureState, RecordingLifecycle};
use capture_types::{
    default_appearance, default_audio_speech_detection_settings,
    default_audio_transcription_settings, default_inactivity_activity_mode,
    default_metadata_settings, default_microphone_vad_adapter, default_ocr_settings,
    default_preview_cache_ttl_seconds, default_privacy_settings, default_retention_policy,
    default_speaker_analysis_settings, default_video_bitrate, AccessSettings, AiRuntimeSettings,
    AppearanceSetting, AudioSpeechDetector, AudioTranscriptionProvider, AudioTranscriptionSettings,
    CaptureErrorResponse, InactivityActivityMode, OcrProvider, RecordingSettings, ScreenResolution,
    ScreenResolutionPreset, UserContextSettings, VideoBitrateMode, VideoBitrateSettings,
};
use std::path::PathBuf;

fn recording_settings_fixture() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
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
    }
}

fn run_async_test(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}

#[cfg(target_os = "macos")]
#[test]
fn display_reconfiguration_online_flags_trigger_wake_recovery() {
    use super::display_reconfiguration_flags_indicate_display_online as online;
    use core_graphics::display::CGDisplayChangeSummaryFlags as F;

    // End-of-configuration with a display coming (back) online re-arms capture:
    // the dark/deep-idle wake case where NSWorkspaceDidWake never fires.
    assert!(online(F::kCGDisplayAddFlag.bits()));
    assert!(online(F::kCGDisplayEnabledFlag.bits()));
    assert!(online(F::kCGDisplaySetMainFlag.bits()));
    assert!(online(F::kCGDisplaySetModeFlag.bits()));
    assert!(online(
        (F::kCGDisplayAddFlag | F::kCGDisplaySetModeFlag).bits()
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn display_reconfiguration_begin_and_offline_flags_skip_wake_recovery() {
    use super::display_reconfiguration_flags_indicate_display_online as online;
    use core_graphics::display::CGDisplayChangeSummaryFlags as F;

    // The begin-configuration half of the pair must not fire — even when paired
    // with online flags — so recovery runs once, at end-of-configuration.
    assert!(!online(F::kCGDisplayBeginConfigurationFlag.bits()));
    assert!(!online(
        (F::kCGDisplayBeginConfigurationFlag | F::kCGDisplayAddFlag).bits()
    ));

    // A display going away (remove/disable only) is not a wake.
    assert!(!online(F::kCGDisplayRemoveFlag.bits()));
    assert!(!online(F::kCGDisplayDisabledFlag.bits()));
    assert!(!online(
        (F::kCGDisplayRemoveFlag | F::kCGDisplayDisabledFlag).bits()
    ));

    // A bare move/mirror reconfiguration with no display coming online is a no-op.
    assert!(!online(F::kCGDisplayMovedFlag.bits()));
    assert!(!online(0));
}

#[cfg(target_os = "macos")]
#[test]
fn insert_privacy_app_candidate_fills_icon_materialization_fields_from_duplicate() {
    let mut candidates = std::collections::BTreeMap::new();
    let bundle_path = PathBuf::from("/Applications/Example.app");

    super::insert_privacy_app_candidate(
        &mut candidates,
        super::PrivacyAppCandidate {
            bundle_id: "com.example.App".to_string(),
            display_name: "Example".to_string(),
            running: true,
            icon_path: None,
            bundle_path: None,
        },
    );
    super::insert_privacy_app_candidate(
        &mut candidates,
        super::PrivacyAppCandidate {
            bundle_id: "com.example.App".to_string(),
            display_name: "Example".to_string(),
            running: false,
            icon_path: Some("/tmp/example-icon.png".to_string()),
            bundle_path: Some(bundle_path.clone()),
        },
    );

    let candidate = candidates
        .get("com.example.App")
        .expect("candidate should be merged by bundle id");
    assert!(candidate.running);
    assert_eq!(
        candidate.icon_path.as_deref(),
        Some("/tmp/example-icon.png")
    );
    assert_eq!(
        candidate.bundle_path.as_deref(),
        Some(bundle_path.as_path())
    );
}

#[test]
fn merge_running_privacy_app_candidates_inserts_unscanned_running_app() {
    let mut candidates = std::collections::BTreeMap::new();
    let bundle_path = PathBuf::from("/Volumes/Tools/Sensitive.app");

    super::merge_running_privacy_app_candidates(
        &mut candidates,
        vec![super::PrivacyAppCandidate {
            bundle_id: "com.example.Sensitive".to_string(),
            display_name: "Sensitive".to_string(),
            running: true,
            icon_path: None,
            bundle_path: Some(bundle_path.clone()),
        }],
    );

    let candidate = candidates
        .get("com.example.Sensitive")
        .expect("running app missing from install scan should be inserted");
    assert!(candidate.running);
    assert_eq!(candidate.display_name, "Sensitive");
    assert_eq!(
        candidate.bundle_path.as_deref(),
        Some(bundle_path.as_path())
    );
}

#[test]
fn audio_transcription_start_warning_requires_enabled_microphone_transcription() {
    let mut settings = recording_settings_fixture();
    settings.capture_microphone = true;
    settings.transcription.enabled = true;

    assert!(should_warn_audio_transcription_unavailable_at_start(
        &settings
    ));

    settings.capture_microphone = false;
    assert!(!should_warn_audio_transcription_unavailable_at_start(
        &settings
    ));

    settings.capture_microphone = true;
    settings.transcription.enabled = false;
    assert!(!should_warn_audio_transcription_unavailable_at_start(
        &settings
    ));
}

#[test]
fn audio_transcription_startup_warning_requires_enabled_microphone_transcription() {
    let mut settings = recording_settings_fixture();
    settings.capture_microphone = true;
    settings.transcription.enabled = true;

    assert!(should_warn_audio_transcription_unavailable_at_startup(
        &settings
    ));

    settings.capture_microphone = false;
    assert!(!should_warn_audio_transcription_unavailable_at_startup(
        &settings
    ));

    settings.capture_microphone = true;
    settings.transcription.enabled = false;
    assert!(!should_warn_audio_transcription_unavailable_at_startup(
        &settings
    ));
}

#[test]
fn speech_detector_preflight_only_requires_system_audio_transcription_gate() {
    let mut settings = recording_settings_fixture();
    settings.capture_screen = false;
    settings.capture_microphone = true;
    settings.capture_system_audio = false;
    settings.transcription.enabled = false;
    settings.transcription.microphone_enabled = false;
    settings.transcription.system_audio_enabled = false;
    settings.audio_speech_detection.detector = AudioSpeechDetector::Silero;

    assert!(!recording_requires_speech_detector(&settings));

    settings.capture_microphone = false;
    settings.capture_system_audio = true;
    settings.transcription.enabled = true;
    settings.transcription.system_audio_enabled = false;
    assert!(!recording_requires_speech_detector(&settings));

    settings.transcription.system_audio_enabled = true;
    assert!(recording_requires_speech_detector(&settings));

    settings.audio_speech_detection.detector = AudioSpeechDetector::Off;
    assert!(!recording_requires_speech_detector(&settings));
}

#[test]
fn ocr_start_warning_requires_screen_capture() {
    let mut settings = recording_settings_fixture();
    assert!(should_warn_ocr_unavailable_at_start(&settings));
    assert!(should_warn_ocr_unavailable_at_startup(&settings));

    settings.capture_screen = false;
    assert!(!should_warn_ocr_unavailable_at_start(&settings));
    assert!(!should_warn_ocr_unavailable_at_startup(&settings));
}

#[test]
fn ocr_unavailable_notification_opens_ocr_settings_tab() {
    let settings = RecordingSettings {
        capture_screen: true,
        ocr: capture_types::OcrSettings {
            provider: OcrProvider::Tesseract,
            model_id: Some("tesseract-5.5.2".to_string()),
            language: Some("eng".to_string()),
            ..capture_types::default_ocr_settings()
        },
        ..recording_settings_fixture()
    };

    let notification = ocr_unavailable_notification(&settings, 1234);

    assert_eq!(notification.id, "ocr-unavailable");
    assert_eq!(notification.severity, "warning");
    assert_eq!(notification.title, "OCR engine unavailable");
    assert!(notification.message.contains("Tesseract `tesseract-5.5.2`"));
    assert_eq!(notification.created_at_unix_ms, 1234);

    let payload = serde_json::to_value(&notification).expect("notification should serialize");
    assert_eq!(payload["action"]["type"], "open_settings_tab");
    assert_eq!(payload["action"]["tab"], "processing");

    match notification.action {
        Some(AppNotificationAction::OpenSettingsTab { tab }) => {
            assert_eq!(tab, "processing");
        }
        _ => panic!("OCR warning should include settings CTA"),
    }
}

#[test]
fn audio_transcription_unavailable_notification_opens_transcription_settings_tab() {
    let settings = RecordingSettings {
        capture_microphone: true,
        transcription: AudioTranscriptionSettings {
            provider: AudioTranscriptionProvider::LocalWhisper,
            model_id: Some("base".to_string()),
            language: "auto".to_string(),
            enabled: true,
            ..capture_types::default_audio_transcription_settings()
        },
        ..recording_settings_fixture()
    };

    let notification = audio_transcription_unavailable_notification(&settings, 1234);

    assert_eq!(notification.id, "audio-transcription-unavailable");
    assert_eq!(notification.severity, "warning");
    assert_eq!(notification.title, "Transcription model unavailable");
    assert!(notification.message.contains("Local Whisper `base`"));
    assert_eq!(notification.created_at_unix_ms, 1234);

    let payload = serde_json::to_value(&notification).expect("notification should serialize");
    assert_eq!(payload["action"]["type"], "open_settings_tab");
    assert_eq!(payload["action"]["tab"], "transcription");

    match notification.action {
        Some(AppNotificationAction::OpenSettingsTab { tab }) => {
            assert_eq!(tab, "transcription");
        }
        _ => panic!("transcription warning should include settings CTA"),
    }
}

#[test]
fn audio_transcription_start_warning_reuses_one_notification_id() {
    let settings = RecordingSettings {
        capture_microphone: true,
        transcription: AudioTranscriptionSettings {
            provider: AudioTranscriptionProvider::Parakeet,
            model_id: Some("parakeet-tdt-0.6b-v3-onnx".to_string()),
            language: "auto".to_string(),
            enabled: true,
            ..capture_types::default_audio_transcription_settings()
        },
        ..recording_settings_fixture()
    };
    let mut runtime = AppNotificationsRuntime::default();

    runtime.push_session_notification(audio_transcription_unavailable_notification(&settings, 1));
    let notifications = runtime
        .push_session_notification(audio_transcription_unavailable_notification(&settings, 2));

    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].id, "audio-transcription-unavailable");
    assert_eq!(notifications[0].created_at_unix_ms, 2);
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
        preview_cache_ttl_seconds: 0,
        follow_timeline_live: true,
        appearance: AppearanceSetting::Dark,
        time_format: Default::default(),
        segment_duration_seconds: 120,
        screen_frame_rate: 24.0,
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
        microphone_activity_sensitivity: 75,
        system_audio_activity_sensitivity: 75,
        inactivity_activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
        ..previous.clone()
    };

    let changes = describe_recording_settings_changes(&previous, &next);

    assert!(changes.contains(&"sources screen=true, microphone=false, system_audio=false -> screen=true, microphone=true, system_audio=false".to_string()));
    assert!(!changes
        .iter()
        .any(|change| change.contains("save_directory")));
    assert!(changes.contains(&"auto_start false -> true".to_string()));
    assert!(changes.contains(&format!(
        "preview_cache_ttl_seconds {} -> 0",
        default_preview_cache_ttl_seconds()
    )));
    assert!(changes.contains(&"follow_timeline_live false -> true".to_string()));
    assert!(changes.contains(&"segment_duration_seconds 60 -> 120".to_string()));
    assert!(changes.contains(&"screen_frame_rate 5 -> 24".to_string()));
    assert!(changes.contains(&"screen_resolution original -> 720p".to_string()));
    assert!(changes.contains(&"video_bitrate preset:medium -> custom:8mbps".to_string()));
    assert!(changes.contains(&"pause_on_inactivity true -> false".to_string()));
    assert!(changes.contains(&"idle_timeout_seconds 10 -> 30".to_string()));
    assert!(changes.contains(&"microphone_activity_sensitivity 50 -> 75".to_string()));
    assert!(changes.contains(&"system_audio_activity_sensitivity 50 -> 75".to_string()));
    assert!(changes.contains(
        &"activity_mode system_input_or_screen -> system_input_or_screen_or_audio".to_string()
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_retry_policy_treats_screen_capture_kit_display_loss_as_transient() {
    let error = CaptureErrorResponse {
        code: "capture_stream_start_failed".to_string(),
        message: "Failed to start ScreenCaptureKit capture: Failed to find any displays or windows to capture (code: -3815)".to_string(),
    };

    assert!(super::is_recover_after_wake_retryable_error(&error));
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_retry_policy_covers_slow_display_reappearance() {
    let total_retry_window_ms: u64 = super::SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS
        .iter()
        .copied()
        .sum();

    assert!(
        total_retry_window_ms >= 60_000,
        "ScreenCaptureKit displays can reappear slowly after wake; retry window was only {total_retry_window_ms}ms"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_retry_policy_does_not_retry_invalid_runtime_state() {
    let error = CaptureErrorResponse {
        code: "invalid_runtime_state".to_string(),
        message: "Capture screen planner missing while recovering after system wake".to_string(),
    };

    assert!(!super::is_recover_after_wake_retryable_error(&error));
}

#[test]
fn firefox_browser_url_support_is_reported_unknown() {
    run_async_test(async {
        let response = super::check_browser_url_support(super::CheckBrowserUrlSupportRequest {
            bundle_id: "org.mozilla.firefox".to_string(),
        })
        .await
        .expect("browser URL support check should succeed");

        assert!(!response.supported);
        assert_eq!(
            response.warning.as_deref(),
            Some(
                "URL metadata support is unknown for this browser. When website privacy rules are enabled, this browser may be redacted because its URL cannot be checked."
            )
        );
    });
}

#[cfg(target_os = "macos")]
#[test]
fn a_take_that_panics_still_hands_the_microphone_back() {
    let state: NativeCaptureState =
        std::sync::Arc::new(std::sync::Mutex::new(RecordingLifecycle::default()));
    state.lock().unwrap().runtime_mut().microphone_lent_out = true;

    let take = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lend = super::MicrophoneLend(&state);
        panic!("the bounded take failed");
    }));

    assert!(take.is_err());
    assert!(!state.lock().unwrap().runtime().microphone_lent_out);
}

// The Accessibility-status DTO lists exactly the Gecko browsers — those whose
// URL strategy is the Accessibility API — and no AppleScript browsers. The
// filter is the load-bearing part; `trusted`/`installed` are environment
// dependent (a real TCC grant + an installed app), so we don't assert them here
// — those are a manual `/verify` step.
#[test]
fn browser_url_accessibility_status_lists_only_gecko_browsers() {
    use std::collections::BTreeSet;

    let status = super::browser_url_accessibility_status();
    let bundle_ids: BTreeSet<&str> = status
        .gecko_browsers
        .iter()
        .map(|browser| browser.bundle_id.as_str())
        .collect();

    let expected: BTreeSet<&str> = ["org.mozilla.firefox", "app.zen-browser.zen"]
        .into_iter()
        .collect();
    assert_eq!(
        bundle_ids, expected,
        "Accessibility status should list exactly the Gecko (AX-strategy) browsers"
    );
}
