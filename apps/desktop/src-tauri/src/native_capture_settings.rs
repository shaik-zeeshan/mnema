use capture_types::{
    default_audio_activity_sensitivity, default_idle_timeout_seconds,
    default_inactivity_activity_mode, default_native_capture_debug_logging_enabled,
    default_pause_capture_on_inactivity, default_video_bitrate, CaptureErrorResponse,
    RecordingSettings, ScreenResolution, ScreenResolutionPreset, UpdateRecordingSettingsRequest,
    VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings,
};
use std::path::{Path, PathBuf};
use tauri::Manager;

const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const MIN_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 1;
const MAX_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 40;
const MIN_IDLE_TIMEOUT_SECONDS: u64 = 1;
const MAX_IDLE_TIMEOUT_SECONDS: u64 = 3600;
const MIN_AUDIO_ACTIVITY_SENSITIVITY: u8 = 0;
const MAX_AUDIO_ACTIVITY_SENSITIVITY: u8 = 100;
const MIN_EFFECTIVE_VIDEO_BITRATE_BPS: u32 = 500_000;
const MAX_EFFECTIVE_VIDEO_BITRATE_BPS: u32 = 120_000_000;
const VIDEO_BITRATE_ROUND_STEP_BPS: u32 = 250_000;

pub(crate) fn default_save_directory() -> String {
    std::env::var("HOME")
        .map(|home| Path::new(&home).join(".z_records"))
        .unwrap_or_else(|_| PathBuf::from(".z_records"))
        .to_string_lossy()
        .to_string()
}

pub(crate) fn default_recording_settings() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: default_save_directory(),
        auto_start: false,
        native_capture_debug_logging_enabled: default_native_capture_debug_logging_enabled(),
        pause_capture_on_inactivity: default_pause_capture_on_inactivity(),
        idle_timeout_seconds: default_idle_timeout_seconds(),
        audio_activity_sensitivity: default_audio_activity_sensitivity(),
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

fn validate_audio_activity_sensitivity(value: u8) -> Result<u8, CaptureErrorResponse> {
    if !(MIN_AUDIO_ACTIVITY_SENSITIVITY..=MAX_AUDIO_ACTIVITY_SENSITIVITY).contains(&value) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: format!(
                "audioActivitySensitivity must be between {MIN_AUDIO_ACTIVITY_SENSITIVITY} and {MAX_AUDIO_ACTIVITY_SENSITIVITY}"
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

    let screen_resolution = validate_screen_resolution(request.screen_resolution)?;
    let video_bitrate = validate_video_bitrate(request.video_bitrate)?;
    let audio_activity_sensitivity =
        validate_audio_activity_sensitivity(request.audio_activity_sensitivity)?;

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
        pause_capture_on_inactivity: request.pause_capture_on_inactivity,
        idle_timeout_seconds: request.idle_timeout_seconds,
        audio_activity_sensitivity,
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
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<RecordingSettings>(&raw).ok()?;
    validate_recording_settings(UpdateRecordingSettingsRequest {
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
        pause_capture_on_inactivity: parsed.pause_capture_on_inactivity,
        idle_timeout_seconds: parsed.idle_timeout_seconds,
        audio_activity_sensitivity: parsed.audio_activity_sensitivity,
        inactivity_activity_mode: parsed.inactivity_activity_mode,
    })
    .ok()
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
}
