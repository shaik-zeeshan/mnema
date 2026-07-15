use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapturePermissionState {
    Granted,
    Denied,
    NotDetermined,
    Unsupported,
    Unknown,
    /// System audio only (ADR 0052). Core Audio process taps have their own TCC
    /// category and **no authorization query at all**, so the two states below
    /// are inferred from what the tap delivered, never read from the OS.
    ///
    /// A tap has delivered sound, which only a granted tap can do.
    AssumedWorking,
    /// A tap has run and never delivered a sound. Denial looks exactly like a
    /// quiet Mac, so this is a suspicion, not a verdict — the surfaces that
    /// render it say "may be blocked" and let the user dismiss it.
    PossiblyBlocked,
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
    pub is_user_paused: bool,
    pub is_low_disk_suspended: bool,
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

// There is no codegen between these types and their hand-written TypeScript
// mirror, so the wire strings are pinned here: `PermissionStatus` in
// `apps/desktop/src/lib/types/session.ts` has to spell them the same way.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_states_round_trip_their_wire_strings() {
        for (state, wire) in [
            (CapturePermissionState::Granted, "granted"),
            (CapturePermissionState::Denied, "denied"),
            (CapturePermissionState::NotDetermined, "not_determined"),
            (CapturePermissionState::Unsupported, "unsupported"),
            (CapturePermissionState::Unknown, "unknown"),
            (CapturePermissionState::AssumedWorking, "assumed_working"),
            (CapturePermissionState::PossiblyBlocked, "possibly_blocked"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<CapturePermissionState>(wire.into()).unwrap(),
                state
            );
        }
    }

    #[test]
    fn permissions_serialize_the_inferred_system_audio_states() {
        let permissions = CapturePermissions {
            screen: CapturePermissionState::Granted,
            microphone: CapturePermissionState::Denied,
            system_audio: CapturePermissionState::PossiblyBlocked,
        };

        assert_eq!(
            serde_json::to_value(&permissions).unwrap(),
            serde_json::json!({
                "screen": "granted",
                "microphone": "denied",
                "systemAudio": "possibly_blocked",
            })
        );
    }
}
