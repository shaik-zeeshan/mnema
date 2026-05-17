use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSessionMeta {
    pub session_id: String,
    pub started_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSessions {
    pub screen: Option<SourceSessionMeta>,
    pub microphone: Option<SourceSessionMeta>,
    pub system_audio: Option<SourceSessionMeta>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSession {
    pub is_running: bool,
    pub is_inactivity_paused: bool,
    pub requested_sources: Option<CaptureSources>,
    pub output_files: Option<CaptureOutputFiles>,
    pub source_sessions: Option<SourceSessions>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSessionResponse {
    pub session: NativeCaptureSession,
}
