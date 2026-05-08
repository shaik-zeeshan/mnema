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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneVadAdapter {
    Silero,
    Webrtc,
    Off,
}

pub fn default_microphone_vad_adapter() -> MicrophoneVadAdapter {
    MicrophoneVadAdapter::Silero
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
pub enum AppearanceSetting {
    System,
    Light,
    Dark,
}

pub fn default_appearance() -> AppearanceSetting {
    AppearanceSetting::System
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrProvider {
    AppleVision,
    Tesseract,
    PaddleOcr,
}

pub fn default_ocr_provider() -> OcrProvider {
    OcrProvider::AppleVision
}

pub fn default_ocr_enabled() -> bool {
    true
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

pub fn default_ocr_model_id() -> Option<String> {
    None
}

pub fn default_ocr_language() -> Option<String> {
    None
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrTesseractPageSegmentationMode {
    Auto,
    SingleBlock,
    SingleLine,
    SingleWord,
    SparseText,
}

pub fn default_ocr_tesseract_page_segmentation_mode() -> OcrTesseractPageSegmentationMode {
    OcrTesseractPageSegmentationMode::SingleBlock
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrTesseractPreprocessMode {
    Grayscale,
    Thresholded,
}

pub fn default_ocr_tesseract_preprocess_mode() -> OcrTesseractPreprocessMode {
    OcrTesseractPreprocessMode::Grayscale
}

pub fn default_ocr_tesseract_upscale_factor() -> u8 {
    1
}

pub fn default_ocr_tesseract_char_whitelist() -> Option<String> {
    None
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrSettings {
    #[serde(default = "default_ocr_enabled")]
    pub enabled: bool,
    #[serde(default = "default_ocr_provider")]
    pub provider: OcrProvider,
    #[serde(default = "default_ocr_model_id")]
    pub model_id: Option<String>,
    #[serde(default = "default_ocr_language")]
    pub language: Option<String>,
    #[serde(default = "default_ocr_recognition_mode")]
    pub recognition_mode: OcrRecognitionMode,
    #[serde(default = "default_ocr_language_correction")]
    pub language_correction: bool,
    #[serde(default = "default_ocr_tesseract_page_segmentation_mode")]
    pub tesseract_page_segmentation_mode: OcrTesseractPageSegmentationMode,
    #[serde(default = "default_ocr_tesseract_preprocess_mode")]
    pub tesseract_preprocess_mode: OcrTesseractPreprocessMode,
    #[serde(default = "default_ocr_tesseract_upscale_factor")]
    pub tesseract_upscale_factor: u8,
    #[serde(default = "default_ocr_tesseract_char_whitelist")]
    pub tesseract_char_whitelist: Option<String>,
}

pub fn default_ocr_settings() -> OcrSettings {
    OcrSettings {
        enabled: default_ocr_enabled(),
        provider: default_ocr_provider(),
        model_id: default_ocr_model_id(),
        language: default_ocr_language(),
        recognition_mode: default_ocr_recognition_mode(),
        language_correction: default_ocr_language_correction(),
        tesseract_page_segmentation_mode: default_ocr_tesseract_page_segmentation_mode(),
        tesseract_preprocess_mode: default_ocr_tesseract_preprocess_mode(),
        tesseract_upscale_factor: default_ocr_tesseract_upscale_factor(),
        tesseract_char_whitelist: default_ocr_tesseract_char_whitelist(),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioTranscriptionProvider {
    LocalWhisper,
    AppleSpeechOnDevice,
    Parakeet,
}

pub fn default_audio_transcription_enabled() -> bool {
    true
}

pub fn default_audio_transcription_provider() -> AudioTranscriptionProvider {
    AudioTranscriptionProvider::LocalWhisper
}

pub fn default_audio_transcription_model_id() -> Option<String> {
    Some("base".to_string())
}

pub fn default_audio_transcription_language() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioTranscriptionMemoryMode {
    Balanced,
    LowMemory,
    Performance,
}

pub fn default_audio_transcription_memory_mode() -> AudioTranscriptionMemoryMode {
    AudioTranscriptionMemoryMode::Balanced
}

pub fn default_audio_transcription_idle_unload_seconds() -> u64 {
    300
}

pub fn default_audio_transcription_chunk_seconds() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionSettings {
    #[serde(default = "default_audio_transcription_enabled")]
    pub enabled: bool,
    #[serde(default = "default_audio_transcription_provider")]
    pub provider: AudioTranscriptionProvider,
    #[serde(default = "default_audio_transcription_model_id")]
    pub model_id: Option<String>,
    #[serde(default = "default_audio_transcription_language")]
    pub language: String,
    #[serde(default = "default_audio_transcription_memory_mode")]
    pub memory_mode: AudioTranscriptionMemoryMode,
    #[serde(default = "default_audio_transcription_idle_unload_seconds")]
    pub idle_unload_seconds: u64,
    #[serde(default = "default_audio_transcription_chunk_seconds")]
    pub chunk_seconds: u64,
}

pub fn default_audio_transcription_settings() -> AudioTranscriptionSettings {
    AudioTranscriptionSettings {
        enabled: default_audio_transcription_enabled(),
        provider: default_audio_transcription_provider(),
        model_id: default_audio_transcription_model_id(),
        language: default_audio_transcription_language(),
        memory_mode: default_audio_transcription_memory_mode(),
        idle_unload_seconds: default_audio_transcription_idle_unload_seconds(),
        chunk_seconds: default_audio_transcription_chunk_seconds(),
    }
}

impl Default for AudioTranscriptionSettings {
    fn default() -> Self {
        default_audio_transcription_settings()
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
    #[serde(default = "default_appearance")]
    pub appearance: AppearanceSetting,
    #[serde(default = "default_ocr_settings")]
    pub ocr: OcrSettings,
    #[serde(default = "default_audio_transcription_settings")]
    pub transcription: AudioTranscriptionSettings,
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_microphone_activity_sensitivity")]
    pub microphone_activity_sensitivity: u8,
    #[serde(default = "default_system_audio_activity_sensitivity")]
    pub system_audio_activity_sensitivity: u8,
    #[serde(default = "default_microphone_vad_adapter")]
    pub microphone_vad_adapter: MicrophoneVadAdapter,
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
    #[serde(default = "default_appearance")]
    pub appearance: AppearanceSetting,
    #[serde(default = "default_ocr_settings")]
    pub ocr: OcrSettings,
    #[serde(default = "default_audio_transcription_settings")]
    pub transcription: AudioTranscriptionSettings,
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_microphone_activity_sensitivity")]
    pub microphone_activity_sensitivity: u8,
    #[serde(default = "default_system_audio_activity_sensitivity")]
    pub system_audio_activity_sensitivity: u8,
    #[serde(default = "default_microphone_vad_adapter")]
    pub microphone_vad_adapter: MicrophoneVadAdapter,
    #[serde(
        default = "crate::default_inactivity_activity_mode",
        rename = "activityMode",
        alias = "inactivityActivityMode"
    )]
    pub inactivity_activity_mode: InactivityActivityMode,
}
