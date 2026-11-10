use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// UID prefix of the private aggregate device the system-audio tap builds
/// (`capture-system-audio`); the reaper keys off it too.
pub const SYSTEM_AUDIO_AGGREGATE_UID_PREFIX: &str = "mnema-system-audio-";
/// That aggregate's display name.
pub const SYSTEM_AUDIO_AGGREGATE_NAME: &str = "Mnema System Audio";

impl MicrophoneDevice {
    /// Mnema's own system-audio aggregate. It is private, so only our process
    /// lists it, and recording it as a microphone would record our own output.
    pub fn is_mnema_system_audio(id: &str, name: &str) -> bool {
        id.starts_with(SYSTEM_AUDIO_AGGREGATE_UID_PREFIX) || name == SYSTEM_AUDIO_AGGREGATE_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_the_system_audio_aggregate() {
        assert!(MicrophoneDevice::is_mnema_system_audio(
            "mnema-system-audio-4242-6C6A2B0E-1111-2222-3333-444455556666",
            "Mnema System Audio"
        ));
        assert!(MicrophoneDevice::is_mnema_system_audio(
            "mnema-system-audio-1-x",
            "Renamed"
        ));
        assert!(!MicrophoneDevice::is_mnema_system_audio(
            "BuiltInMicrophoneDevice",
            "MacBook Air Microphone"
        ));
        assert!(!MicrophoneDevice::is_mnema_system_audio(
            "usb-1",
            "Mnema System Audio Mic"
        ));
    }
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
