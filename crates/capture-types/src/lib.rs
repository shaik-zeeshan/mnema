use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePermissionState {
    Granted,
    Denied,
    NotDetermined,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSources {
    pub screen: bool,
    pub microphone: bool,
    pub system_audio: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSupportResponse {
    pub platform: String,
    pub native_capture_supported: bool,
    pub supported_sources: CaptureSources,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePermissions {
    pub screen: CapturePermissionState,
    pub microphone: CapturePermissionState,
    pub system_audio: CapturePermissionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutputFiles {
    pub screen_file: Option<String>,
    pub screen_files: Vec<String>,
    pub microphone_file: Option<String>,
    pub microphone_files: Vec<String>,
    pub system_audio_file: Option<String>,
    pub system_audio_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSession {
    pub is_running: bool,
    pub is_inactivity_paused: bool,
    pub session_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub requested_sources: Option<CaptureSources>,
    pub output_files: Option<CaptureOutputFiles>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePermissionsResponse {
    pub permissions: CapturePermissions,
    pub session: NativeCaptureSession,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartNativeCaptureRequest {
    pub capture_screen: bool,
    pub capture_microphone: bool,
    pub capture_system_audio: bool,
}

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

pub fn default_audio_activity_sensitivity() -> u8 {
    50
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InactivityActivityMode {
    SystemInputOnly,
    SystemInputOrScreen,
    SystemInputOrScreenOrAudio,
}

pub fn default_inactivity_activity_mode() -> InactivityActivityMode {
    InactivityActivityMode::SystemInputOrScreen
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
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_audio_activity_sensitivity")]
    pub audio_activity_sensitivity: u8,
    #[serde(
        default = "default_inactivity_activity_mode",
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
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_audio_activity_sensitivity")]
    pub audio_activity_sensitivity: u8,
    #[serde(
        default = "default_inactivity_activity_mode",
        rename = "activityMode",
        alias = "inactivityActivityMode"
    )]
    pub inactivity_activity_mode: InactivityActivityMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSessionResponse {
    pub session: NativeCaptureSession,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MicrophonePreferenceMode {
    Default,
    SpecificDevice,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MicrophonePreference {
    pub mode: MicrophonePreferenceMode,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneDisconnectPolicy {
    FallbackToDefault,
    WaitForSameDevice,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneControllerState {
    pub devices: Vec<MicrophoneDevice>,
    pub preference: MicrophonePreference,
    pub disconnect_policy: MicrophoneDisconnectPolicy,
    pub effective_device: Option<MicrophoneDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMicrophoneControllerRequest {
    pub preference: MicrophonePreference,
    pub disconnect_policy: MicrophoneDisconnectPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            settings.audio_activity_sensitivity,
            default_audio_activity_sensitivity()
        );
        assert_eq!(
            settings.inactivity_activity_mode,
            InactivityActivityMode::SystemInputOrScreenOrAudio
        );
    }
}
