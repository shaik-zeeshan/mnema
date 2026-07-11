use capture_types::{
    default_appearance, default_audio_speech_detection_settings,
    default_audio_transcription_settings, default_developer_options_enabled,
    default_follow_timeline_live, default_idle_timeout_seconds, default_metadata_settings,
    default_microphone_activity_sensitivity, default_native_capture_debug_logging_enabled,
    default_ocr_settings, default_ocr_tesseract_char_whitelist,
    default_ocr_tesseract_page_segmentation_mode, default_ocr_tesseract_preprocess_mode,
    default_ocr_tesseract_upscale_factor, default_pause_capture_on_inactivity,
    default_preview_cache_ttl_seconds, default_privacy_settings, default_semantic_search_model_id,
    default_semantic_search_provider, default_speaker_analysis_model_id,
    default_speaker_analysis_settings, default_speaker_analysis_timeout_seconds,
    default_system_audio_activity_sensitivity, default_video_bitrate, AccessSettings,
    AiRuntimeSettings, AudioSpeechDetectionSettings, AudioSpeechDetector, AudioTranscriptionProvider,
    AudioTranscriptionSettings, CaptureErrorResponse, OcrProvider, OcrRecognitionMode, OcrSettings,
    RecordingSettings, RetentionPolicy, ScreenResolution, ScreenResolutionPreset,
    SemanticSearchSettings, SettingsOwnershipDomain, SpeakerAnalysisSettings,
    UpdateAccessSettingsRequest,
    UpdateAiRuntimeSettingsRequest, UpdateCaptureSourceSettingsRequest,
    UpdateCaptureTimingSettingsRequest,
    UpdateDeveloperSettingsRequest, UpdateDisplaySettingsRequest, UpdateInactivitySettingsRequest,
    UpdateMetadataSettingsRequest, UpdateProcessingSettingsRequest, UpdateRecordingSettingsRequest,
    UpdateSemanticSearchSettingsRequest, UpdateStorageSettingsRequest,
    UpdateUserContextSettingsRequest, UpdateVideoSettingsRequest, UserContextSettings,
    VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings,
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
const MAX_SEGMENT_DURATION_SECONDS: u64 = 300;
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

/// The Tauri bundle identifier (`tauri.conf.json` -> `identifier`), used to
/// scope the Windows default capture library under LocalAppData so it matches
/// the convention used for Mnema's other Windows app folders.
#[cfg(windows)]
const APP_IDENTIFIER: &str = "com.shaikzeeshan.mnema";

/// Honor `MNEMA_SAVE_DIRECTORY` verbatim when set, matching the broker/CLI
/// read-only resolver in `crates/app-infra/src/brokered_access.rs`
/// (`default_save_directory_from_config`): any set value is taken as the
/// directory path with no trimming, no `~` expansion, and no emptiness
/// filtering. This keeps the dev sandbox (`dev:sandbox` sets the env var to
/// `$HOME/.mnema-dev`) isolated from the production default root. Checked first
/// on every platform, ahead of the OS-specific default.
fn save_directory_env_override() -> Option<String> {
    std::env::var("MNEMA_SAVE_DIRECTORY")
        .ok()
        .map(|path| PathBuf::from(path).to_string_lossy().to_string())
}

/// `<home>/.mnema`, falling back to a bare relative `.mnema` only when the
/// home directory cannot be resolved. Shared last-resort for both platforms.
///
/// `std::env::home_dir` reads `$HOME` on Unix, so the default lands at
/// `<home>/.mnema` instead of a bare relative `.mnema` when `HOME` is absent.
fn home_dot_mnema_save_directory() -> String {
    std::env::home_dir()
        .map(|home| home.join(".mnema"))
        .unwrap_or_else(|| PathBuf::from(".mnema"))
        .to_string_lossy()
        .to_string()
}

#[cfg(not(windows))]
pub(crate) fn default_save_directory() -> String {
    if let Some(path) = save_directory_env_override() {
        return path;
    }
    // Resolve the user's home directory cross-platform, landing at
    // `<home>/.mnema` (see `home_dot_mnema_save_directory`).
    home_dot_mnema_save_directory()
}

/// Pure Windows default-library logic, isolated from the process environment so
/// it can be unit-tested without mutating the global `LOCALAPPDATA` (which other
/// tests in this multi-threaded binary observe). Prefers
/// `%LOCALAPPDATA%\com.shaikzeeshan.mnema\library`, falling back to the shared
/// `<home>/.mnema` last resort when LocalAppData is unset or empty.
#[cfg(windows)]
fn windows_default_save_directory(local_app_data: Option<std::ffi::OsString>) -> String {
    if let Some(local_app_data) = local_app_data {
        let local_app_data = PathBuf::from(local_app_data);
        if !local_app_data.as_os_str().is_empty() {
            return local_app_data
                .join(APP_IDENTIFIER)
                .join("library")
                .to_string_lossy()
                .to_string();
        }
    }

    home_dot_mnema_save_directory()
}

/// Windows default capture library: a shallow, NON-roaming location under
/// `%LOCALAPPDATA%\com.shaikzeeshan.mnema\library` (see
/// `docs/windows/storage-access-release-research.md`). This avoids AppData
/// roaming for the large capture library, follows Windows conventions, and
/// keeps the path shallow to leave headroom under the legacy `MAX_PATH`
/// (260-char) limit for the nested `recordings/YYYY/MM/DD/` tree underneath it.
///
/// Prefer `%LOCALAPPDATA%` directly to stay dependency-light (this crate does
/// not depend on `dirs`). Fall back to `<USERPROFILE>\.mnema` if LocalAppData is
/// unset, and only as an absolute last resort use a bare relative `.mnema`.
///
/// MIGRATION NOTE: `default_save_directory()` is only the *fresh-install*
/// default. Once any recording setting is saved, `save_directory` is persisted
/// to `recording-settings.json` and that persisted value is reused on every
/// launch (see `initialize_recording_settings_state_from_disk`). Changing this
/// default therefore only affects installs that have never persisted settings.
/// On Windows (unreleased) that means dev-only recordings made under the old
/// `HOME/.mnema` default and never saved would be orphaned on the next launch.
#[cfg(windows)]
pub(crate) fn default_save_directory() -> String {
    if let Some(path) = save_directory_env_override() {
        return path;
    }
    windows_default_save_directory(std::env::var_os("LOCALAPPDATA"))
}

pub(crate) fn default_recording_settings() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 0.5,
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
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: AccessSettings::default(),
        ai_runtime: AiRuntimeSettings::default(),
        user_context: UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: default_pause_capture_on_inactivity(),
        idle_timeout_seconds: default_idle_timeout_seconds(),
        microphone_activity_sensitivity: default_microphone_activity_sensitivity(),
        system_audio_activity_sensitivity: default_system_audio_activity_sensitivity(),
        microphone_vad_adapter: capture_types::default_audio_speech_detector(),
        inactivity_activity_mode: capture_types::InactivityActivityMode::SystemInputOrScreenOrAudio,
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

fn validate_audio_speech_detection_settings(
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

pub(crate) fn validate_privacy_settings(
    value: capture_types::PrivacySettings,
) -> Result<capture_types::PrivacySettings, CaptureErrorResponse> {
    let capture_types::PrivacySettings { excluded_apps } = value;

    let mut seen_app_bundle_ids = std::collections::BTreeSet::new();
    let excluded_apps = excluded_apps
        .into_iter()
        .filter_map(|mut app| {
            app.bundle_id = canonicalize_app_bundle_id(&app.bundle_id);
            if app.bundle_id.is_empty() || !seen_app_bundle_ids.insert(app.bundle_id.clone()) {
                return None;
            }
            Some(app)
        })
        .collect();

    Ok(capture_types::PrivacySettings { excluded_apps })
}

pub(crate) fn canonicalize_app_bundle_id(bundle_id: &str) -> String {
    bundle_id.trim().to_string()
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
fn default_model_id_for_speaker_provider(provider: &str) -> Option<&'static str> {
    match provider {
        speaker_analysis::SPEAKRS_PROVIDER_ID => Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID),
        _ => None,
    }
}

fn validate_speaker_analysis_settings(value: SpeakerAnalysisSettings) -> SpeakerAnalysisSettings {
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
fn validate_semantic_search_settings(value: SemanticSearchSettings) -> SemanticSearchSettings {
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

/// Map a persisted [`OcrProvider`] to its stable provider-id string so the
/// settings path can ask [`ocr::provider_runtime_available`]. Reuses the
/// `ocr::*_PROVIDER_ID` constants (the same ids `ocr::OcrProviderKind::as_str`
/// emits) rather than hardcoding strings.
fn ocr_provider_runtime_id(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => ocr::APPLE_VISION_PROVIDER_ID,
        OcrProvider::Tesseract => ocr::TESSERACT_PROVIDER_ID,
        OcrProvider::PaddleOcr => ocr::PADDLE_OCR_PROVIDER_ID,
    }
}

fn validate_ocr_settings(value: OcrSettings) -> Result<OcrSettings, CaptureErrorResponse> {
    // Runtime-availability guard (the durable fix for OCR defaults across OSes):
    // any persisted provider that cannot actually run on this OS — Apple Vision
    // on Windows/Linux, the legacy PaddleOCR option where it is not built, or any
    // future mismatch — is silently coerced to the platform default. That default
    // (`default_ocr_settings()`) is guaranteed runnable here because its provider
    // comes from `default_ocr_provider()`'s `cfg`-seeded default (Apple Vision on
    // macOS, Tesseract elsewhere). A provider that IS runtime-runnable falls
    // through unchanged into the provider-specific match below.
    if !ocr::provider_runtime_available(ocr_provider_runtime_id(value.provider)) {
        let mut settings = default_ocr_settings();
        settings.enabled = value.enabled;
        return Ok(settings);
    }

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
            // Reached only when PaddleOCR IS runtime-available on this build (e.g.
            // the `paddle-rs`-enabled non-Windows target); the runtime guard above
            // already coerces it where it cannot run. PaddleOCR remains available
            // in the OCR crate for existing queued jobs and direct provider tests,
            // but it is no longer a user-selectable recording setting, so legacy
            // persisted settings are still normalized back to the supported default.
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
            (width as f64) * (height as f64) * settings.screen_frame_rate * factor
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

fn current_capture_support_capabilities() -> (bool, bool) {
    let screen_support = capture_screen::support_for_current_platform();
    (
        screen_support.non_original_resolution,
        super::system_audio_requires_screen_for_platform(&screen_support.platform),
    )
}

pub(crate) fn validate_recording_settings(
    request: UpdateRecordingSettingsRequest,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let (non_original_resolution_supported, system_audio_requires_screen) =
        current_capture_support_capabilities();
    validate_recording_settings_with_capture_support(
        request,
        non_original_resolution_supported,
        system_audio_requires_screen,
    )
}

pub(crate) fn validate_recording_settings_with_capture_support(
    request: UpdateRecordingSettingsRequest,
    non_original_resolution_supported: bool,
    system_audio_requires_screen: bool,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    if !request.capture_screen && !request.capture_microphone && !request.capture_system_audio {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "At least one capture source must be enabled".to_string(),
        });
    }

    if system_audio_requires_screen && request.capture_system_audio && !request.capture_screen {
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

    if !(1..=MAX_SEGMENT_DURATION_SECONDS).contains(&request.segment_duration_seconds) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: format!(
                "segmentDurationSeconds must be between 1 and {MAX_SEGMENT_DURATION_SECONDS}"
            ),
        });
    }

    if !(0.5..=10.0).contains(&request.screen_frame_rate) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "screenFrameRate must be between 0.5 and 10".to_string(),
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
            message: "Selected screen resolution is not supported by the active native capture backend. Only the original display resolution is supported on this system.".to_string(),
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
/// their ids differ, so same-kind providers can coexist.
fn normalize_ai_runtime_settings(mut ai_runtime: AiRuntimeSettings) -> AiRuntimeSettings {
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
    ai_runtime
}

pub(crate) fn recording_settings_file_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(RECORDING_SETTINGS_FILE_NAME);
    }

    PathBuf::from(default_save_directory()).join(RECORDING_SETTINGS_FILE_NAME)
}

fn load_recording_settings_from_path(path: &Path) -> Option<RecordingSettings> {
    let (non_original_resolution_supported, system_audio_requires_screen) =
        current_capture_support_capabilities();
    load_recording_settings_from_path_with_capture_support(
        path,
        non_original_resolution_supported,
        system_audio_requires_screen,
    )
}

fn load_recording_settings_from_path_with_capture_support(
    path: &Path,
    non_original_resolution_supported: bool,
    system_audio_requires_screen: bool,
) -> Option<RecordingSettings> {
    let raw = std::fs::read_to_string(path).ok()?;

    let raw = migrate_legacy_recording_settings_json(&raw);

    // ADR 0034: a legacy engine-centric `aiRuntime` block (engineKind/cloud*/
    // local* + additionalEngines) migrates into the provider list inside
    // `AiRuntimeSettings`' deserialization; the next save rewrites only the new
    // shape. Log once at load so the upgrade is visible.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
        if let Some(ai_runtime) = value.get("aiRuntime").and_then(|value| value.as_object()) {
            if !ai_runtime.contains_key("providers") && ai_runtime.contains_key("engineKind") {
                tauri_plugin_log::log::info!(
                    "migrating legacy engine-centric aiRuntime settings to the provider-centric shape (rewritten on next save)"
                );
            }
        }
    }

    let parsed = serde_json::from_str::<RecordingSettings>(&raw).ok()?;
    validate_recording_settings_with_capture_support(
        UpdateRecordingSettingsRequest {
            capture_screen: parsed.capture_screen,
            capture_microphone: parsed.capture_microphone,
            capture_system_audio: parsed.capture_system_audio,
            segment_duration_seconds: parsed
                .segment_duration_seconds
                .clamp(1, MAX_SEGMENT_DURATION_SECONDS),
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
            audio_speech_detection: parsed.audio_speech_detection,
            metadata: parsed.metadata,
            privacy: parsed.privacy,
            access: parsed.access,
            ai_runtime: parsed.ai_runtime,
            user_context: parsed.user_context,
            semantic_search: parsed.semantic_search,
            pause_capture_on_inactivity: parsed.pause_capture_on_inactivity,
            idle_timeout_seconds: parsed.idle_timeout_seconds,
            microphone_activity_sensitivity: parsed.microphone_activity_sensitivity,
            system_audio_activity_sensitivity: parsed.system_audio_activity_sensitivity,
            microphone_vad_adapter: parsed.microphone_vad_adapter,
            inactivity_activity_mode: parsed.inactivity_activity_mode,
        },
        non_original_resolution_supported,
        system_audio_requires_screen,
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

    if !obj.contains_key("audioSpeechDetection") {
        if let Some(legacy) = obj.get("microphoneVadAdapter").cloned() {
            obj.insert(
                "audioSpeechDetection".to_string(),
                serde_json::json!({ "detector": legacy }),
            );
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

pub(crate) enum RecordingSettingsDomainPatch {
    CaptureSources(UpdateCaptureSourceSettingsRequest),
    CaptureTiming(UpdateCaptureTimingSettingsRequest),
    Video(UpdateVideoSettingsRequest),
    Storage(UpdateStorageSettingsRequest),
    Display(UpdateDisplaySettingsRequest),
    Metadata(UpdateMetadataSettingsRequest),
    Inactivity(UpdateInactivitySettingsRequest),
    Processing(UpdateProcessingSettingsRequest),
    Developer(UpdateDeveloperSettingsRequest),
    Access(UpdateAccessSettingsRequest),
    AiRuntime(UpdateAiRuntimeSettingsRequest),
    UserContext(UpdateUserContextSettingsRequest),
    /// Generic **Semantic Search** settings patch from the untrusted IPC boundary
    /// (`update_semantic_search_settings`). Honors the `enabled` toggle and other
    /// non-dimension-affecting fields, but DELIBERATELY ignores `model_id` /
    /// `provider`: changing the model through a generic patch would re-open the
    /// non-atomic dimension split (the persisted `model_id` moves but the `vec0`
    /// table is not rebuilt to the new model's dimension). Model/provider changes
    /// must go through the dedicated atomic switch (`select_semantic_search_model`
    /// → [`Self::SemanticSearchModelSwitch`]), which rebuilds the table and
    /// persists the selection together. See review finding low #4 (PR #126).
    SemanticSearch(UpdateSemanticSearchSettingsRequest),
    /// Trusted **Semantic Search Model Tier** persist from the atomic switch
    /// (`select_semantic_search_model` via `persist_semantic_search_settings`),
    /// which has ALREADY rebuilt the `vec0` table to the new model's dimension
    /// before persisting. This variant honors `model_id` / `provider` so the
    /// persisted selection lands in lockstep with the live table dimension. It is
    /// internal-only — there is no `#[tauri::command]` that constructs it.
    SemanticSearchModelSwitch(UpdateSemanticSearchSettingsRequest),
}

impl RecordingSettingsDomainPatch {
    pub(crate) fn domain(&self) -> SettingsOwnershipDomain {
        match self {
            Self::CaptureSources(_) => SettingsOwnershipDomain::CaptureSources,
            Self::CaptureTiming(_) => SettingsOwnershipDomain::CaptureTiming,
            Self::Video(_) => SettingsOwnershipDomain::Video,
            Self::Storage(_) => SettingsOwnershipDomain::Storage,
            Self::Display(_) => SettingsOwnershipDomain::Display,
            Self::Metadata(_) => SettingsOwnershipDomain::Metadata,
            Self::Inactivity(_) => SettingsOwnershipDomain::Inactivity,
            Self::Processing(_) => SettingsOwnershipDomain::Processing,
            Self::Developer(_) => SettingsOwnershipDomain::Developer,
            Self::Access(_) => SettingsOwnershipDomain::Access,
            Self::AiRuntime(_) => SettingsOwnershipDomain::AiRuntime,
            Self::UserContext(_) => SettingsOwnershipDomain::UserContext,
            Self::SemanticSearch(_) => SettingsOwnershipDomain::SemanticSearch,
            Self::SemanticSearchModelSwitch(_) => SettingsOwnershipDomain::SemanticSearch,
        }
    }
}

fn empty_domain_patch_error(domain: SettingsOwnershipDomain) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "empty_settings_patch".to_string(),
        message: format!("{domain:?} settings patch must include at least one field"),
    }
}

fn recording_settings_request_from_settings(
    settings: RecordingSettings,
) -> UpdateRecordingSettingsRequest {
    UpdateRecordingSettingsRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
        segment_duration_seconds: settings.segment_duration_seconds,
        screen_frame_rate: settings.screen_frame_rate,
        screen_resolution: settings.screen_resolution,
        video_bitrate: settings.video_bitrate,
        save_directory: settings.save_directory,
        auto_start: settings.auto_start,
        native_capture_debug_logging_enabled: settings.native_capture_debug_logging_enabled,
        developer_options_enabled: settings.developer_options_enabled,
        preview_cache_ttl_seconds: settings.preview_cache_ttl_seconds,
        follow_timeline_live: settings.follow_timeline_live,
        retention_policy: settings.retention_policy,
        appearance: settings.appearance,
        ocr: settings.ocr,
        transcription: settings.transcription,
        speaker_analysis: settings.speaker_analysis,
        audio_speech_detection: settings.audio_speech_detection,
        metadata: settings.metadata,
        privacy: settings.privacy,
        access: settings.access,
        ai_runtime: settings.ai_runtime,
        user_context: settings.user_context,
        semantic_search: settings.semantic_search,
        pause_capture_on_inactivity: settings.pause_capture_on_inactivity,
        idle_timeout_seconds: settings.idle_timeout_seconds,
        microphone_activity_sensitivity: settings.microphone_activity_sensitivity,
        system_audio_activity_sensitivity: settings.system_audio_activity_sensitivity,
        microphone_vad_adapter: settings.microphone_vad_adapter,
        inactivity_activity_mode: settings.inactivity_activity_mode,
    }
}

fn apply_domain_patch_to_settings(
    settings: &mut RecordingSettings,
    patch: RecordingSettingsDomainPatch,
) -> Result<(), CaptureErrorResponse> {
    let domain = patch.domain();
    let mut touched = false;

    match patch {
        RecordingSettingsDomainPatch::CaptureSources(request) => {
            if let Some(value) = request.capture_screen {
                settings.capture_screen = value;
                touched = true;
            }
            if let Some(value) = request.capture_microphone {
                settings.capture_microphone = value;
                touched = true;
            }
            if let Some(value) = request.capture_system_audio {
                settings.capture_system_audio = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::CaptureTiming(request) => {
            if let Some(value) = request.segment_duration_seconds {
                settings.segment_duration_seconds = value;
                touched = true;
            }
            if let Some(value) = request.auto_start {
                settings.auto_start = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Video(request) => {
            if let Some(value) = request.screen_frame_rate {
                settings.screen_frame_rate = value;
                touched = true;
            }
            if let Some(value) = request.screen_resolution {
                settings.screen_resolution = value;
                touched = true;
            }
            if let Some(value) = request.video_bitrate {
                settings.video_bitrate = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Storage(request) => {
            if let Some(value) = request.save_directory {
                settings.save_directory = value;
                touched = true;
            }
            if let Some(value) = request.retention_policy {
                settings.retention_policy = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Display(request) => {
            if let Some(value) = request.appearance {
                settings.appearance = value;
                touched = true;
            }
            if let Some(value) = request.follow_timeline_live {
                settings.follow_timeline_live = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Metadata(request) => {
            if let Some(value) = request.enabled {
                settings.metadata.enabled = value;
                touched = true;
            }
            if let Some(value) = request.browser_url_mode {
                settings.metadata.browser_url_mode = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Inactivity(request) => {
            if let Some(value) = request.pause_capture_on_inactivity {
                settings.pause_capture_on_inactivity = value;
                touched = true;
            }
            if let Some(value) = request.idle_timeout_seconds {
                settings.idle_timeout_seconds = value;
                touched = true;
            }
            if let Some(value) = request.microphone_activity_sensitivity {
                settings.microphone_activity_sensitivity = value;
                touched = true;
            }
            if let Some(value) = request.system_audio_activity_sensitivity {
                settings.system_audio_activity_sensitivity = value;
                touched = true;
            }
            if let Some(value) = request.audio_speech_detection {
                settings.audio_speech_detection = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Processing(request) => {
            if let Some(value) = request.ocr {
                settings.ocr = value;
                touched = true;
            }
            if let Some(value) = request.transcription {
                settings.transcription = value;
                touched = true;
            }
            if let Some(value) = request.speaker_analysis {
                settings.speaker_analysis = value;
                touched = true;
            }
            if let Some(value) = request.preview_cache_ttl_seconds {
                settings.preview_cache_ttl_seconds = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Access(request) => {
            if let Some(value) = request.ask_ai_enabled {
                settings.access.ask_ai_enabled = value;
                touched = true;
            }
            if let Some(value) = request.ask_ai_web_fetch_enabled {
                settings.access.ask_ai_web_fetch_enabled = value;
                touched = true;
            }
            if let Some(value) = request.ask_ai_max_tool_calls {
                settings.access.ask_ai_max_tool_calls = value;
                touched = true;
            }
            if let Some(value) = request.ask_ai_model {
                let trimmed = value.trim();
                settings.access.ask_ai_model = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::AiRuntime(request) => {
            if let Some(value) = request.enabled {
                settings.ai_runtime.enabled = value;
                touched = true;
            }
            if let Some(value) = request.providers {
                // Replacement provider list (wholesale, like additionalEngines
                // before it); normalization (trim/dedupe) happens in validation.
                settings.ai_runtime.providers = value;
                touched = true;
            }
            if let Some(value) = request.mcp_servers {
                // Replacement MCP connector list (wholesale, like providers).
                settings.ai_runtime.mcp_servers = value;
                touched = true;
            }
            if let Some(value) = request.default_model {
                // Double-Option: an explicit `null` clears the default model.
                settings.ai_runtime.default_model = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::UserContext(request) => {
            if let Some(value) = request.enabled {
                settings.user_context.enabled = value;
                touched = true;
            }
            if let Some(value) = request.derivation_budget_tier {
                settings.user_context.derivation_budget_tier = value;
                touched = true;
            }
            if let Some(value) = request.backfill_window_days {
                settings.user_context.backfill_window_days = value;
                touched = true;
            }
            if let Some(value) = request.backfill_go_deeper {
                settings.user_context.backfill_go_deeper = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::SemanticSearch(request) => {
            if let Some(value) = request.enabled {
                settings.semantic_search.enabled = value;
                touched = true;
            }
            // Ignore `model_id` / `provider` from this generic IPC patch. Changing
            // the model here would re-open the non-atomic dimension split: the
            // persisted `model_id` would move but the `vec0` table is NOT rebuilt
            // to the new model's dimension, so the index width would disagree with
            // the selection (search degrades to keyword-only until startup
            // reconciliation, and vectors could be discarded). Model/provider
            // changes must go through the dedicated atomic switch
            // (`select_semantic_search_model`), which rebuilds the table and
            // persists the selection together. We ignore-with-log rather than
            // error so existing callers that harmlessly echo the current value
            // don't break.
            if request.model_id.is_some() || request.provider.is_some() {
                crate::native_capture::debug_log::log_info(
                    "semantic search: ignoring `model_id`/`provider` in a generic settings update; \
                     model/provider changes must go through the atomic switch \
                     (`select_semantic_search_model`) that rebuilds the vec0 table at the new dimension",
                );
            }
        }
        RecordingSettingsDomainPatch::SemanticSearchModelSwitch(request) => {
            // Trusted path: `select_semantic_search_model` has ALREADY rebuilt the
            // `vec0` table to the new model's dimension before persisting, so the
            // selection lands in lockstep with the live table width. Honor every
            // field, including `model_id` / `provider`.
            if let Some(value) = request.enabled {
                settings.semantic_search.enabled = value;
                touched = true;
            }
            if let Some(value) = request.provider {
                settings.semantic_search.provider = value;
                touched = true;
            }
            if let Some(value) = request.model_id {
                // Double-Option: an explicit `null` clears the selected model.
                settings.semantic_search.model_id = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Developer(request) => {
            if let Some(value) = request.developer_options_enabled {
                settings.developer_options_enabled = value;
                touched = true;
            }
            if let Some(value) = request.native_capture_debug_logging_enabled {
                settings.native_capture_debug_logging_enabled = value;
                touched = true;
            }
        }
    }

    if !touched {
        return Err(empty_domain_patch_error(domain));
    }

    Ok(())
}

pub(crate) fn apply_recording_settings_update(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    request: UpdateRecordingSettingsRequest,
) -> Result<AppliedRecordingSettingsUpdate, CaptureErrorResponse> {
    let requested_privacy = validate_privacy_settings(request.privacy.clone())?;
    let settings = validate_recording_settings(request)?;

    let mut runtime = state.lock().expect("recording settings state poisoned");
    if requested_privacy != runtime.settings.privacy {
        return Err(CaptureErrorResponse {
            code: "invalid_privacy_rule".to_string(),
            message: "Privacy rules must be changed with dedicated privacy commands".to_string(),
        });
    }

    let previous_settings = runtime.settings.clone();
    let previous_save_directory = previous_settings.save_directory.clone();
    let save_directory_changed = previous_save_directory != settings.save_directory;
    let debug_logging_enabled_changed = previous_settings.native_capture_debug_logging_enabled
        != settings.native_capture_debug_logging_enabled;

    persist_recording_settings(app_handle, &settings)?;
    runtime.settings = settings.clone();

    Ok(AppliedRecordingSettingsUpdate {
        settings,
        previous_settings,
        previous_save_directory,
        save_directory_changed,
        debug_logging_enabled_changed,
    })
}

pub(crate) fn apply_recording_settings_domain_patch(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    patch: RecordingSettingsDomainPatch,
) -> Result<(SettingsOwnershipDomain, AppliedRecordingSettingsUpdate), CaptureErrorResponse> {
    let domain = patch.domain();
    apply_recording_settings_domain_mutation(app_handle, state, domain, |settings| {
        apply_domain_patch_to_settings(settings, patch)?;
        Ok(())
    })
    .map(|update| (domain, update))
}

pub(crate) fn apply_recording_settings_domain_mutation(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    domain: SettingsOwnershipDomain,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<AppliedRecordingSettingsUpdate, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("recording settings state poisoned");
    let mut next_settings = runtime.settings.clone();

    mutate(&mut next_settings)?;

    let settings =
        validate_recording_settings(recording_settings_request_from_settings(next_settings))?;
    let previous_settings = runtime.settings.clone();
    let previous_save_directory = previous_settings.save_directory.clone();
    let save_directory_changed = previous_save_directory != settings.save_directory;
    let debug_logging_enabled_changed = previous_settings.native_capture_debug_logging_enabled
        != settings.native_capture_debug_logging_enabled;

    if domain != SettingsOwnershipDomain::AppPrivacyExclusion
        && previous_settings.privacy != settings.privacy
    {
        return Err(CaptureErrorResponse {
            code: "invalid_privacy_rule".to_string(),
            message: "Privacy rules must be changed with dedicated privacy commands".to_string(),
        });
    }

    persist_recording_settings(app_handle, &settings)?;
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
    persist_recording_settings_to_path(&file_path, settings)
}

pub(crate) fn persist_recording_settings_to_path(
    file_path: &Path,
    settings: &RecordingSettings,
) -> Result<(), CaptureErrorResponse> {
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
    use capture_types::default_inactivity_activity_mode;
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
    fn canonicalize_app_bundle_id_trim_only_is_windows_path_safe() {
        // On Windows (ADR 0043) the canonical executable path is stored opaquely
        // in `app_bundle_id`. Canonicalization is trim-only, so it must preserve
        // backslashes, the drive letter, and internal casing/spaces — anything
        // else would corrupt the path grouping key. (Case-insensitive `app:`
        // matching is handled at query time by `LOWER(TRIM(...))`, not here.)
        assert_eq!(
            canonicalize_app_bundle_id(
                "  C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe  "
            ),
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"
        );
        // A macOS bundle id is likewise only trimmed, unchanged otherwise.
        assert_eq!(
            canonicalize_app_bundle_id("  com.google.Chrome "),
            "com.google.Chrome"
        );
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

    #[cfg(not(windows))]
    #[test]
    fn default_save_directory_uses_home_dot_mnema_on_unix() {
        let save_directory = default_save_directory();
        assert!(
            save_directory.ends_with(".mnema"),
            "expected default save directory to end with .mnema, got {save_directory}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_save_directory_uses_local_app_data_library() {
        // Exercise the pure helper directly so we never mutate the process-global
        // `LOCALAPPDATA`, which a concurrently-running sibling test reading
        // `default_save_directory()` could otherwise observe (this binary runs
        // tests multi-threaded).
        let save_directory =
            windows_default_save_directory(Some(std::ffi::OsString::from(
                r"C:\Users\test\AppData\Local",
            )));

        assert_eq!(
            save_directory,
            r"C:\Users\test\AppData\Local\com.shaikzeeshan.mnema\library"
        );
        assert!(
            save_directory.contains(APP_IDENTIFIER),
            "expected default save directory to contain the bundle identifier, got {save_directory}"
        );
        assert!(
            save_directory.ends_with("library"),
            "expected default save directory to end with the library subfolder, got {save_directory}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_save_directory_falls_back_for_empty_local_app_data() {
        let save_directory =
            windows_default_save_directory(Some(std::ffi::OsString::new()));

        assert!(
            save_directory.ends_with(".mnema"),
            "expected empty LocalAppData to fall back to the home/.mnema path, got {save_directory}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_save_directory_falls_back_for_missing_local_app_data() {
        let save_directory = windows_default_save_directory(None);

        assert!(
            save_directory.ends_with(".mnema"),
            "expected missing LocalAppData to fall back to the home/.mnema path, got {save_directory}"
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

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

        assert!(loaded.access.ask_ai_enabled);
    }

    #[test]
    fn load_recording_settings_from_path_migrates_legacy_microphone_vad_adapter_to_shared_detector()
    {
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

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

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

    fn apply_domain_patch_for_test_with_capture_support(
        mut settings: RecordingSettings,
        patch: RecordingSettingsDomainPatch,
        system_audio_requires_screen: bool,
    ) -> Result<RecordingSettings, CaptureErrorResponse> {
        apply_domain_patch_to_settings(&mut settings, patch)?;
        validate_recording_settings_with_capture_support(
            recording_settings_request_from_settings(settings),
            true,
            system_audio_requires_screen,
        )
    }

    fn apply_domain_patch_for_test(
        settings: RecordingSettings,
        patch: RecordingSettingsDomainPatch,
    ) -> Result<RecordingSettings, CaptureErrorResponse> {
        apply_domain_patch_for_test_with_capture_support(settings, patch, true)
    }

    /// OCR settings as a caller observes them after settings normalization.
    /// `validate_ocr_settings` intentionally enriches the Tesseract provider
    /// (the Windows default) with its pinned model id and language, so
    /// preserve-unrelated-fields tests must compare against the normalized
    /// form of their base OCR settings rather than the raw default (which
    /// carries `model_id: None` / `language: None`). On macOS the Apple
    /// Vision default normalizes to itself, so this is a no-op there.
    fn normalized_ocr_settings(ocr: OcrSettings) -> OcrSettings {
        validate_ocr_settings(ocr).expect("test base ocr settings should validate")
    }

    #[test]
    fn domain_update_preserves_unrelated_settings_fields() {
        let mut base = default_recording_settings();
        base.capture_microphone = true;
        base.ocr.enabled = false;
        base.appearance = capture_types::AppearanceSetting::Dark;
        base.save_directory = "/tmp/mnema-before".to_string();

        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
                capture_screen: None,
                capture_microphone: Some(false),
                capture_system_audio: None,
            }),
        )
        .expect("capture source patch should validate");

        assert!(!updated.capture_microphone);
        assert_eq!(updated.capture_screen, base.capture_screen);
        assert_eq!(updated.ocr, normalized_ocr_settings(base.ocr.clone()));
        assert_eq!(updated.appearance, base.appearance);
        assert_eq!(updated.save_directory, base.save_directory);
    }

    #[test]
    fn semantic_search_model_switch_patch_switches_model_and_preserves_other_fields() {
        let mut base = default_recording_settings();
        base.capture_microphone = true;
        base.save_directory = "/tmp/mnema-before".to_string();
        assert_eq!(
            base.semantic_search.model_id.as_deref(),
            Some("nomic-embed-text-v1.5")
        );

        // Switch to the Multilingual tier via the trusted atomic-switch variant
        // (the path `select_semantic_search_model` uses AFTER it has rebuilt the
        // vec0 table to the new model's dimension).
        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::SemanticSearchModelSwitch(
                capture_types::UpdateSemanticSearchSettingsRequest {
                    enabled: None,
                    provider: None,
                    model_id: Some(Some("multilingual-e5-small".to_string())),
                },
            ),
        )
        .expect("semantic search model switch should validate");

        assert_eq!(
            updated.semantic_search.model_id.as_deref(),
            Some("multilingual-e5-small")
        );
        // Unrelated fields are untouched.
        assert!(updated.capture_microphone);
        assert_eq!(updated.save_directory, base.save_directory);
        assert_eq!(updated.ocr, normalized_ocr_settings(base.ocr.clone()));
    }

    #[test]
    fn generic_semantic_search_patch_ignores_model_id_and_provider() {
        let mut base = default_recording_settings();
        base.semantic_search.enabled = false;
        let original_model = base.semantic_search.model_id.clone();
        let original_provider = base.semantic_search.provider.clone();
        assert!(original_model.is_some());

        // A generic IPC patch that carries `model_id`/`provider` must NOT change
        // the persisted model or provider (changing them here would re-open the
        // non-atomic dimension split). The honored fields (here `enabled`) still
        // apply, so the patch is non-empty and validates.
        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::SemanticSearch(
                capture_types::UpdateSemanticSearchSettingsRequest {
                    enabled: Some(true),
                    provider: Some("some-other-provider".to_string()),
                    model_id: Some(Some("bge-m3".to_string())),
                },
            ),
        )
        .expect("generic semantic search patch should validate");

        assert!(updated.semantic_search.enabled, "enabled toggle is honored");
        assert_eq!(
            updated.semantic_search.model_id, original_model,
            "model_id must be ignored by the generic IPC patch"
        );
        assert_eq!(
            updated.semantic_search.provider, original_provider,
            "provider must be ignored by the generic IPC patch"
        );
    }

    #[test]
    fn generic_semantic_search_patch_with_only_model_id_is_rejected_as_empty() {
        // A generic patch whose ONLY fields are the now-ignored `model_id` /
        // `provider` leaves nothing honored, so it is rejected as an empty patch
        // rather than silently no-op'ing the persist.
        let mut base = default_recording_settings();
        let error = apply_domain_patch_to_settings(
            &mut base,
            RecordingSettingsDomainPatch::SemanticSearch(
                capture_types::UpdateSemanticSearchSettingsRequest {
                    enabled: None,
                    provider: Some("some-other-provider".to_string()),
                    model_id: Some(Some("bge-m3".to_string())),
                },
            ),
        )
        .expect_err("a patch carrying only ignored fields must be rejected");
        assert_eq!(error.code, "empty_settings_patch");
        // The persisted model is left untouched by the rejected patch.
        assert_eq!(
            base.semantic_search.model_id.as_deref(),
            Some("nomic-embed-text-v1.5")
        );
    }

    #[test]
    fn semantic_search_model_switch_can_clear_and_toggle() {
        let base = default_recording_settings();

        // An explicit null clears the selected model; disabling the feature flips
        // `enabled`. This is the trusted atomic-switch variant, so model_id is
        // honored.
        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::SemanticSearchModelSwitch(
                capture_types::UpdateSemanticSearchSettingsRequest {
                    enabled: Some(false),
                    provider: None,
                    model_id: Some(None),
                },
            ),
        )
        .expect("semantic search model switch should validate");

        assert!(!updated.semantic_search.enabled);
        assert_eq!(updated.semantic_search.model_id, None);
    }

    #[test]
    fn empty_semantic_search_patch_is_rejected() {
        let mut base = default_recording_settings();
        let error = apply_domain_patch_to_settings(
            &mut base,
            RecordingSettingsDomainPatch::SemanticSearch(
                capture_types::UpdateSemanticSearchSettingsRequest::default(),
            ),
        )
        .expect_err("an empty patch must be rejected");
        assert_eq!(error.code, "empty_settings_patch");
    }

    #[test]
    fn access_domain_update_preserves_unrelated_settings_fields() {
        let mut base = default_recording_settings();
        base.capture_microphone = true;
        base.ocr.enabled = false;
        base.appearance = capture_types::AppearanceSetting::Dark;
        base.save_directory = "/tmp/mnema-before".to_string();

        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::Access(UpdateAccessSettingsRequest {
                ask_ai_enabled: Some(true),
                ask_ai_max_tool_calls: Some(0),
                ask_ai_web_fetch_enabled: Some(true),
                ask_ai_model: Some("anthropic:claude-opus-4".to_string()),
            }),
        )
        .expect("access patch should validate");

        assert!(updated.access.ask_ai_enabled);
        assert!(updated.access.ask_ai_web_fetch_enabled);
        assert_eq!(updated.access.ask_ai_max_tool_calls, 0);
        assert_eq!(
            updated.access.ask_ai_model.as_deref(),
            Some("anthropic:claude-opus-4")
        );
        assert_eq!(updated.capture_microphone, base.capture_microphone);
        assert_eq!(updated.ocr, normalized_ocr_settings(base.ocr.clone()));
        assert_eq!(updated.appearance, base.appearance);
        assert_eq!(updated.save_directory, base.save_directory);
    }

    /// Provider-centric AI runtime settings with the master switch ON, for the
    /// wipe-flips-switch and AiRuntime patch tests.
    fn enabled_ai_runtime_settings() -> AiRuntimeSettings {
        AiRuntimeSettings {
            enabled: true,
            providers: vec![capture_types::AiProviderConfig {
                id: "anthropic".to_string(),
                kind: capture_types::AiProviderKind::Anthropic,
                label: String::new(),
                base_url: String::new(),
            }],
            default_model: Some(capture_types::AiEngineRef {
                provider: "anthropic".to_string(),
                model: "claude-haiku-4-5".to_string(),
            }),
            mcp_servers: Vec::new(),
        }
    }

    #[test]
    fn ai_runtime_patch_flips_master_switch_off_for_wipe_user_context() {
        // Wipe User Context turns the master AI switch off through this exact
        // patch (`UpdateAiRuntimeSettingsRequest { enabled: Some(false), .. }`
        // in `wipe_user_context`); the flip must only touch `enabled`, leaving
        // the connected providers and default model intact for a re-opt-in.
        let mut base = default_recording_settings();
        base.ai_runtime = enabled_ai_runtime_settings();

        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
                enabled: Some(false),
                ..Default::default()
            }),
        )
        .expect("ai runtime patch should validate");

        assert!(!updated.ai_runtime.enabled);
        assert_eq!(updated.ai_runtime.providers, base.ai_runtime.providers);
        assert_eq!(
            updated.ai_runtime.default_model,
            base.ai_runtime.default_model
        );
    }

    #[test]
    fn ai_runtime_patch_replaces_providers_and_clears_default_model() {
        let mut base = default_recording_settings();
        base.ai_runtime = enabled_ai_runtime_settings();

        let updated = apply_domain_patch_for_test(
            base,
            RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
                enabled: None,
                providers: Some(vec![
                    capture_types::AiProviderConfig {
                        id: "ollama".to_string(),
                        kind: capture_types::AiProviderKind::Ollama,
                        label: String::new(),
                        base_url: " http://localhost:11434 ".to_string(),
                    },
                    // A second instance of the SAME kind with a distinct id is
                    // kept — same-kind providers coexist (e.g. two local boxes).
                    capture_types::AiProviderConfig {
                        id: "ollama-2".to_string(),
                        kind: capture_types::AiProviderKind::Ollama,
                        label: "Other box".to_string(),
                        base_url: "http://other:11434".to_string(),
                    },
                    // A duplicate *instance id* is dropped (first wins).
                    capture_types::AiProviderConfig {
                        id: "ollama".to_string(),
                        kind: capture_types::AiProviderKind::Ollama,
                        label: String::new(),
                        base_url: "http://dupe:11434".to_string(),
                    },
                ]),
                // Explicit `null` over the wire clears the default model.
                default_model: Some(None),
                mcp_servers: None,
            }),
        )
        .expect("ai runtime patch should validate");

        assert!(updated.ai_runtime.enabled);
        assert_eq!(
            updated.ai_runtime.providers,
            vec![
                capture_types::AiProviderConfig {
                    id: "ollama".to_string(),
                    kind: capture_types::AiProviderKind::Ollama,
                    label: String::new(),
                    base_url: "http://localhost:11434".to_string(),
                },
                capture_types::AiProviderConfig {
                    id: "ollama-2".to_string(),
                    kind: capture_types::AiProviderKind::Ollama,
                    label: "Other box".to_string(),
                    base_url: "http://other:11434".to_string(),
                },
            ]
        );
        assert_eq!(updated.ai_runtime.default_model, None);
    }

    #[test]
    fn ai_runtime_patch_persists_mcp_servers_on_some() {
        // The MCP connector list is what reconcile/warm/turn all read. The other
        // ai_runtime patch tests pass `mcp_servers: None`, so the wholesale-replace
        // branch (native_capture_settings.rs) never runs — a regression that
        // dropped the write would go unnoticed. Exercise the `Some` branch.
        let mut base = default_recording_settings();
        base.ai_runtime = enabled_ai_runtime_settings();

        let server = capture_types::McpServerConfig {
            id: "connector".to_string(),
            label: "GitHub".to_string(),
            enabled: true,
            transport: capture_types::McpTransport::Stdio,
            auth_mode: capture_types::McpAuthMode::Bearer,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "@modelcontextprotocol/server-github".to_string()],
            env: Vec::new(),
            url: None,
            secret_env_name: Some("GITHUB_TOKEN".to_string()),
            enabled_tools: Some(vec!["search".to_string()]),
        };

        let updated = apply_domain_patch_for_test(
            base,
            RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
                enabled: None,
                providers: None,
                default_model: None,
                mcp_servers: Some(vec![server.clone()]),
            }),
        )
        .expect("ai runtime patch should validate");

        assert_eq!(updated.ai_runtime.mcp_servers, vec![server]);
    }

    #[test]
    fn ai_runtime_patch_mcp_servers_none_leaves_existing_list_unchanged() {
        // `mcp_servers: None` means "leave unchanged", not "clear". This exact
        // partial-update shape is reachable via `wipe_user_context`, whose
        // `enabled: Some(false)` patch must not drop configured connectors.
        let mut base = default_recording_settings();
        base.ai_runtime = enabled_ai_runtime_settings();
        base.ai_runtime.mcp_servers = vec![capture_types::McpServerConfig {
            id: "connector".to_string(),
            label: "GitHub".to_string(),
            enabled: true,
            transport: capture_types::McpTransport::Stdio,
            auth_mode: capture_types::McpAuthMode::Bearer,
            command: Some("npx".to_string()),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-github".to_string(),
            ],
            env: Vec::new(),
            url: None,
            secret_env_name: Some("GITHUB_TOKEN".to_string()),
            enabled_tools: Some(vec!["search".to_string()]),
        }];

        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
                enabled: Some(false),
                ..Default::default()
            }),
        )
        .expect("ai runtime patch should validate");

        assert!(!updated.ai_runtime.enabled);
        assert_eq!(
            updated.ai_runtime.mcp_servers,
            base.ai_runtime.mcp_servers,
            "mcp_servers: None must leave the existing connector list unchanged"
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
                "nativeCaptureDebugLoggingEnabled": false,
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

        let loaded =
            load_recording_settings_from_path(&path).expect("settings should load from disk");

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
    fn empty_access_domain_patch_is_rejected() {
        let error = apply_domain_patch_for_test(
            default_recording_settings(),
            RecordingSettingsDomainPatch::Access(UpdateAccessSettingsRequest::default()),
        )
        .expect_err("empty access patch should be rejected");

        assert_eq!(error.code, "empty_settings_patch");
    }

    #[test]
    fn empty_domain_patch_is_rejected() {
        let error = apply_domain_patch_for_test(
            default_recording_settings(),
            RecordingSettingsDomainPatch::CaptureSources(
                UpdateCaptureSourceSettingsRequest::default(),
            ),
        )
        .expect_err("empty patch should be rejected");

        assert_eq!(error.code, "empty_settings_patch");
    }

    #[test]
    fn capture_sources_domain_rejects_system_audio_without_screen_when_capability_requires_screen()
    {
        let error = apply_domain_patch_for_test(
            default_recording_settings(),
            RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
                capture_screen: Some(false),
                capture_microphone: Some(true),
                capture_system_audio: Some(true),
            }),
        )
        .expect_err("system audio without screen should be rejected");

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(
            error.message,
            "System audio capture requires screen capture"
        );
    }

    #[test]
    fn capture_sources_domain_allows_system_audio_without_screen_when_capability_allows_independent_audio(
    ) {
        let updated = apply_domain_patch_for_test_with_capture_support(
            default_recording_settings(),
            RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
                capture_screen: Some(false),
                capture_microphone: Some(true),
                capture_system_audio: Some(true),
            }),
            false,
        )
        .expect("independent system audio should validate without screen");

        assert!(!updated.capture_screen);
        assert!(updated.capture_system_audio);
    }

    #[test]
    fn inactivity_domain_rejects_detector_off_when_system_audio_transcription_enabled() {
        let mut base = default_recording_settings();
        base.transcription.enabled = true;
        base.transcription.system_audio_enabled = true;

        let error = apply_domain_patch_for_test(
            base,
            RecordingSettingsDomainPatch::Inactivity(UpdateInactivitySettingsRequest {
                audio_speech_detection: Some(capture_types::AudioSpeechDetectionSettings {
                    detector: capture_types::AudioSpeechDetector::Off,
                }),
                ..UpdateInactivitySettingsRequest::default()
            }),
        )
        .expect_err("detector off should be rejected for system audio transcription");

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(
            error.message,
            "audioSpeechDetection.detector cannot be off while transcription.systemAudioEnabled is true"
        );
    }

    #[test]
    fn processing_domain_rejects_system_audio_transcription_when_detector_is_off() {
        let mut base = default_recording_settings();
        base.audio_speech_detection.detector = capture_types::AudioSpeechDetector::Off;
        base.microphone_vad_adapter = capture_types::AudioSpeechDetector::Off;
        base.transcription.system_audio_enabled = false;

        let mut transcription = base.transcription.clone();
        transcription.system_audio_enabled = true;

        let error = apply_domain_patch_for_test(
            base,
            RecordingSettingsDomainPatch::Processing(UpdateProcessingSettingsRequest {
                transcription: Some(transcription),
                ..UpdateProcessingSettingsRequest::default()
            }),
        )
        .expect_err("system audio transcription should require a speech detector");

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(
            error.message,
            "audioSpeechDetection.detector cannot be off while transcription.systemAudioEnabled is true"
        );
    }

    #[test]
    fn storage_domain_preserves_non_storage_settings() {
        let mut base = default_recording_settings();
        base.capture_microphone = true;
        base.developer_options_enabled = true;
        base.ocr.enabled = false;

        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::Storage(UpdateStorageSettingsRequest {
                save_directory: Some("/tmp/mnema-after".to_string()),
                retention_policy: Some(RetentionPolicy::Days7),
            }),
        )
        .expect("storage patch should validate");

        assert_eq!(updated.save_directory, "/tmp/mnema-after");
        assert_eq!(updated.retention_policy, RetentionPolicy::Days7);
        assert_eq!(updated.capture_microphone, base.capture_microphone);
        assert_eq!(
            updated.developer_options_enabled,
            base.developer_options_enabled
        );
        assert_eq!(updated.ocr, normalized_ocr_settings(base.ocr.clone()));
    }

    #[test]
    fn developer_domain_preserves_processing_and_capture_fields() {
        let mut base = default_recording_settings();
        base.capture_microphone = true;
        base.ocr.enabled = false;
        base.preview_cache_ttl_seconds = 120;

        let updated = apply_domain_patch_for_test(
            base.clone(),
            RecordingSettingsDomainPatch::Developer(UpdateDeveloperSettingsRequest {
                developer_options_enabled: Some(true),
                native_capture_debug_logging_enabled: Some(true),
            }),
        )
        .expect("developer patch should validate");

        assert!(updated.developer_options_enabled);
        assert!(updated.native_capture_debug_logging_enabled);
        assert_eq!(updated.capture_microphone, base.capture_microphone);
        assert_eq!(updated.ocr, normalized_ocr_settings(base.ocr.clone()));
        assert_eq!(
            updated.preview_cache_ttl_seconds,
            base.preview_cache_ttl_seconds
        );
    }

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

    #[test]
    fn validate_recording_settings_normalizes_microphone_vad_adapter_from_shared_detector() {
        let settings = validate_recording_settings_with_capture_support(
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
                native_capture_debug_logging_enabled: false,
                developer_options_enabled: false,
                preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
                follow_timeline_live: false,
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
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

        let settings = validate_recording_settings_with_capture_support(
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
                native_capture_debug_logging_enabled: false,
                developer_options_enabled: false,
                preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
                follow_timeline_live: false,
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
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

        let settings = validate_recording_settings_with_capture_support(
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
                native_capture_debug_logging_enabled: false,
                developer_options_enabled: false,
                preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
                follow_timeline_live: false,
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
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
            true,
        )
        .expect("legacy PaddleOCR settings should normalize");

        assert_eq!(settings.ocr, default_ocr_settings());
    }

    // The platform default provider must always be runtime-runnable, otherwise the
    // coercion guard would hand back an unusable provider. (On this Windows host
    // that default is Tesseract; on macOS it is Apple Vision.)
    #[test]
    fn default_ocr_settings_provider_is_runtime_available() {
        assert!(
            ocr::provider_runtime_available(ocr_provider_runtime_id(
                default_ocr_settings().provider
            )),
            "default OCR provider must be runnable on this OS"
        );
    }

    // A provider that can actually run on this OS passes the guard untouched and
    // keeps its provider through the provider-specific match arms.
    #[test]
    fn validate_ocr_settings_passes_through_runnable_provider() {
        let value = default_ocr_settings();
        let provider = value.provider;
        assert!(ocr::provider_runtime_available(ocr_provider_runtime_id(
            provider
        )));

        let normalized =
            validate_ocr_settings(value).expect("runnable default provider should validate");

        assert_eq!(normalized.provider, provider);
    }

    // Apple Vision can never run off macOS, so a persisted Apple-Vision selection
    // is silently coerced to the runnable platform default (Tesseract here),
    // preserving `enabled`. This assertion executes on the Windows host.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn validate_ocr_settings_coerces_apple_vision_when_not_runnable() {
        let mut value = default_ocr_settings();
        value.provider = OcrProvider::AppleVision;
        value.enabled = false;

        let normalized =
            validate_ocr_settings(value).expect("apple vision should coerce off macOS");

        assert_ne!(normalized.provider, OcrProvider::AppleVision);
        assert!(
            ocr::provider_runtime_available(ocr_provider_runtime_id(normalized.provider)),
            "coerced provider must be runtime-available, got {:?}",
            normalized.provider
        );
        let mut expected = default_ocr_settings();
        expected.enabled = false;
        assert_eq!(normalized, expected);
    }

    // On the Windows build PaddleOCR is not compiled (`paddle-rs` is off), so a
    // legacy persisted PaddleOCR selection hits the runtime guard and is coerced
    // to the runnable default.
    #[cfg(windows)]
    #[test]
    fn validate_ocr_settings_coerces_legacy_paddle_ocr_when_not_runnable() {
        assert!(
            !ocr::provider_runtime_available(ocr::PADDLE_OCR_PROVIDER_ID),
            "precondition: PaddleOCR is not runnable on the Windows build"
        );

        let mut value = default_ocr_settings();
        value.provider = OcrProvider::PaddleOcr;
        value.model_id = Some(ocr::DEFAULT_PADDLE_OCR_MODEL_ID.to_string());
        value.language = Some(ocr::DEFAULT_PADDLE_OCR_LANGUAGE.to_string());

        let normalized = validate_ocr_settings(value).expect("legacy PaddleOCR should coerce");

        assert_eq!(normalized, default_ocr_settings());
        assert!(ocr::provider_runtime_available(ocr_provider_runtime_id(
            normalized.provider
        )));
    }

    #[test]
    fn validate_recording_settings_preserves_transcription_update() {
        let mut transcription = default_audio_transcription_settings();
        transcription.provider = capture_types::AudioTranscriptionProvider::Parakeet;
        transcription.model_id = Some("parakeet-tdt-0.6b-v3-onnx".to_string());
        transcription.language = " en ".to_string();

        let settings = validate_recording_settings_with_capture_support(
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
                native_capture_debug_logging_enabled: false,
                developer_options_enabled: false,
                preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
                follow_timeline_live: false,
                retention_policy: RetentionPolicy::Never,
                appearance: default_appearance(),
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
        assert_eq!(loaded.ocr, normalized_ocr_settings(default_ocr_settings()));
        assert_eq!(loaded.access, AccessSettings::default());
        assert_eq!(loaded.transcription, default_audio_transcription_settings());
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
            native_capture_debug_logging_enabled: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: MAX_PREVIEW_CACHE_TTL_SECONDS + 1,
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
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

        assert_eq!(validated.provider, semantic_search::SEMANTIC_SEARCH_PROVIDER_ID);
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
}
