use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioActivitySample {
    pub last_unix_ms: Option<u64>,
    pub level: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioActivityDecision {
    pub enabled: bool,
    pub idle_ms: Option<u64>,
    pub activity_threshold: Option<f32>,
    pub detector: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneVadStatus {
    pub configured_adapter: String,
    pub effective_adapter: String,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSourcesStatus {
    pub screen: RuntimeSourceStatus,
    pub microphone: RuntimeSourceStatus,
    pub system_audio: RuntimeSourceStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSourceStatus {
    pub requested: bool,
    pub paused: bool,
    pub session_active: Option<bool>,
    pub writer_active: Option<bool>,
    pub output_path: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDebugActivitySource {
    pub kind: String,
    pub enabled: bool,
    pub available: bool,
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
    pub activity_threshold: Option<f32>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDebugInfo {
    pub system_idle_ms: Option<u64>,
    pub system_idle_available: bool,
    pub inactivity_enabled: bool,
    pub idle_timeout_seconds: u64,
    pub is_inactivity_paused: bool,
    pub detector_source: String,
    pub activity_mode: String,
    pub microphone_activity_sensitivity: u8,
    pub system_audio_activity_sensitivity: u8,
    pub screen_activity_last_unix_ms: Option<u64>,
    pub screen_activity_idle_ms: Option<u64>,
    pub microphone_activity_sample: AudioActivitySample,
    pub microphone_activity_decision: AudioActivityDecision,
    pub system_audio_activity_sample: AudioActivitySample,
    pub system_audio_activity_decision: AudioActivityDecision,
    pub microphone_vad: MicrophoneVadStatus,
    pub effective_idle_ms: u64,
    #[serde(rename = "effectiveActivitySource")]
    pub effective_idle_source: String,
    pub screen_effective_idle_ms: u64,
    #[serde(rename = "screenEffectiveActivitySource")]
    pub screen_effective_idle_source: String,
    pub screen_paused: bool,
    pub microphone_effective_idle_ms: u64,
    #[serde(rename = "microphoneEffectiveActivitySource")]
    pub microphone_effective_idle_source: String,
    pub microphone_paused: bool,
    pub system_audio_effective_idle_ms: u64,
    #[serde(rename = "systemAudioEffectiveActivitySource")]
    pub system_audio_effective_idle_source: String,
    pub system_audio_paused: bool,
    pub activity_sources: Vec<IdleDebugActivitySource>,
    pub runtime_sources: RuntimeSourcesStatus,
}
