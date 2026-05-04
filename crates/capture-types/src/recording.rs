use crate::InactivityActivityMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScreenResolutionPreset {
    Original,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "540p")]
    P540,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ScreenResolution {
    Preset { preset: ScreenResolutionPreset },
    Custom { width: u32, height: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoBitratePreset {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoBitrateMode {
    Preset,
    Custom,
}

pub fn default_video_bitrate_mode() -> VideoBitrateMode {
    VideoBitrateMode::Preset
}

pub fn default_video_bitrate_preset() -> Option<VideoBitratePreset> {
    Some(VideoBitratePreset::Medium)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VideoBitrateSettings {
    #[serde(default = "default_video_bitrate_mode")]
    pub mode: VideoBitrateMode,
    #[serde(default = "default_video_bitrate_preset")]
    pub preset: Option<VideoBitratePreset>,
    #[serde(default)]
    pub custom_mbps: Option<u32>,
}

pub fn default_screen_resolution() -> ScreenResolution {
    ScreenResolution::Preset {
        preset: ScreenResolutionPreset::Original,
    }
}

pub fn default_video_bitrate() -> VideoBitrateSettings {
    VideoBitrateSettings {
        mode: VideoBitrateMode::Preset,
        preset: Some(VideoBitratePreset::Medium),
        custom_mbps: None,
    }
}

pub fn default_pause_capture_on_inactivity() -> bool {
    true
}

pub fn default_idle_timeout_seconds() -> u64 {
    10
}

pub fn default_microphone_activity_sensitivity() -> u8 {
    50
}

pub fn default_system_audio_activity_sensitivity() -> u8 {
    50
}

pub fn default_native_capture_debug_logging_enabled() -> bool {
    false
}

pub fn default_developer_options_enabled() -> bool {
    false
}

pub fn default_preview_cache_ttl_seconds() -> u64 {
    3600
}

pub fn default_follow_timeline_live() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrRecognitionMode {
    Fast,
    Accurate,
}

pub fn default_ocr_recognition_mode() -> OcrRecognitionMode {
    OcrRecognitionMode::Fast
}

pub fn default_ocr_language_correction() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrSettings {
    #[serde(default = "default_ocr_recognition_mode")]
    pub recognition_mode: OcrRecognitionMode,
    #[serde(default = "default_ocr_language_correction")]
    pub language_correction: bool,
}

pub fn default_ocr_settings() -> OcrSettings {
    OcrSettings {
        recognition_mode: default_ocr_recognition_mode(),
        language_correction: default_ocr_language_correction(),
    }
}

impl Default for VideoBitrateSettings {
    fn default() -> Self {
        default_video_bitrate()
    }
}

impl Default for ScreenResolution {
    fn default() -> Self {
        default_screen_resolution()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSettings {
    pub capture_screen: bool,
    pub capture_microphone: bool,
    pub capture_system_audio: bool,
    pub segment_duration_seconds: u64,
    pub screen_frame_rate: u32,
    #[serde(default = "default_screen_resolution")]
    pub screen_resolution: ScreenResolution,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate: VideoBitrateSettings,
    pub save_directory: String,
    pub auto_start: bool,
    #[serde(default = "default_native_capture_debug_logging_enabled")]
    pub native_capture_debug_logging_enabled: bool,
    #[serde(default = "default_developer_options_enabled")]
    pub developer_options_enabled: bool,
    #[serde(default = "default_preview_cache_ttl_seconds")]
    pub preview_cache_ttl_seconds: u64,
    #[serde(default = "default_follow_timeline_live")]
    pub follow_timeline_live: bool,
    #[serde(default = "default_ocr_settings")]
    pub ocr: OcrSettings,
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_microphone_activity_sensitivity")]
    pub microphone_activity_sensitivity: u8,
    #[serde(default = "default_system_audio_activity_sensitivity")]
    pub system_audio_activity_sensitivity: u8,
    #[serde(
        default = "crate::default_inactivity_activity_mode",
        rename = "activityMode",
        alias = "inactivityActivityMode"
    )]
    pub inactivity_activity_mode: InactivityActivityMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecordingSettingsRequest {
    pub capture_screen: bool,
    pub capture_microphone: bool,
    pub capture_system_audio: bool,
    pub segment_duration_seconds: u64,
    pub screen_frame_rate: u32,
    #[serde(default = "default_screen_resolution")]
    pub screen_resolution: ScreenResolution,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate: VideoBitrateSettings,
    pub save_directory: String,
    pub auto_start: bool,
    #[serde(default = "default_native_capture_debug_logging_enabled")]
    pub native_capture_debug_logging_enabled: bool,
    #[serde(default = "default_developer_options_enabled")]
    pub developer_options_enabled: bool,
    #[serde(default = "default_preview_cache_ttl_seconds")]
    pub preview_cache_ttl_seconds: u64,
    #[serde(default = "default_follow_timeline_live")]
    pub follow_timeline_live: bool,
    #[serde(default = "default_ocr_settings")]
    pub ocr: OcrSettings,
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_microphone_activity_sensitivity")]
    pub microphone_activity_sensitivity: u8,
    #[serde(default = "default_system_audio_activity_sensitivity")]
    pub system_audio_activity_sensitivity: u8,
    #[serde(
        default = "crate::default_inactivity_activity_mode",
        rename = "activityMode",
        alias = "inactivityActivityMode"
    )]
    pub inactivity_activity_mode: InactivityActivityMode,
}
