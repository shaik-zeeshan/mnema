use capture_types::{
    default_ocr_settings, default_ocr_tesseract_char_whitelist,
    default_ocr_tesseract_page_segmentation_mode, default_ocr_tesseract_preprocess_mode,
    default_ocr_tesseract_upscale_factor, default_semantic_search_model_id,
    default_semantic_search_provider, default_speaker_analysis_model_id,
    default_speaker_analysis_settings, default_speaker_analysis_timeout_seconds, AiEngineRef,
    AiRuntimeSettings, AudioSpeechDetectionSettings, AudioSpeechDetector,
    AudioTranscriptionProvider, AudioTranscriptionSettings, CaptureErrorResponse, OcrProvider,
    OcrRecognitionMode, OcrSettings, RecordingSettings, ScreenResolution, ScreenResolutionPreset,
    SemanticSearchSettings, SpeakerAnalysisSettings, UpdateRecordingSettingsRequest,
    VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings, AI_RECENT_MODELS_CAP,
};

use super::*;

pub(super) fn validate_screen_resolution(
    value: ScreenResolution,
) -> Result<ScreenResolution, CaptureErrorResponse> {
    match value {
        ScreenResolution::Preset { .. } => Ok(value),
        ScreenResolution::Custom { width, height } => {
            const MIN_DIMENSION: u32 = 16;
            const MAX_DIMENSION: u32 = 8192;

            if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&width)
                || !(MIN_DIMENSION..=MAX_DIMENSION).contains(&height)
            {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: format!(
                        "Custom screen resolution width/height must be between {MIN_DIMENSION} and {MAX_DIMENSION}"
                    ),
                });
            }

            Ok(ScreenResolution::Custom { width, height })
        }
    }
}

pub(super) fn validate_video_bitrate(
    value: VideoBitrateSettings,
) -> Result<VideoBitrateSettings, CaptureErrorResponse> {
    match value.mode {
        VideoBitrateMode::Preset => Ok(VideoBitrateSettings {
            mode: VideoBitrateMode::Preset,
            preset: Some(value.preset.unwrap_or(VideoBitratePreset::Medium)),
            custom_mbps: None,
        }),
        VideoBitrateMode::Custom => {
            let custom_mbps = value.custom_mbps.ok_or_else(|| CaptureErrorResponse {
                code: "invalid_recording_settings".to_string(),
                message: "videoBitrate.customMbps is required when videoBitrate.mode is custom"
                    .to_string(),
            })?;

            if !(MIN_CUSTOM_VIDEO_BITRATE_MBPS..=MAX_CUSTOM_VIDEO_BITRATE_MBPS)
                .contains(&custom_mbps)
            {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: format!(
                        "videoBitrate.customMbps must be between {MIN_CUSTOM_VIDEO_BITRATE_MBPS} and {MAX_CUSTOM_VIDEO_BITRATE_MBPS}"
                    ),
                });
            }

            Ok(VideoBitrateSettings {
                mode: VideoBitrateMode::Custom,
                preset: None,
                custom_mbps: Some(custom_mbps),
            })
        }
    }
}

pub(super) fn validate_audio_transcription_settings(
    value: AudioTranscriptionSettings,
) -> Result<AudioTranscriptionSettings, CaptureErrorResponse> {
    let language = {
        let trimmed = value.language.trim();
        if trimmed.is_empty() {
            "auto".to_string()
        } else {
            trimmed.to_string()
        }
    };

    let model_id = value
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty());

    let model_id = match value.provider {
        AudioTranscriptionProvider::LocalWhisper => {
            let model_id = model_id.unwrap_or("base");
            if !matches!(model_id, "tiny" | "base" | "small" | "medium") {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: "transcription.modelId must be one of tiny, base, small, or medium for local_whisper".to_string(),
                });
            }
            Some(model_id.to_string())
        }
        AudioTranscriptionProvider::Parakeet => {
            let model_id = model_id.unwrap_or("parakeet-tdt-0.6b-v3-onnx-int8");
            if !matches!(
                model_id,
                "parakeet-tdt-0.6b-v3-onnx" | "parakeet-tdt-0.6b-v3-onnx-int8"
            ) {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: "transcription.modelId must be parakeet-tdt-0.6b-v3-onnx or parakeet-tdt-0.6b-v3-onnx-int8 for parakeet"
                        .to_string(),
                });
            }
            Some(model_id.to_string())
        }
        AudioTranscriptionProvider::Deepgram => {
            let model_id = model_id.unwrap_or("nova-3");
            if !matches!(model_id, "nova-3" | "nova-2") {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: "transcription.modelId must be nova-3 or nova-2 for deepgram"
                        .to_string(),
                });
            }
            Some(model_id.to_string())
        }
        AudioTranscriptionProvider::AppleSpeechOnDevice => None,
    };

    if value.idle_unload_seconds > 24 * 60 * 60 {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "transcription.idleUnloadSeconds must be <= 86400".to_string(),
        });
    }
    if value.chunk_seconds > 60 * 60 {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "transcription.chunkSeconds must be <= 3600".to_string(),
        });
    }

    Ok(AudioTranscriptionSettings {
        enabled: value.enabled,
        microphone_enabled: value.microphone_enabled,
        system_audio_enabled: value.system_audio_enabled,
        provider: value.provider,
        model_id,
        language,
        memory_mode: value.memory_mode,
        idle_unload_seconds: value.idle_unload_seconds,
        chunk_seconds: value.chunk_seconds,
    })
}

pub(super) fn validate_audio_speech_detection_settings(
    value: AudioSpeechDetectionSettings,
    transcription: &AudioTranscriptionSettings,
) -> Result<AudioSpeechDetectionSettings, CaptureErrorResponse> {
    if transcription.enabled
        && transcription.system_audio_enabled
        && value.detector == AudioSpeechDetector::Off
    {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "audioSpeechDetection.detector cannot be off while transcription.systemAudioEnabled is true".to_string(),
        });
    }

    Ok(value)
}

pub fn validate_privacy_settings(
    value: capture_types::PrivacySettings,
) -> Result<capture_types::PrivacySettings, CaptureErrorResponse> {
    let capture_types::PrivacySettings {
        excluded_apps,
        filter_system_audio,
    } = value;

    let mut seen_app_bundle_ids = std::collections::BTreeSet::new();
    let mut seen_pending_display_names = std::collections::BTreeSet::new();
    let excluded_apps = excluded_apps
        .into_iter()
        .filter_map(|mut app| {
            app.bundle_id = canonicalize_app_bundle_id(&app.bundle_id);
            // A rule for an app that is not installed yet has no bundle id, only
            // a typed name. It is kept so the first sighting of that app can fill
            // the id in place; until then `evaluate_privacy` skips it, so it
            // reaches neither the screen filter nor the system-audio tap.
            if app.bundle_id.is_empty() {
                let display_name = canonicalize_app_display_name(&app.display_name);
                if display_name.is_empty() || !seen_pending_display_names.insert(display_name) {
                    return None;
                }
                return Some(app);
            }
            if !seen_app_bundle_ids.insert(app.bundle_id.clone()) {
                return None;
            }
            Some(app)
        })
        .collect();

    Ok(capture_types::PrivacySettings {
        excluded_apps,
        filter_system_audio,
    })
}

pub fn canonicalize_app_bundle_id(bundle_id: &str) -> String {
    bundle_id.trim().to_string()
}

/// Match key for an app-exclusion rule that has no bundle id yet — the only
/// identity a not-yet-installed app has.
pub fn canonicalize_app_display_name(display_name: &str) -> String {
    display_name.trim().to_lowercase()
}

/// The default `model_id` for a known speaker-analysis provider, or `None` if the
/// provider is unknown. Drives the validation fallback below: an unknown model_id
/// for the known provider resets to that provider's default rather than dropping
/// the provider choice. Resolved from the manifest constants so adding a future
/// provider only needs a new arm here (and a manifest descriptor), not changes to
/// the validation control flow.
///
/// speakrs is the sole on-device provider; sherpa-onnx is removed, so any other
/// provider string (including the legacy `sherpa_onnx` literal) is "unknown" here
/// and gets remapped to speakrs by the caller.
pub(super) fn default_model_id_for_speaker_provider(provider: &str) -> Option<&'static str> {
    match provider {
        speaker_analysis::SPEAKRS_PROVIDER_ID => Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID),
        _ => None,
    }
}

pub(super) fn validate_speaker_analysis_settings(
    value: SpeakerAnalysisSettings,
) -> SpeakerAnalysisSettings {
    const MIN_TIMEOUT_SECONDS: u64 = 60;
    const MAX_TIMEOUT_SECONDS: u64 = 3600;

    let manifest = speaker_analysis::builtin_model_manifest();
    let requested_provider = value.provider.trim();
    let requested_model_id = value.model_id.as_deref().map(str::trim);

    // Dispatch model-id validation BY provider against the manifest:
    //   * a valid (provider, model_id) pair is kept verbatim;
    //   * an unknown model_id for the speakrs provider resets to its default
    //     model (the provider choice is preserved);
    //   * ANY non-speakrs provider — including the legacy `sherpa_onnx` literal a
    //     pre-removal settings file persisted — is remapped to speakrs + the
    //     speakrs default model. This is the upgrade-migration path: it is
    //     impossible to leave a user pinned on the removed sherpa provider.
    // Validating against the manifest (rather than a hardcoded id list) means a
    // future preset needs no change here.
    let (provider, model_id) = match default_model_id_for_speaker_provider(requested_provider) {
        Some(provider_default_model_id) => {
            let model_id = requested_model_id
                .filter(|model_id| {
                    speaker_analysis::find_model_descriptor(
                        &manifest,
                        requested_provider,
                        Some(model_id),
                    )
                    .is_some()
                })
                .unwrap_or(provider_default_model_id);
            (requested_provider.to_string(), Some(model_id.to_string()))
        }
        None => {
            // Legacy/unknown provider (e.g. `sherpa_onnx`) → speakrs default.
            (
                default_speaker_analysis_settings().provider,
                default_speaker_analysis_model_id(),
            )
        }
    };

    SpeakerAnalysisSettings {
        separate_speakers: value.separate_speakers,
        recognize_saved_people: value.recognize_saved_people,
        auto_label_owner: value.auto_label_owner,
        provider,
        model_id,
        timeout_seconds: if value.timeout_seconds == 0 {
            default_speaker_analysis_timeout_seconds()
        } else {
            value
                .timeout_seconds
                .clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS)
        },
    }
}

/// Normalize the **Semantic Search** settings before they are persisted, mirroring
/// [`validate_speaker_analysis_settings`] (finding L4): every other model-bearing
/// domain trims/normalizes its provider + model id, but `semantic_search` was
/// persisted raw, so a whitespace/empty/incoherent `provider`+`model_id` (a
/// hand-edited config, or a future free-text Custom picker) would land verbatim,
/// `resolve_selected_descriptor` would return `None`, and the worker + query would
/// silently no-op forever while the toggle still read enabled.
///
/// Like the speaker-analysis validator this is **infallible** (it normalizes rather
/// than rejecting):
/// - `provider`: trimmed; reset to the default (`"local"`) if it is not the one
///   recognized provider, exactly as the speaker validator resets an unrecognized
///   provider to `"sherpa_onnx"`.
/// - `model_id`: an explicit `None` is the legitimate **"no model selected"**
///   sentinel (the feature is default-on but model-gated, so cleared → keyword-only)
///   and is kept as `None` — this is the one deliberate divergence from the speaker
///   validator, whose `None` resets to a default because speaker analysis has no
///   "no model" off-state. A **present** id is trimmed; only an empty/whitespace id
///   (no real selection) falls back to the default model (`nomic-embed-text-v1.5`).
///   A present-but-unresolvable id is **preserved verbatim** rather than swapped to a
///   possibly dimension-incompatible default: changing the selected model must happen
///   only through the explicit atomic switch path, never as a silent side effect of an
///   unrelated recording-settings save. The live-dimension authority no-ops an
///   unresolvable id into keyword-only and startup reconciliation re-aligns it, so a
///   silent swap here would only desync the persisted selection from the vec0 table
///   until the next restart's reconcile. A present-and-known id is kept as-is, so a
///   real Custom selection survives.
/// - `enabled`: a plain bool, carried through unchanged (the speaker validator
///   likewise carries its bool flags through).
pub(super) fn validate_semantic_search_settings(
    value: SemanticSearchSettings,
) -> SemanticSearchSettings {
    let provider = if value.provider.trim() == semantic_search::SEMANTIC_SEARCH_PROVIDER_ID {
        semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string()
    } else {
        default_semantic_search_provider()
    };

    let model_id = match value.model_id {
        // Explicitly cleared — keep "no model selected" (keyword-only) rather than
        // resurrecting the default. This is the intentional model-gated off-state.
        None => None,
        // A present id: trim. Only an empty/whitespace id (no real selection) falls
        // back to the default. A present-but-unresolvable id is preserved verbatim
        // rather than swapped to a possibly dimension-incompatible default: changing
        // the selected model must happen only through the explicit atomic switch
        // path, never as a silent side effect of an unrelated settings save. The
        // live-dimension authority no-ops an unresolvable id into keyword-only and
        // startup reconciliation re-aligns it — wiping it here would leave the vec0
        // table disagreeing with the persisted selection until the next restart.
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                default_semantic_search_model_id()
            } else {
                Some(trimmed.to_string())
            }
        }
    };

    SemanticSearchSettings {
        enabled: value.enabled,
        provider,
        model_id,
    }
}

pub(super) fn validate_ocr_settings(
    value: OcrSettings,
) -> Result<OcrSettings, CaptureErrorResponse> {
    let model_id = value
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty());
    let language = value
        .language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty());
    let tesseract_char_whitelist = value
        .tesseract_char_whitelist
        .as_deref()
        .map(str::trim)
        .filter(|whitelist| !whitelist.is_empty())
        .map(ToOwned::to_owned);

    if !(1..=4).contains(&value.tesseract_upscale_factor) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "ocr.tesseractUpscaleFactor must be between 1 and 4".to_string(),
        });
    }

    let (
        enabled,
        model_id,
        language,
        recognition_mode,
        language_correction,
        tesseract_page_segmentation_mode,
        tesseract_preprocess_mode,
        tesseract_upscale_factor,
        tesseract_char_whitelist,
    ) = match value.provider {
        OcrProvider::AppleVision => (
            value.enabled,
            None,
            language.map(ToOwned::to_owned),
            value.recognition_mode,
            value.language_correction,
            default_ocr_tesseract_page_segmentation_mode(),
            default_ocr_tesseract_preprocess_mode(),
            default_ocr_tesseract_upscale_factor(),
            default_ocr_tesseract_char_whitelist(),
        ),
        OcrProvider::Tesseract => {
            let model_id = model_id.unwrap_or(ocr::DEFAULT_TESSERACT_MODEL_ID);
            if model_id != ocr::DEFAULT_TESSERACT_MODEL_ID {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: format!(
                        "ocr.modelId must be {} for tesseract",
                        ocr::DEFAULT_TESSERACT_MODEL_ID
                    ),
                });
            }
            let language = language.unwrap_or(ocr::DEFAULT_TESSERACT_LANGUAGE);
            if language != ocr::DEFAULT_TESSERACT_LANGUAGE {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: format!(
                        "ocr.language must be {} for tesseract in this build",
                        ocr::DEFAULT_TESSERACT_LANGUAGE
                    ),
                });
            }
            (
                value.enabled,
                Some(model_id.to_string()),
                Some(language.to_string()),
                OcrRecognitionMode::Fast,
                false,
                value.tesseract_page_segmentation_mode,
                value.tesseract_preprocess_mode,
                value.tesseract_upscale_factor,
                tesseract_char_whitelist,
            )
        }
        OcrProvider::PaddleOcr => {
            // PaddleOCR remains available in the OCR crate for existing queued jobs and
            // direct provider tests, but it is no longer a user-selectable recording
            // setting. Normalize legacy persisted settings back to the supported default.
            let mut settings = default_ocr_settings();
            settings.enabled = value.enabled;
            return Ok(settings);
        }
    };

    Ok(OcrSettings {
        enabled,
        provider: value.provider,
        model_id,
        language,
        recognition_mode,
        language_correction,
        tesseract_page_segmentation_mode,
        tesseract_preprocess_mode,
        tesseract_upscale_factor,
        tesseract_char_whitelist,
    })
}

pub(super) fn validate_audio_activity_sensitivity(
    field_name: &str,
    value: u8,
) -> Result<u8, CaptureErrorResponse> {
    if !(MIN_AUDIO_ACTIVITY_SENSITIVITY..=MAX_AUDIO_ACTIVITY_SENSITIVITY).contains(&value) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: format!(
                "{field_name} must be between {MIN_AUDIO_ACTIVITY_SENSITIVITY} and {MAX_AUDIO_ACTIVITY_SENSITIVITY}"
            ),
        });
    }

    Ok(value)
}

pub(super) fn video_bitrate_preset_factor(preset: VideoBitratePreset) -> f64 {
    match preset {
        VideoBitratePreset::Low => 0.07,
        VideoBitratePreset::Medium => 0.10,
        VideoBitratePreset::High => 0.14,
    }
}

pub(super) fn resolve_bitrate_dimensions(
    screen_resolution: &ScreenResolution,
) -> Option<(u32, u32)> {
    match screen_resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => None,
            ScreenResolutionPreset::P1080 => Some((1920, 1080)),
            ScreenResolutionPreset::P720 => Some((1280, 720)),
            ScreenResolutionPreset::P540 => Some((960, 540)),
        },
        ScreenResolution::Custom { width, height } => Some((*width, *height)),
    }
}

pub(super) fn clamp_and_round_bitrate_bits_per_second(raw_bps: f64) -> u32 {
    let clamped = raw_bps
        .clamp(
            MIN_EFFECTIVE_VIDEO_BITRATE_BPS as f64,
            MAX_EFFECTIVE_VIDEO_BITRATE_BPS as f64,
        )
        .round() as u64;
    let step = VIDEO_BITRATE_ROUND_STEP_BPS as u64;
    let rounded = ((clamped + (step / 2)) / step) * step;
    rounded as u32
}

pub fn compute_effective_screen_bitrate_bps(settings: &RecordingSettings) -> Option<u32> {
    if !settings.capture_screen {
        return None;
    }

    let bitrate = match settings.video_bitrate.mode {
        VideoBitrateMode::Custom => {
            let custom_mbps = settings.video_bitrate.custom_mbps? as f64;
            custom_mbps * 1_000_000.0
        }
        VideoBitrateMode::Preset => {
            let preset = settings
                .video_bitrate
                .preset
                .clone()
                .unwrap_or(VideoBitratePreset::Medium);
            let factor = video_bitrate_preset_factor(preset);
            let (width, height) =
                resolve_bitrate_dimensions(&settings.screen_resolution).unwrap_or((1920, 1080));
            (width as f64) * (height as f64) * settings.screen_frame_rate * factor
        }
    };

    Some(clamp_and_round_bitrate_bits_per_second(bitrate))
}

pub(super) fn is_original_screen_resolution(value: &ScreenResolution) -> bool {
    matches!(
        value,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original
        }
    )
}

pub(super) fn supports_non_original_screen_resolution() -> bool {
    capture_screen::supports_screen_capture_kit_backend()
}

pub fn validate_recording_settings(
    request: UpdateRecordingSettingsRequest,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    validate_recording_settings_with_resolution_support(
        request,
        supports_non_original_screen_resolution(),
    )
}

pub fn validate_recording_settings_with_resolution_support(
    request: UpdateRecordingSettingsRequest,
    non_original_resolution_supported: bool,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    if !request.capture_screen && !request.capture_microphone && !request.capture_system_audio {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "At least one capture source must be enabled".to_string(),
        });
    }

    let save_directory = request.save_directory.trim().to_string();
    if save_directory.is_empty() {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "saveDirectory must be non-empty".to_string(),
        });
    }

    if !(1..=MAX_SEGMENT_DURATION_SECONDS).contains(&request.segment_duration_seconds) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: format!(
                "segmentDurationSeconds must be between 1 and {MAX_SEGMENT_DURATION_SECONDS}"
            ),
        });
    }

    if !(MIN_SCREEN_FRAME_RATE..=MAX_SCREEN_FRAME_RATE).contains(&request.screen_frame_rate) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "screenFrameRate must be between 1/60 (one snapshot per minute) and 10"
                .to_string(),
        });
    }

    if !(MIN_IDLE_TIMEOUT_SECONDS..=MAX_IDLE_TIMEOUT_SECONDS)
        .contains(&request.idle_timeout_seconds)
    {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: format!(
                "idleTimeoutSeconds must be between {MIN_IDLE_TIMEOUT_SECONDS} and {MAX_IDLE_TIMEOUT_SECONDS}"
            ),
        });
    }

    if request.preview_cache_ttl_seconds > MAX_PREVIEW_CACHE_TTL_SECONDS {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: format!(
                "previewCacheTtlSeconds must be between 0 and {MAX_PREVIEW_CACHE_TTL_SECONDS}"
            ),
        });
    }

    let screen_resolution = validate_screen_resolution(request.screen_resolution)?;
    let video_bitrate = validate_video_bitrate(request.video_bitrate)?;
    let ocr = validate_ocr_settings(request.ocr)?;
    let transcription = validate_audio_transcription_settings(request.transcription)?;
    let audio_speech_detection =
        validate_audio_speech_detection_settings(request.audio_speech_detection, &transcription)?;
    let privacy = validate_privacy_settings(request.privacy)?;
    let speaker_analysis = validate_speaker_analysis_settings(request.speaker_analysis);
    let microphone_activity_sensitivity = validate_audio_activity_sensitivity(
        "microphoneActivitySensitivity",
        request.microphone_activity_sensitivity,
    )?;
    let system_audio_activity_sensitivity = validate_audio_activity_sensitivity(
        "systemAudioActivitySensitivity",
        request.system_audio_activity_sensitivity,
    )?;

    if request.capture_screen
        && !non_original_resolution_supported
        && !is_original_screen_resolution(&screen_resolution)
    {
        return Err(CaptureErrorResponse {
            code: "screen_resolution_unsupported".to_string(),
            message: "Selected screen resolution requires the ScreenCaptureKit backend (macOS 15+). On this backend, only the original display resolution is supported.".to_string(),
        });
    }

    Ok(RecordingSettings {
        capture_screen: request.capture_screen,
        capture_microphone: request.capture_microphone,
        capture_system_audio: request.capture_system_audio,
        segment_duration_seconds: request.segment_duration_seconds,
        screen_frame_rate: request.screen_frame_rate,
        screen_resolution,
        video_bitrate,
        save_directory,
        auto_start: request.auto_start,
        developer_options_enabled: request.developer_options_enabled,
        preview_cache_ttl_seconds: request.preview_cache_ttl_seconds,
        follow_timeline_live: request.follow_timeline_live,
        retention_policy: request.retention_policy,
        appearance: request.appearance,
        time_format: request.time_format,
        ocr,
        transcription,
        speaker_analysis,
        audio_speech_detection: audio_speech_detection.clone(),
        metadata: request.metadata,
        privacy,
        access: request.access,
        ai_runtime: normalize_ai_runtime_settings(request.ai_runtime),
        user_context: request.user_context,
        semantic_search: validate_semantic_search_settings(request.semantic_search),
        pause_capture_on_inactivity: request.pause_capture_on_inactivity,
        idle_timeout_seconds: request.idle_timeout_seconds,
        microphone_activity_sensitivity,
        system_audio_activity_sensitivity,
        microphone_vad_adapter: audio_speech_detection.detector,
        inactivity_activity_mode: capture_types::InactivityActivityMode::SystemInputOrScreenOrAudio,
    })
}

/// Normalize the provider-centric AI runtime settings (ADR 0034, amended):
/// trim ids/labels/base URLs, backfill an empty provider id to its kind id (a
/// legacy single-per-kind file), drop duplicate provider *instance ids* (first
/// wins — the keychain key lives at the instance id), and clear a default model
/// whose model id is blank. Multiple instances of one kind are kept as long as
/// their ids differ, so same-kind providers can coexist. Favorite and recent
/// models drop blank ids, duplicates, and refs to a provider no longer
/// connected (so removing a provider cleans them up for every caller); recents
/// are capped at [`AI_RECENT_MODELS_CAP`].
pub(super) fn normalize_ai_runtime_settings(
    mut ai_runtime: AiRuntimeSettings,
) -> AiRuntimeSettings {
    for provider in &mut ai_runtime.providers {
        provider.id = provider.id.trim().to_string();
        if provider.id.is_empty() {
            provider.id = provider.kind.id().to_string();
        }
        provider.label = provider.label.trim().to_string();
        provider.base_url = provider.base_url.trim().to_string();
    }
    let mut seen: Vec<String> = Vec::new();
    ai_runtime.providers.retain(|provider| {
        if seen.contains(&provider.id) {
            return false;
        }
        seen.push(provider.id.clone());
        true
    });
    ai_runtime.default_model = ai_runtime.default_model.and_then(|mut default_model| {
        default_model.model = default_model.model.trim().to_string();
        if default_model.model.is_empty() {
            None
        } else {
            Some(default_model)
        }
    });
    let providers = &ai_runtime.providers;
    let clean = |list: Vec<AiEngineRef>| {
        let mut out: Vec<AiEngineRef> = Vec::new();
        for mut engine in list {
            engine.provider = engine.provider.trim().to_string();
            engine.model = engine.model.trim().to_string();
            if engine.model.is_empty()
                || !providers.iter().any(|p| p.id == engine.provider)
                || out.contains(&engine)
            {
                continue;
            }
            out.push(engine);
        }
        out
    };
    let favorite_models = clean(std::mem::take(&mut ai_runtime.favorite_models));
    let mut recent_models = clean(std::mem::take(&mut ai_runtime.recent_models));
    recent_models.truncate(AI_RECENT_MODELS_CAP);
    ai_runtime.favorite_models = favorite_models;
    ai_runtime.recent_models = recent_models;
    ai_runtime
}
