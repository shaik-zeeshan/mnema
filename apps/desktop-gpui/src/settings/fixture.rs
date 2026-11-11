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

/// MCP golden states over Intelligence: `(state, the modal to open)`.
pub const MCP_STATES: [(&str, &str); 4] = [
    ("mcp-custom-form", "form"),
    ("mcp-tool-list", "tools"),
    ("mcp-oauth-idle", "oauth"),
    ("mcp-preset-connect", "connect"),
];

/// Provider-row golden states over Intelligence (`ProviderRow.svelte` opened).
pub const PROVIDER_STATES: [&str; 4] = [
    "provider-manage-local",
    "provider-manage-key",
    "provider-chatgpt",
    "provider-chatgpt-code",
];

/// Open one [`PROVIDER_STATES`] row the way a click would.
pub fn open_provider_state(
    s: &mut super::Settings,
    which: &str,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<super::Settings>,
) {
    super::sections::provider_row::fixture_state(s, which, window, cx);
}

/// Model-section golden states: `(state, ?tab=, what to set up)`.
pub const MODEL_STATES: [(&str, &str, &str); 15] = [
    ("models-unreadable", "ocr", "unreadable"),
    ("models-checking", "speakers", "checking"),
    ("speakers-installed", "speakers", "installed"),
    ("transcription-downloading", "transcription", "downloading"),
    ("semantic-reindexing", "semantic-search", "reindexing"),
    ("semantic-language", "semantic-search", "language"),
    ("mcp-authorizing", "intelligence", "authorizing"),
    ("ai-status-error", "intelligence", "ai-status-error"),
    ("ocr-unused-deleted", "ocr", "unused-deleted"),
    ("shortcuts-conflict", "shortcuts", "shortcut-conflict"),
    ("capture-sysaudio-hint", "capture", "sysaudio-hint"),
    ("video-custom-errors", "video", "video-errors"),
    ("storage-after", "storage", "storage-after"),
    ("license-overcap", "license", "license-overcap"),
    ("privacy-add-app", "privacy", "add-app"),
];

/// Set one [`MODEL_STATES`] state up on the loaded page.
pub fn open_model_state(
    s: &mut super::Settings,
    which: &str,
    cx: &mut gpui_kit::Context<super::Settings>,
) {
    use super::data::Family;
    match which {
        "unreadable" => {
            s.data.ocr_models = None;
            s.data.transcription_models = None;
            for f in [Family::Ocr, Family::Transcription] {
                s.data.model_errors.insert(
                    f,
                    "failed to inspect models: manifest.json: expected value at line 1 column 1"
                        .into(),
                );
            }
        }
        "checking" => {
            s.data.speaker_models = None;
            s.data.voice_checked = false;
            s.ui.models_loading = true;
        }
        "installed" => {
            if let Some(m) = s.data.speaker_models.as_mut() {
                let m = &mut m["providers"][0]["models"][0];
                m["available"] = Value::Bool(true);
                m["status"] = Value::String("installed".into());
            }
        }
        "downloading" => {
            s.ui.downloads.insert(
                "local_whisper:base".into(),
                (61_000_000, Some(148_000_000), "downloading".into()),
            );
        }
        "reindexing" => {
            s.ui.reindex = Some("Cleared 1,204 vectors; re-indexing in the background.".into());
        }
        "language" => {
            s.data.os_locale = "de-DE".into();
            s.edit(|d| d.semantic_search.model_id = None, cx);
        }
        "authorizing" => {
            s.edit(
                |d| {
                    if let Some(m) = d.ai_runtime.mcp_servers.first_mut() {
                        m.auth_mode = capture_types::McpAuthMode::OAuth;
                    }
                },
                cx,
            );
            s.data.mcp_oauth = vec![serde_json::json!({ "id": "github", "state": "authorizing" })];
        }
        "ai-status-error" => {
            s.data.ai_status = None;
            s.data.ai_status_error = Some("Ai runtime status: settings file is unreadable".into());
        }
        "unused-deleted" => {
            s.ui.unused_notes
                .insert(Family::Ocr, (true, "Deleted 2 unused OCR models.".into()));
        }
        "shortcut-conflict" => s.edit_keyboard(
            |k| k["appShortcuts"]["openSettings"] = Value::String("CommandOrControl+D".into()),
            cx,
        ),
        "sysaudio-hint" => {
            s.data.system_audio_blocked = true;
            s.edit(|d| d.capture_system_audio = true, cx);
        }
        "video-errors" => s.edit(
            |d| {
                d.capture_screen = true;
                d.screen_resolution = capture_types::ScreenResolution::Custom {
                    width: 1920,
                    height: 1080,
                };
                d.video_bitrate.mode = capture_types::VideoBitrateMode::Custom;
            },
            cx,
        ),
        "storage-after" => {
            s.ui.picked_location = Some("/Volumes/Archive/mnema".into());
            s.ui.cleanup = Some(Ok((12, 18_402, 9)));
            s.ui.delete_receipt = Some((
                crate::ui::Tone::Warn,
                super::sections::delete_receipt(300, 2).1,
            ));
        }
        "add-app" => {
            s.ui.app_picker = Some(vec![
                (
                    "com.agilebits.onepassword7".into(),
                    "1Password 7".into(),
                    true,
                ),
                ("com.bitwarden.desktop".into(), "Bitwarden".into(), false),
                ("com.apple.Passwords".into(), "Passwords".into(), false),
                ("org.mozilla.firefox".into(), "Firefox".into(), true),
            ]);
        }
        "license-overcap" => {
            s.data.license = Some(capture_types::LicenseStatus::Licensed {
                update_through_ms: 1_794_560_520_000,
                in_window: true,
                email: "ana@example.com".into(),
                name: "Ana".into(),
                activation: capture_types::Activation::RefusedOverCap {
                    reset_url: "https://mnema.day/help/devices".into(),
                    buy_url: "https://mnema.day/buy".into(),
                },
            });
        }
        _ => {}
    }
    cx.notify();
}

/// Open one [`MCP_STATES`] modal (`which`) over the page.
pub fn open_mcp_state(
    s: &mut super::Settings,
    which: &str,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<super::Settings>,
) {
    super::sections::mcp::fixture_state(s, which, window, cx);
}

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
        voice_checked: true,
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
    ChatgptBegin(String),
    ChatgptCancel(String),
    TestConnection,
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
    /// An MCP secret went to the vault (the secret itself is never recorded).
    McpSetSecret(String),
    McpClearSecret(String),
    McpTools(String),
    McpOAuthBegin(String),
    McpOAuthStatuses,
    McpCheckNode,
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
    /// What `mcp_tools` lists: `(name, description)`.
    pub mcp_tools: Mutex<Vec<(String, Option<String>)>>,
    /// What `mcp_oauth_statuses` answers (`{ id, state }`).
    pub mcp_oauth: Mutex<Vec<Value>>,
    /// `mcp_check_node` answers "not found".
    pub no_node: bool,
    /// When set, `mcp_tools` / `mcp_set_secret` / `mcp_oauth_begin` fail with it.
    pub mcp_error: Mutex<Option<String>>,
}

/// The fixture's ChatGPT device code and verify page.
pub const CHATGPT_CODE: &str = "ABCD-1234";
pub const CHATGPT_URL: &str = "https://auth.openai.com/codex/device";

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
    fn mcp_failure(&self) -> Result<(), String> {
        match self.mcp_error.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
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
    fn chatgpt_begin(&self, provider: &str) -> Result<(String, String), String> {
        self.record(Call::ChatgptBegin(provider.into()))?;
        Ok((CHATGPT_CODE.into(), CHATGPT_URL.into()))
    }
    fn chatgpt_cancel(&self, provider: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(Call::ChatgptCancel(provider.into()));
    }
    /// The default model answers (`ai_runtime_test_connection`'s shape).
    fn test_connection(&self) -> Result<Value, String> {
        self.record(Call::TestConnection)?;
        Ok(serde_json::json!({
            "ok": true, "provider": "anthropic", "model": "claude-sonnet-4-5",
            "message": "Connection succeeded.", "rawJson": "{\"text\":\"OK\"}"
        }))
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
    fn mcp_tools(&self, id: &str) -> Result<Vec<(String, Option<String>)>, String> {
        self.record(Call::McpTools(id.into()))?;
        self.mcp_failure()?;
        Ok(self.mcp_tools.lock().unwrap().clone())
    }
    fn mcp_set_secret(&self, id: &str, _secret: &str) -> Result<(), String> {
        self.record(Call::McpSetSecret(id.into()))?;
        self.mcp_failure()
    }
    fn mcp_clear_secret(&self, id: &str) -> Result<(), String> {
        self.record(Call::McpClearSecret(id.into()))
    }
    fn mcp_oauth_begin(&self, id: &str) -> Result<(), String> {
        self.record(Call::McpOAuthBegin(id.into()))?;
        self.mcp_failure()
    }
    fn mcp_oauth_statuses(&self) -> Option<Vec<Value>> {
        self.calls.lock().unwrap().push(Call::McpOAuthStatuses);
        Some(self.mcp_oauth.lock().unwrap().clone())
    }
    fn mcp_check_node(&self) -> Result<Option<String>, String> {
        self.calls.lock().unwrap().push(Call::McpCheckNode);
        Ok((!self.no_node).then(|| "v22.11.0".into()))
    }
}
