//! Recording settings: the persisted `recording-settings.json`, its defaults,
//! validation and per-domain patches. The capture-coupled save guards (sources
//! locked while recording) stay with capture in the desktop crate until S3.

mod domain_patch;
mod validate;

pub use domain_patch::*;
pub use validate::*;

use capture_types::{
    default_appearance, default_audio_speech_detection_settings,
    default_audio_transcription_settings, default_developer_options_enabled,
    default_follow_timeline_live, default_idle_timeout_seconds, default_metadata_settings,
    default_microphone_activity_sensitivity, default_ocr_settings,
    default_pause_capture_on_inactivity, default_preview_cache_ttl_seconds,
    default_privacy_settings, default_speaker_analysis_settings,
    default_system_audio_activity_sensitivity, default_video_bitrate, AccessSettings,
    AiRuntimeSettings, CaptureErrorResponse, RecordingSettings, RetentionPolicy, ScreenResolution,
    ScreenResolutionPreset, UpdateRecordingSettingsRequest, UserContextSettings,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::AppCore;

const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const MIN_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 1;
const MAX_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 40;
const MIN_IDLE_TIMEOUT_SECONDS: u64 = 1;
const MAX_IDLE_TIMEOUT_SECONDS: u64 = 3600;
const MIN_AUDIO_ACTIVITY_SENSITIVITY: u8 = 0;
const MAX_AUDIO_ACTIVITY_SENSITIVITY: u8 = 100;
const MAX_PREVIEW_CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;
const MAX_SEGMENT_DURATION_SECONDS: u64 = 300;
// One snapshot per minute. Matches the frontend capture-rate ladder floor
// (lib/components/capture-rate.ts); JS `1/60` and Rust `1.0/60.0` produce the
// same f64, so the boundary value round-trips the wire exactly.
const MIN_SCREEN_FRAME_RATE: f64 = 1.0 / 60.0;
const MAX_SCREEN_FRAME_RATE: f64 = 10.0;
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

/// Shared (`Arc`) so the Tauri builder and the `AppCore` state map hold the same
/// settings, like [`crate::AppInfraState`].
pub type RecordingSettingsState = Arc<Mutex<RecordingSettingsRuntime>>;

pub struct LoadedRecordingSettings {
    pub settings: RecordingSettings,
    pub source: &'static str,
}

pub struct AppliedRecordingSettingsUpdate {
    pub settings: RecordingSettings,
    pub previous_settings: RecordingSettings,
    pub previous_save_directory: String,
    pub save_directory_changed: bool,
}

pub fn default_save_directory() -> String {
    // Honor MNEMA_SAVE_DIRECTORY verbatim when set, matching the broker/CLI
    // read-only resolver in `crates/app-infra/src/brokered_access.rs`
    // (`default_save_directory_from_config`): any set value is taken as the
    // directory path with no trimming, no `~` expansion, and no emptiness
    // filtering. This keeps the dev sandbox (`dev:sandbox` sets the env var to
    // `$HOME/.mnema-dev`) isolated from the production `~/.mnema` root.
    if let Ok(path) = std::env::var("MNEMA_SAVE_DIRECTORY") {
        return PathBuf::from(path).to_string_lossy().to_string();
    }

    std::env::var("HOME")
        .map(|home| Path::new(&home).join(".mnema"))
        .unwrap_or_else(|_| PathBuf::from(".mnema"))
        .to_string_lossy()
        .to_string()
}

pub fn default_recording_settings() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 0.5,
        // 720p, not Original: the disk estimates are anchored on a 720p
        // measurement, and native Retina frames cost several times as much for
        // recall screenshots that OCR reads equally well. Original remains a
        // choice — and the only valid value where SCK scaling is unavailable
        // (macOS < 15), where validation rejects everything else.
        screen_resolution: ScreenResolution::Preset {
            preset: if supports_non_original_screen_resolution() {
                ScreenResolutionPreset::P720
            } else {
                ScreenResolutionPreset::Original
            },
        },
        video_bitrate: default_video_bitrate(),
        save_directory: default_save_directory(),
        // On: a fresh install that forgets to press Record loses whole days.
        auto_start: true,
        developer_options_enabled: default_developer_options_enabled(),
        preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
        follow_timeline_live: default_follow_timeline_live(),
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
        pause_capture_on_inactivity: default_pause_capture_on_inactivity(),
        idle_timeout_seconds: default_idle_timeout_seconds(),
        microphone_activity_sensitivity: default_microphone_activity_sensitivity(),
        system_audio_activity_sensitivity: default_system_audio_activity_sensitivity(),
        microphone_vad_adapter: capture_types::default_audio_speech_detector(),
        inactivity_activity_mode: capture_types::InactivityActivityMode::SystemInputOrScreenOrAudio,
    }
}

pub fn recording_settings_file_path(core: &AppCore) -> PathBuf {
    core.paths().config.join(RECORDING_SETTINGS_FILE_NAME)
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

    // ADR 0034: a legacy engine-centric `aiRuntime` block (engineKind/cloud*/
    // local* + additionalEngines) migrates into the provider list inside
    // `AiRuntimeSettings`' deserialization; the next save rewrites only the new
    // shape. Log once at load so the upgrade is visible.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
        if let Some(ai_runtime) = value.get("aiRuntime").and_then(|value| value.as_object()) {
            if !ai_runtime.contains_key("providers") && ai_runtime.contains_key("engineKind") {
                log::info!(
                    "migrating legacy engine-centric aiRuntime settings to the provider-centric shape (rewritten on next save)"
                );
            }
        }
    }

    let parsed = serde_json::from_str::<RecordingSettings>(&raw).ok()?;
    validate_recording_settings_with_resolution_support(
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
            developer_options_enabled: parsed.developer_options_enabled,
            preview_cache_ttl_seconds: parsed.preview_cache_ttl_seconds,
            follow_timeline_live: parsed.follow_timeline_live,
            retention_policy: parsed.retention_policy,
            appearance: parsed.appearance,
            time_format: parsed.time_format,
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

pub fn load_recording_settings_from_disk(core: &AppCore) -> Option<RecordingSettings> {
    load_recording_settings_from_path(&recording_settings_file_path(core))
}

pub fn load_recording_settings_or_default(core: &AppCore) -> RecordingSettings {
    load_recording_settings_from_disk(core).unwrap_or_else(default_recording_settings)
}

pub fn initialize_recording_settings_state_from_disk(
    core: &AppCore,
    state: &RecordingSettingsState,
) -> LoadedRecordingSettings {
    let loaded = match load_recording_settings_from_disk(core) {
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

pub fn current_recording_settings(state: &RecordingSettingsState) -> RecordingSettings {
    state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone()
}

pub fn current_auto_start(state: &RecordingSettingsState) -> bool {
    state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .auto_start
}

pub fn persist_recording_settings(
    core: &AppCore,
    settings: &RecordingSettings,
) -> Result<(), CaptureErrorResponse> {
    let file_path = recording_settings_file_path(core);
    persist_recording_settings_to_path(&file_path, settings)
}

pub fn persist_recording_settings_to_path(
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
mod tests;
