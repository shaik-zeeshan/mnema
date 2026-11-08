use super::*;

#[test]
fn validate_privacy_settings_preserves_app_bundle_id_casing_and_dedupes_exact_ids() {
    let mut privacy = default_privacy_settings();
    privacy.excluded_apps = vec![
        capture_metadata::ExcludedAppEntry {
            id: "app-a".to_string(),
            enabled: true,
            bundle_id: " com.apple.Safari ".to_string(),
            display_name: "Safari".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-b".to_string(),
            enabled: true,
            bundle_id: "com.apple.Safari".to_string(),
            display_name: "Safari duplicate exact".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-c".to_string(),
            enabled: true,
            bundle_id: "com.apple.safari".to_string(),
            display_name: "Different-case bundle ID".to_string(),
        },
    ];

    let normalized = validate_privacy_settings(privacy).expect("privacy should validate");

    assert_eq!(normalized.excluded_apps.len(), 2);
    assert_eq!(normalized.excluded_apps[0].id, "app-a");
    assert_eq!(normalized.excluded_apps[0].bundle_id, "com.apple.Safari");
    assert_eq!(normalized.excluded_apps[1].id, "app-c");
    assert_eq!(normalized.excluded_apps[1].bundle_id, "com.apple.safari");
}

// A rule for an app the user has not installed yet has no bundle id, only a
// typed name. Validation runs on every settings write and on load, so
// dropping it here would silently delete the rule the user just made.
#[test]
fn validate_privacy_settings_keeps_a_pending_rule_and_dedupes_it_by_display_name() {
    let mut privacy = default_privacy_settings();
    privacy.excluded_apps = vec![
        capture_metadata::ExcludedAppEntry {
            id: "app-pending".to_string(),
            enabled: false,
            bundle_id: "  ".to_string(),
            display_name: " Figma ".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-pending-duplicate".to_string(),
            enabled: true,
            bundle_id: String::new(),
            display_name: "figma".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-nameless".to_string(),
            enabled: true,
            bundle_id: String::new(),
            display_name: "   ".to_string(),
        },
    ];

    let normalized = validate_privacy_settings(privacy).expect("privacy should validate");

    assert_eq!(normalized.excluded_apps.len(), 1);
    assert_eq!(normalized.excluded_apps[0].id, "app-pending");
    assert!(normalized.excluded_apps[0].bundle_id.is_empty());
    assert_eq!(normalized.excluded_apps[0].display_name, " Figma ");
    assert!(!normalized.excluded_apps[0].enabled);
}

#[test]
fn validate_recording_settings_normalizes_microphone_vad_adapter_from_shared_detector() {
    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr: default_ocr_settings(),
            transcription: default_audio_transcription_settings(),
            speaker_analysis: default_speaker_analysis_settings(),
            audio_speech_detection: capture_types::AudioSpeechDetectionSettings {
                detector: capture_types::AudioSpeechDetector::Off,
            },
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
            microphone_vad_adapter: capture_types::MicrophoneVadAdapter::Webrtc,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("settings should validate");

    assert_eq!(
        settings.microphone_vad_adapter,
        capture_types::AudioSpeechDetector::Off
    );
}

#[test]
fn validate_recording_settings_preserves_ocr_provider_specific_defaults() {
    let mut ocr = default_ocr_settings();
    ocr.provider = capture_types::OcrProvider::Tesseract;
    ocr.model_id = None;
    ocr.language = Some(" eng ".to_string());
    ocr.recognition_mode = capture_types::OcrRecognitionMode::Accurate;
    ocr.language_correction = true;
    ocr.tesseract_upscale_factor = 2;
    ocr.tesseract_char_whitelist = Some(" 0123456789 ".to_string());
    ocr.tesseract_page_segmentation_mode =
        capture_types::OcrTesseractPageSegmentationMode::SparseText;
    ocr.tesseract_preprocess_mode = capture_types::OcrTesseractPreprocessMode::Thresholded;

    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr,
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
            microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("settings should validate");

    assert_eq!(settings.ocr.provider, capture_types::OcrProvider::Tesseract);
    assert_eq!(
        settings.ocr.model_id.as_deref(),
        Some(ocr::DEFAULT_TESSERACT_MODEL_ID)
    );
    assert_eq!(
        settings.ocr.language.as_deref(),
        Some(ocr::DEFAULT_TESSERACT_LANGUAGE)
    );
    assert_eq!(
        settings.ocr.recognition_mode,
        capture_types::OcrRecognitionMode::Fast
    );
    assert!(!settings.ocr.language_correction);
    assert_eq!(
        settings.ocr.tesseract_page_segmentation_mode,
        capture_types::OcrTesseractPageSegmentationMode::SparseText
    );
    assert_eq!(
        settings.ocr.tesseract_preprocess_mode,
        capture_types::OcrTesseractPreprocessMode::Thresholded
    );
    assert_eq!(settings.ocr.tesseract_upscale_factor, 2);
    assert_eq!(
        settings.ocr.tesseract_char_whitelist.as_deref(),
        Some("0123456789")
    );
}

#[test]
fn validate_recording_settings_normalizes_legacy_paddle_ocr_to_default_provider() {
    let mut ocr_settings = default_ocr_settings();
    ocr_settings.provider = capture_types::OcrProvider::PaddleOcr;
    ocr_settings.model_id = Some(ocr::DEFAULT_PADDLE_OCR_MODEL_ID.to_string());
    ocr_settings.language = Some(ocr::DEFAULT_PADDLE_OCR_LANGUAGE.to_string());

    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr: ocr_settings,
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
            microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("legacy PaddleOCR settings should normalize");

    assert_eq!(settings.ocr, default_ocr_settings());
}

#[test]
fn validate_recording_settings_preserves_transcription_update() {
    let mut transcription = default_audio_transcription_settings();
    transcription.provider = capture_types::AudioTranscriptionProvider::Parakeet;
    transcription.model_id = Some("parakeet-tdt-0.6b-v3-onnx".to_string());
    transcription.language = " en ".to_string();

    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr: default_ocr_settings(),
            transcription,
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
            microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("settings should validate");

    assert_eq!(
        settings.transcription.provider,
        capture_types::AudioTranscriptionProvider::Parakeet
    );
    assert_eq!(
        settings.transcription.model_id.as_deref(),
        Some("parakeet-tdt-0.6b-v3-onnx")
    );
    assert_eq!(settings.transcription.language, "en");
}

#[test]
fn validate_audio_transcription_settings_deepgram_model_rules() {
    let deepgram = |model_id: Option<&str>| {
        let mut settings = default_audio_transcription_settings();
        settings.provider = AudioTranscriptionProvider::Deepgram;
        settings.model_id = model_id.map(str::to_string);
        validate_audio_transcription_settings(settings)
    };

    // Explicit supported model is preserved.
    let ok = deepgram(Some("nova-2")).expect("nova-2 should validate");
    assert_eq!(ok.model_id.as_deref(), Some("nova-2"));

    // Missing model falls back to the nova-3 default.
    let defaulted = deepgram(None).expect("missing model should default");
    assert_eq!(defaulted.model_id.as_deref(), Some("nova-3"));

    // Unsupported model is rejected.
    let err = deepgram(Some("whisper-large")).expect_err("whisper-large should be rejected");
    assert_eq!(err.code, "invalid_recording_settings");
}

#[test]
fn validate_recording_settings_rejects_preview_cache_ttl_above_max() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 1.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        developer_options_enabled: false,
        preview_cache_ttl_seconds: MAX_PREVIEW_CACHE_TTL_SECONDS + 1,
        follow_timeline_live: false,
        retention_policy: RetentionPolicy::Never,
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
        microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("preview cache ttl above max must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        format!("previewCacheTtlSeconds must be between 0 and {MAX_PREVIEW_CACHE_TTL_SECONDS}")
    );
}

#[test]
fn validate_speaker_analysis_settings_keeps_default_model() {
    let settings = SpeakerAnalysisSettings {
        model_id: Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_falls_back_for_unknown_model() {
    let settings = SpeakerAnalysisSettings {
        model_id: Some("bogus-model-xyz".to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    // speakrs is the global default provider, so its unknown-model fallback
    // also coincides with the global default model.
    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(validated.model_id, default_speaker_analysis_model_id());
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_keeps_speakrs_default_model() {
    let settings = SpeakerAnalysisSettings {
        provider: speaker_analysis::SPEAKRS_PROVIDER_ID.to_string(),
        model_id: Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_speakrs_unknown_model_resets_to_speakrs_default() {
    // Unknown model_id for a KNOWN provider resets to THAT provider's default
    // model — the provider choice (speakrs) is preserved, not dropped to
    // sherpa.
    let settings = SpeakerAnalysisSettings {
        provider: speaker_analysis::SPEAKRS_PROVIDER_ID.to_string(),
        model_id: Some("bogus-speakrs-model".to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

/// MIGRATION: a settings file persisted before sherpa removal carries
/// `provider = "sherpa_onnx"` (plus a sherpa model id). Because sherpa is no
/// longer a known provider, validation must remap it to speakrs + the speakrs
/// default model — it must be impossible to leave a user pinned on sherpa.
#[test]
fn validate_speaker_analysis_settings_remaps_legacy_sherpa_provider_to_speakrs() {
    let settings = SpeakerAnalysisSettings {
        provider: "sherpa_onnx".to_string(),
        model_id: Some("pyannote-3.0-nemo-titanet-small".to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_unknown_provider_resets_to_speakrs_default() {
    let settings = SpeakerAnalysisSettings {
        provider: "totally-unknown-provider".to_string(),
        model_id: Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    let defaults = default_speaker_analysis_settings();
    assert_eq!(validated.provider, defaults.provider);
    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(validated.model_id, defaults.model_id);
}

#[test]
fn validate_speaker_analysis_settings_known_provider_missing_model_uses_provider_default() {
    // `model_id: None` for a known non-default provider resolves to that
    // provider's default model, not the global default.
    let settings = SpeakerAnalysisSettings {
        provider: speaker_analysis::SPEAKRS_PROVIDER_ID.to_string(),
        model_id: None,
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_semantic_search_settings_trims_and_keeps_known_model() {
    // L4: an untrimmed but otherwise-known guided-tier provider + model is
    // trimmed and kept (mirrors validate_speaker_analysis_settings keeping a
    // known model). The default model id is a known guided tier.
    let known_model = default_semantic_search_model_id().expect("a default model id");
    let settings = SemanticSearchSettings {
        enabled: true,
        provider: format!("  {}  ", semantic_search::SEMANTIC_SEARCH_PROVIDER_ID),
        model_id: Some(format!("  {known_model}  ")),
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(
        validated.provider,
        semantic_search::SEMANTIC_SEARCH_PROVIDER_ID
    );
    assert_eq!(validated.model_id.as_deref(), Some(known_model.as_str()));
    assert!(validated.enabled, "the enabled flag is carried through");
}

#[test]
fn validate_semantic_search_settings_resets_unknown_provider_to_default() {
    // L4: an unrecognized provider resets to the default ("local"), exactly
    // as the speaker validator resets an unknown provider to "sherpa_onnx".
    let settings = SemanticSearchSettings {
        enabled: false,
        provider: "made-up-provider".to_string(),
        model_id: default_semantic_search_model_id(),
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(validated.provider, default_semantic_search_provider());
    // The model still resolves under the reset default provider, so it survives.
    assert_eq!(validated.model_id, default_semantic_search_model_id());
    assert!(!validated.enabled, "the enabled flag is carried through");
}

#[test]
fn validate_semantic_search_settings_falls_back_for_empty_model() {
    // L4: a PRESENT but empty/whitespace model id (no real selection) falls back
    // to the default model.
    for raw_model in ["   ", ""] {
        let settings = SemanticSearchSettings {
            enabled: true,
            provider: semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: Some(raw_model.to_string()),
            ..Default::default()
        };

        let validated = validate_semantic_search_settings(settings);

        assert_eq!(
            validated.model_id,
            default_semantic_search_model_id(),
            "a present empty/whitespace model {raw_model:?} must fall back to the default"
        );
    }
}

#[test]
fn validate_semantic_search_settings_preserves_a_present_unresolvable_model() {
    // An unrelated recording-settings save must NOT silently swap a present-but-
    // unresolvable model id to the (possibly dimension-incompatible) default —
    // that would desync the persisted selection from the vec0 table until the
    // next restart's reconcile. The id is preserved verbatim (trimmed); the
    // live-dimension authority no-ops it into keyword-only and reconciliation
    // re-aligns it. Changing the selected model happens only via the explicit
    // atomic switch path.
    let settings = SemanticSearchSettings {
        enabled: true,
        provider: semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some("  bogus-model-xyz  ".to_string()),
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(
        validated.model_id.as_deref(),
        Some("bogus-model-xyz"),
        "a present unresolvable model must be preserved (trimmed), not swapped to the default"
    );
}

#[test]
fn validate_semantic_search_settings_keeps_an_explicitly_cleared_model() {
    // L4 boundary: an explicit `None` is the legitimate "no model selected"
    // (keyword-only) sentinel and must NOT be resurrected to the default — this
    // is the one deliberate divergence from the speaker-analysis validator, and
    // it preserves the model-gated clear semantics the domain patch relies on.
    let settings = SemanticSearchSettings {
        enabled: false,
        provider: semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: None,
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(validated.model_id, None);
}
