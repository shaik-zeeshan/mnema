//! Parity fixtures -> Settings. `apps/desktop/parity/fixtures/settings/default.json`
//! is what the Svelte stub serves for every Settings state (command -> JSON);
//! the goldens read the same map, so both sides show one install. `FixtureBackend`
//! records saves for the timing tests and answers them like the backend would.

use std::sync::Mutex;

use capture_types::{MicrophoneControllerState, RecordingSettings};
use serde_json::Value;

use super::autosave::Unit;
use super::data::Backend;
use super::model::SettingsData;
use crate::shell::fixture::{get, load};

/// Parity states (and golden names): each group's top, then the deeper
/// sections of the tall groups, as `(state, ?tab= value)`.
pub const STATES: [(&str, &str); 12] = [
    ("general", "appearance"),
    ("general-shortcuts", "shortcuts"),
    ("capture", "capture"),
    ("capture-video", "video"),
    ("capture-privacy", "privacy"),
    ("intelligence", "intelligence"),
    ("intelligence-askai", "user-context"),
    ("intelligence-ocr", "ocr"),
    ("intelligence-speakers", "speakers"),
    ("data", "storage"),
    ("data-access", "access"),
    ("about", "license"),
];

pub fn settings_data() -> anyhow::Result<SettingsData> {
    let map = load("settings", "default")?;
    let v = |k: &str| map.get(k).cloned().filter(|v| !v.is_null());
    let mut d = SettingsData {
        settings: get(&map, &["get_recording_settings"])?,
        keyboard: v("get_keyboard_bindings_settings"),
        mic: get(&map, &["get_microphone_controller_state"])?,
        permissions: map
            .get("get_capture_permissions")
            .map(|p| p["permissions"].clone()),
        capture_support: v("get_capture_support"),
        license: get(&map, &["get_license_status"])?,
        storage_usage: v("get_storage_usage"),
        storage_location: get(&map, &["get_storage_location"])?,
        ai_status: v("get_ai_runtime_status"),
        ask_ai: v("ask_ai_availability"),
        user_context: v("get_user_context_status"),
        ocr_models: v("get_ocr_model_status"),
        transcription_models: v("get_audio_transcription_model_status"),
        speaker_models: v("get_speaker_analysis_model_status"),
        semantic_models: v("get_semantic_search_model_status"),
        cli_status: v("get_cli_access_status"),
        grants: map["list_cli_access_grants"]["grants"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        history: map["list_cli_access_history"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        update: v("get_app_update_status"),
        log_status: v("get_general_app_log_status"),
        notices: v("get_third_party_notices"),
        ..Default::default()
    };
    // The stub answers the per-id key / secret probes the same for every id.
    if let Some(s) = &d.settings {
        if map["ai_runtime_has_provider_key"] == Value::Bool(true) {
            let ending = map["ai_runtime_provider_key_ending"]
                .as_str()
                .map(str::to_string);
            for pv in s.ai_runtime.providers.iter().filter(|p| !p.kind.is_local()) {
                d.keys_saved.insert(pv.id.clone());
                if let Some(e) = &ending {
                    d.key_endings.insert(pv.id.clone(), e.clone());
                }
            }
        }
        if map["mcp_has_server_secret"] == Value::Bool(true) {
            d.mcp_secrets = s
                .ai_runtime
                .mcp_servers
                .iter()
                .map(|m| m.id.clone())
                .collect();
        }
    }
    Ok(d)
}

/// One recorded save.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Domain(Unit),
    Keyboard,
    Mic,
    Privacy,
    /// A provider key went to the vault (the key itself is never recorded).
    SaveKey(String),
    ClearKey(String),
    /// A one-time prompt record write.
    Prompt(super::privacy_prompt::Mark),
    /// Window focus re-read the permissions.
    Recheck,
    SelectSemantic(String),
    /// The fake recorder opened the microphone for a take of this many ms.
    RecordClip(u64),
    /// A running take was ended early (the microphone released).
    CancelClip,
    Enroll,
}

/// Saves succeed (echoing the draft) unless `fail` is set; every call is recorded.
/// The recorder is a fake: it never opens a microphone and answers at once.
#[derive(Default)]
pub struct FixtureBackend {
    pub(crate) calls: Mutex<Vec<Call>>,
    /// Each recording-domain save's settings, as sent.
    pub(crate) sent: Mutex<Vec<(Unit, RecordingSettings)>>,
    pub fail: bool,
    /// What `enroll` answers (`None` = enrolled).
    pub enrollment: Mutex<Option<crate::voice::Enrollment>>,
    /// The take's player (`Play the take`).
    pub player: crate::timeline::audio::fixture::FakePlayer,
    /// The recorder answers "cancelled" (a quit ended the take).
    pub clip_cancelled: Mutex<bool>,
}

/// The fake recorder's clip path.
pub const CLIP_PATH: &str = "/tmp/mnema-fixture-voice-take.m4a";

impl FixtureBackend {
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    /// The last save of `unit`'s settings, as sent.
    pub fn last_sent(&self, unit: Unit) -> Option<RecordingSettings> {
        let sent = self.sent.lock().unwrap();
        sent.iter()
            .rev()
            .find(|(u, _)| *u == unit)
            .map(|(_, s)| s.clone())
    }
    fn record(&self, c: Call) -> Result<(), String> {
        self.calls.lock().unwrap().push(c);
        if self.fail {
            Err("fixture: save failed".into())
        } else {
            Ok(())
        }
    }
}

impl Backend for FixtureBackend {
    fn save_domain(
        &self,
        unit: Unit,
        draft: &RecordingSettings,
    ) -> Result<RecordingSettings, String> {
        self.sent.lock().unwrap().push((unit, draft.clone()));
        self.record(Call::Domain(unit)).map(|_| draft.clone())
    }
    fn save_keyboard(&self, bindings: &Value) -> Result<Value, String> {
        self.record(Call::Keyboard).map(|_| bindings.clone())
    }
    fn save_privacy(&self, next: &RecordingSettings) -> Result<RecordingSettings, String> {
        self.record(Call::Privacy).map(|_| next.clone())
    }
    fn save_mic(
        &self,
        state: &MicrophoneControllerState,
    ) -> Result<MicrophoneControllerState, String> {
        self.record(Call::Mic).map(|_| state.clone())
    }
    fn prompt(&self, _: &str, mark: super::privacy_prompt::Mark) -> Result<(), String> {
        self.record(Call::Prompt(mark))
    }
    /// System audio now reads as possibly blocked.
    fn recheck_access(&self) -> Option<SettingsData> {
        let _ = self.record(Call::Recheck);
        Some(SettingsData {
            system_audio_blocked: true,
            ..Default::default()
        })
    }
    /// Every provider answers live (the parity mock's `verify_ai_provider`), unless `fail`.
    fn verify_provider(
        &self,
        _: &str,
        _: &[capture_types::AiProviderConfig],
    ) -> Result<(), String> {
        if self.fail {
            Err("unreachable".into())
        } else {
            Ok(())
        }
    }

    fn save_key(&self, provider: &str, _key: &str) -> Result<Option<String>, String> {
        self.record(Call::SaveKey(provider.into()))
            .map(|_| Some("9f2a".into()))
    }
    fn clear_key(&self, provider: &str) -> Result<(), String> {
        self.record(Call::ClearKey(provider.into()))
    }
    /// The parity map's `list_semantic_search_supported_models`.
    fn semantic_catalog(&self) -> Result<Vec<Value>, String> {
        if self.fail {
            return Err("fixture: catalog failed".into());
        }
        let map = load("settings", "default").map_err(|e| e.to_string())?;
        Ok(map["list_semantic_search_supported_models"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
    fn select_semantic_model(&self, model_id: &str) -> Result<u64, String> {
        self.record(Call::SelectSemantic(model_id.into()))
            .map(|_| 42)
    }
    fn record_clip(&self, ms: u64) -> Result<Option<String>, String> {
        self.record(Call::RecordClip(ms))?;
        Ok((!*self.clip_cancelled.lock().unwrap()).then(|| CLIP_PATH.into()))
    }
    fn cancel_clip(&self) {
        self.calls.lock().unwrap().push(Call::CancelClip);
    }
    fn mic_level(&self) -> Option<f32> {
        Some(0.5)
    }
    fn enroll(&self, _: &str) -> Result<crate::voice::Enrollment, String> {
        self.record(Call::Enroll)?;
        Ok(self
            .enrollment
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(crate::voice::Enrollment::Enrolled))
    }
    fn open_clip(
        &self,
        _: &str,
    ) -> Result<Box<dyn crate::timeline::audio::transport::Player + Send>, String> {
        Ok(Box::new(self.player.clone()))
    }
}
