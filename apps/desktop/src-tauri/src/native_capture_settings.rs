use capture_types::{
    default_appearance, default_audio_transcription_settings, default_developer_options_enabled,
    default_follow_timeline_live, default_idle_timeout_seconds, default_inactivity_activity_mode,
    default_microphone_activity_sensitivity, default_native_capture_debug_logging_enabled,
    default_ocr_settings, default_ocr_tesseract_char_whitelist,
    default_ocr_tesseract_page_segmentation_mode, default_ocr_tesseract_preprocess_mode,
    default_ocr_tesseract_upscale_factor, default_pause_capture_on_inactivity,
    default_preview_cache_ttl_seconds, default_speaker_analysis_model_id,
    default_speaker_analysis_settings, default_system_audio_activity_sensitivity,
    default_video_bitrate, AudioTranscriptionProvider, AudioTranscriptionSettings,
    CaptureErrorResponse, OcrProvider, OcrRecognitionMode, OcrSettings, RecordingSettings,
    RetentionPolicy, ScreenResolution, ScreenResolutionPreset, SpeakerAnalysisSettings,
    UpdateRecordingSettingsRequest, VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const MIN_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 1;
const MAX_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 40;
const MIN_IDLE_TIMEOUT_SECONDS: u64 = 1;
const MAX_IDLE_TIMEOUT_SECONDS: u64 = 3600;
const MIN_AUDIO_ACTIVITY_SENSITIVITY: u8 = 0;
const MAX_AUDIO_ACTIVITY_SENSITIVITY: u8 = 100;
const MAX_PREVIEW_CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;
const MIN_EFFECTIVE_VIDEO_BITRATE_BPS: u32 = 500_000;
const MAX_EFFECTIVE_VIDEO_BITRATE_BPS: u32 = 120_000_000;
const VIDEO_BITRATE_ROUND_STEP_BPS: u32 = 250_000;

#[derive(Debug, Clone)]
pub struct RecordingSettingsRuntime {
    pub settings: RecordingSettings,
}

impl Default for RecordingSettingsRuntime {
    fn default() -> Self {
        Self {
            settings: default_recording_settings(),
        }
    }
}

pub type RecordingSettingsState = Mutex<RecordingSettingsRuntime>;

pub(crate) struct LoadedRecordingSettings {
    pub(crate) settings: RecordingSettings,
    pub(crate) source: &'static str,
}

pub(crate) struct AppliedRecordingSettingsUpdate {
    pub(crate) settings: RecordingSettings,
    pub(crate) previous_settings: RecordingSettings,
    pub(crate) previous_save_directory: String,
    pub(crate) save_directory_changed: bool,
    pub(crate) debug_logging_enabled_changed: bool,
}

pub(crate) fn default_save_directory() -> String {
    std::env::var("HOME")
        .map(|home| Path::new(&home).join(".mnema"))
        .unwrap_or_else(|_| PathBuf::from(".mnema"))
        .to_string_lossy()
        .to_string()
}

pub(crate) fn default_recording_settings() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 1,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: default_save_directory(),
        auto_start: false,
        native_capture_debug_logging_enabled: default_native_capture_debug_logging_enabled(),
        developer_options_enabled: default_developer_options_enabled(),
        preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
        follow_timeline_live: default_follow_timeline_live(),
        retention_policy: RetentionPolicy::Never,
        appearance: default_appearance(),
        ocr: default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: default_speaker_analysis_settings(),
        pause_capture_on_inactivity: default_pause_capture_on_inactivity(),
        idle_timeout_seconds: default_idle_timeout_seconds(),
        microphone_activity_sensitivity: default_microphone_activity_sensitivity(),
        system_audio_activity_sensitivity: default_system_audio_activity_sensitivity(),
        microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    }
}

fn validate_screen_resolution(
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

fn validate_video_bitrate(
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

fn validate_audio_transcription_settings(
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
        provider: value.provider,
        model_id,
        language,
        memory_mode: value.memory_mode,
        idle_unload_seconds: value.idle_unload_seconds,
        chunk_seconds: value.chunk_seconds,
    })
}

fn validate_speaker_analysis_settings(value: SpeakerAnalysisSettings) -> SpeakerAnalysisSettings {
    const SHERPA_ONNX_PROVIDER_ID: &str = "sherpa_onnx";
    const DEFAULT_SHERPA_MODEL_ID: &str = "pyannote-3.0-nemo-titanet-small";

    let provider = if value.provider.trim() == SHERPA_ONNX_PROVIDER_ID {
        SHERPA_ONNX_PROVIDER_ID.to_string()
    } else {
        default_speaker_analysis_settings().provider
    };
    let model_id = value
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| *model_id == DEFAULT_SHERPA_MODEL_ID)
        .map(ToOwned::to_owned)
        .or_else(default_speaker_analysis_model_id);

    SpeakerAnalysisSettings {
        separate_speakers: value.separate_speakers,
        recognize_saved_people: value.recognize_saved_people,
        provider,
        model_id,
    }
}

fn validate_ocr_settings(value: OcrSettings) -> Result<OcrSettings, CaptureErrorResponse> {
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

fn validate_audio_activity_sensitivity(
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

fn video_bitrate_preset_factor(preset: VideoBitratePreset) -> f64 {
    match preset {
        VideoBitratePreset::Low => 0.07,
        VideoBitratePreset::Medium => 0.10,
        VideoBitratePreset::High => 0.14,
    }
}

fn resolve_bitrate_dimensions(screen_resolution: &ScreenResolution) -> Option<(u32, u32)> {
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

fn clamp_and_round_bitrate_bits_per_second(raw_bps: f64) -> u32 {
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

pub(crate) fn compute_effective_screen_bitrate_bps(settings: &RecordingSettings) -> Option<u32> {
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
            (width as f64) * (height as f64) * (settings.screen_frame_rate as f64) * factor
        }
    };

    Some(clamp_and_round_bitrate_bits_per_second(bitrate))
}

fn is_original_screen_resolution(value: &ScreenResolution) -> bool {
    matches!(
        value,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original
        }
    )
}

fn supports_non_original_screen_resolution() -> bool {
    capture_screen::support_for_current_platform().system_audio
}

pub(crate) fn validate_recording_settings(
    request: UpdateRecordingSettingsRequest,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    validate_recording_settings_with_resolution_support(
        request,
        supports_non_original_screen_resolution(),
    )
}

pub(crate) fn validate_recording_settings_with_resolution_support(
    request: UpdateRecordingSettingsRequest,
    non_original_resolution_supported: bool,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    if !request.capture_screen && !request.capture_microphone && !request.capture_system_audio {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "At least one capture source must be enabled".to_string(),
        });
    }

    if request.capture_system_audio && !request.capture_screen {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "System audio capture requires screen capture".to_string(),
        });
    }

    let save_directory = request.save_directory.trim().to_string();
    if save_directory.is_empty() {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "saveDirectory must be non-empty".to_string(),
        });
    }

    if request.segment_duration_seconds == 0 {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "segmentDurationSeconds must be greater than 0".to_string(),
        });
    }

    if !(1..=120).contains(&request.screen_frame_rate) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "screenFrameRate must be between 1 and 120".to_string(),
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
        native_capture_debug_logging_enabled: request.native_capture_debug_logging_enabled,
        developer_options_enabled: request.developer_options_enabled,
        preview_cache_ttl_seconds: request.preview_cache_ttl_seconds,
        follow_timeline_live: request.follow_timeline_live,
        retention_policy: request.retention_policy,
        appearance: request.appearance,
        ocr,
        transcription,
        speaker_analysis,
        pause_capture_on_inactivity: request.pause_capture_on_inactivity,
        idle_timeout_seconds: request.idle_timeout_seconds,
        microphone_activity_sensitivity,
        system_audio_activity_sensitivity,
        microphone_vad_adapter: request.microphone_vad_adapter,
        inactivity_activity_mode: request.inactivity_activity_mode,
    })
}

fn recording_settings_file_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(RECORDING_SETTINGS_FILE_NAME);
    }

    PathBuf::from(default_save_directory()).join(RECORDING_SETTINGS_FILE_NAME)
}

fn load_recording_settings_from_path(path: &Path) -> Option<RecordingSettings> {
    load_recording_settings_from_path_with_resolution_support(path, true)
}

fn load_recording_settings_from_path_with_resolution_support(
    path: &Path,
    non_original_resolution_supported: bool,
) -> Option<RecordingSettings> {
    let raw = std::fs::read_to_string(path).ok()?;

    let raw = migrate_legacy_recording_settings_json(&raw);

    let parsed = serde_json::from_str::<RecordingSettings>(&raw).ok()?;
    validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: parsed.capture_screen,
            capture_microphone: parsed.capture_microphone,
            capture_system_audio: parsed.capture_system_audio,
            segment_duration_seconds: parsed.segment_duration_seconds,
            screen_frame_rate: parsed.screen_frame_rate,
            screen_resolution: parsed.screen_resolution,
            video_bitrate: parsed.video_bitrate,
            save_directory: parsed.save_directory,
            auto_start: parsed.auto_start,
            native_capture_debug_logging_enabled: parsed.native_capture_debug_logging_enabled,
            developer_options_enabled: parsed.developer_options_enabled,
            preview_cache_ttl_seconds: parsed.preview_cache_ttl_seconds,
            follow_timeline_live: parsed.follow_timeline_live,
            retention_policy: parsed.retention_policy,
            appearance: parsed.appearance,
            ocr: parsed.ocr,
            transcription: parsed.transcription,
            speaker_analysis: parsed.speaker_analysis,
            pause_capture_on_inactivity: parsed.pause_capture_on_inactivity,
            idle_timeout_seconds: parsed.idle_timeout_seconds,
            microphone_activity_sensitivity: parsed.microphone_activity_sensitivity,
            system_audio_activity_sensitivity: parsed.system_audio_activity_sensitivity,
            microphone_vad_adapter: parsed.microphone_vad_adapter,
            inactivity_activity_mode: parsed.inactivity_activity_mode,
        },
        non_original_resolution_supported,
    )
    .ok()
}

fn migrate_legacy_recording_settings_json(raw: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };
    let Some(obj) = value.as_object_mut() else {
        return raw.to_string();
    };

    if let Some(legacy) = obj.remove("audioActivitySensitivity") {
        if !obj.contains_key("microphoneActivitySensitivity") {
            obj.insert("microphoneActivitySensitivity".to_string(), legacy.clone());
        }
        if !obj.contains_key("systemAudioActivitySensitivity") {
            obj.insert("systemAudioActivitySensitivity".to_string(), legacy);
        }
    }

    if obj
        .get("retentionPolicy")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == "minutes_5" || value == "minutes5")
    {
        obj.insert(
            "retentionPolicy".to_string(),
            serde_json::Value::String("never".to_string()),
        );
    }

    serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string())
}

#[cfg(test)]
fn load_recording_settings_from_path_or_default(path: &Path) -> RecordingSettings {
    load_recording_settings_from_path(path).unwrap_or_else(default_recording_settings)
}

pub(crate) fn load_recording_settings_from_disk(
    app_handle: &tauri::AppHandle,
) -> Option<RecordingSettings> {
    load_recording_settings_from_path(&recording_settings_file_path(app_handle))
}

pub(crate) fn load_recording_settings_or_default(
    app_handle: &tauri::AppHandle,
) -> RecordingSettings {
    load_recording_settings_from_disk(app_handle).unwrap_or_else(default_recording_settings)
}

pub(crate) fn initialize_recording_settings_state_from_disk(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
) -> LoadedRecordingSettings {
    let loaded = match load_recording_settings_from_disk(app_handle) {
        Some(settings) => LoadedRecordingSettings {
            settings,
            source: "disk",
        },
        None => LoadedRecordingSettings {
            settings: default_recording_settings(),
            source: "defaults",
        },
    };

    let mut runtime = state.lock().expect("recording settings state poisoned");
    runtime.settings = loaded.settings.clone();

    loaded
}

pub(crate) fn current_recording_settings(state: &RecordingSettingsState) -> RecordingSettings {
    state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone()
}

pub(crate) fn current_auto_start(state: &RecordingSettingsState) -> bool {
    state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .auto_start
}

pub(crate) fn current_native_capture_debug_logging_enabled(state: &RecordingSettingsState) -> bool {
    state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .native_capture_debug_logging_enabled
}

pub(crate) fn apply_recording_settings_update(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    request: UpdateRecordingSettingsRequest,
) -> Result<AppliedRecordingSettingsUpdate, CaptureErrorResponse> {
    let settings = validate_recording_settings(request)?;
    persist_recording_settings(app_handle, &settings)?;

    let mut runtime = state.lock().expect("recording settings state poisoned");
    let previous_settings = runtime.settings.clone();
    let previous_save_directory = previous_settings.save_directory.clone();
    let save_directory_changed = previous_save_directory != settings.save_directory;
    let debug_logging_enabled_changed = previous_settings.native_capture_debug_logging_enabled
        != settings.native_capture_debug_logging_enabled;

    runtime.settings = settings.clone();

    Ok(AppliedRecordingSettingsUpdate {
        settings,
        previous_settings,
        previous_save_directory,
        save_directory_changed,
        debug_logging_enabled_changed,
    })
}

pub(crate) fn persist_recording_settings(
    app_handle: &tauri::AppHandle,
    settings: &RecordingSettings,
) -> Result<(), CaptureErrorResponse> {
    let file_path = recording_settings_file_path(app_handle);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to create settings directory: {error}"),
        })?;
    }

    let serialized =
        serde_json::to_string_pretty(settings).map_err(|error| CaptureErrorResponse {
            code: "serialization_error".to_string(),
            message: format!("Failed to serialize recording settings: {error}"),
        })?;

    std::fs::write(file_path, serialized).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to persist recording settings: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("desktop-settings-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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
    fn default_recording_settings_disable_native_capture_debug_logging() {
        assert!(!default_recording_settings().native_capture_debug_logging_enabled);
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
    fn load_recording_settings_from_path_preserves_native_capture_debug_logging_flag() {
        let dir = TestDir::new("debug-log-enabled");
        let path = dir.path().join("recording-settings.json");
        let mut settings = default_recording_settings();
        settings.native_capture_debug_logging_enabled = true;

        fs::write(
            &path,
            serde_json::to_string_pretty(&settings).expect("settings should serialize"),
        )
        .expect("settings file should write");

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

        assert!(loaded.native_capture_debug_logging_enabled);
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

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

        assert!(loaded.developer_options_enabled);
    }

    #[test]
    fn load_recording_settings_from_path_preserves_microphone_vad_adapter() {
        let dir = TestDir::new("microphone-vad-adapter");
        let path = dir.path().join("recording-settings.json");
        let mut settings = default_recording_settings();
        settings.microphone_vad_adapter = capture_types::MicrophoneVadAdapter::Webrtc;

        fs::write(
            &path,
            serde_json::to_string_pretty(&settings).expect("settings should serialize"),
        )
        .expect("settings file should write");

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

        assert_eq!(
            loaded.microphone_vad_adapter,
            capture_types::MicrophoneVadAdapter::Webrtc
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

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

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
    fn validate_recording_settings_preserves_microphone_vad_adapter_update() {
        let settings = validate_recording_settings_with_resolution_support(
            UpdateRecordingSettingsRequest {
                capture_screen: true,
                capture_microphone: true,
                capture_system_audio: false,
                segment_duration_seconds: 60,
                screen_frame_rate: 1,
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
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
                ocr: default_ocr_settings(),
                transcription: default_audio_transcription_settings(),
                speaker_analysis: default_speaker_analysis_settings(),
                pause_capture_on_inactivity: true,
                idle_timeout_seconds: 10,
                microphone_activity_sensitivity: 50,
                system_audio_activity_sensitivity: 50,
                microphone_vad_adapter: capture_types::MicrophoneVadAdapter::Off,
                inactivity_activity_mode: default_inactivity_activity_mode(),
            },
            true,
        )
        .expect("settings should validate");

        assert_eq!(
            settings.microphone_vad_adapter,
            capture_types::MicrophoneVadAdapter::Off
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
                screen_frame_rate: 1,
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
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
                ocr,
                transcription: default_audio_transcription_settings(),
                speaker_analysis: default_speaker_analysis_settings(),
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
                screen_frame_rate: 1,
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
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
                ocr: ocr_settings,
                transcription: default_audio_transcription_settings(),
                speaker_analysis: default_speaker_analysis_settings(),
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
                screen_frame_rate: 1,
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
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
                ocr: default_ocr_settings(),
                transcription,
                speaker_analysis: default_speaker_analysis_settings(),
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
    fn default_recording_settings_include_preview_cache_ttl() {
        assert_eq!(
            default_recording_settings().preview_cache_ttl_seconds,
            default_preview_cache_ttl_seconds()
        );
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
                "nativeCaptureDebugLoggingEnabled": false,
                "pauseCaptureOnInactivity": true,
                "idleTimeoutSeconds": 10,
                "microphoneActivitySensitivity": 50,
                "systemAudioActivitySensitivity": 50,
                "activityMode": "system_input_or_screen"
            }"#,
        )
        .expect("settings file should write");

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

        assert_eq!(
            loaded.preview_cache_ttl_seconds,
            default_preview_cache_ttl_seconds()
        );
        assert_eq!(loaded.follow_timeline_live, default_follow_timeline_live());
        assert_eq!(loaded.appearance, default_appearance());
        assert_eq!(loaded.ocr, default_ocr_settings());
        assert_eq!(loaded.transcription, default_audio_transcription_settings());
    }

    #[test]
    fn validate_recording_settings_rejects_preview_cache_ttl_above_max() {
        let error = validate_recording_settings(UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            native_capture_debug_logging_enabled: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: MAX_PREVIEW_CACHE_TTL_SECONDS + 1,
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            ocr: default_ocr_settings(),
            transcription: default_audio_transcription_settings(),
            speaker_analysis: default_speaker_analysis_settings(),
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
}
