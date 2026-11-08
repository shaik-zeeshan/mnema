//! Fills Chat's models from `AppCore`, always off the main thread
//! (`Tokio::spawn`). Views never call the backend; this file does.
//!
//! The conversation store and Ask AI commands still live in src-tauri until S3
//! moves `ask_ai` / `conversation` into app-core, so those calls answer with
//! the empty/unavailable value and a pointer to wire them.

use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::AppCore;
use capture_types::{AiEngineRef, AskAiScope, Conversation, RecordingSettings, TurnSnapshot};
use serde::Deserialize;

use super::model::{ChatData, Engine, History};
use crate::ui::model_picker::{PoolModel, ProviderFailure};

pub async fn load(core: &AppCore) -> ChatData {
    let settings = core
        .try_state::<RecordingSettingsState>()
        .map(|s| current_recording_settings(&s));
    ChatData {
        // shortcut: list_conversations not in app-core until S3 (conversation); wire it then.
        history: History::Rows(Vec::new()),
        // shortcut: ask_ai_availability not in app-core until S3 (ask_ai); None keeps
        // Send disabled, like the Svelte composer while availability loads.
        availability: None,
        engine: settings.as_ref().map(engine).unwrap_or_default(),
    }
}

/// The engine facts from the recording settings (`askAiDefaultEngine` + friends).
pub fn engine(s: &RecordingSettings) -> Engine {
    let ai = &s.ai_runtime;
    let pair = |e: &AiEngineRef| (e.provider.clone(), e.model.clone());
    let global = ai
        .default_model
        .as_ref()
        .filter(|d| !d.model.trim().is_empty())
        .map(pair);
    let model = s.access.ask_ai_model.as_deref().unwrap_or("").trim();
    let provider = s
        .access
        .ask_ai_model_provider
        .as_deref()
        .unwrap_or("")
        .trim();
    let default = if model.is_empty()
        || provider.is_empty()
        || !ai.providers.iter().any(|p| p.id == provider)
    {
        global
    } else {
        Some((provider.to_string(), model.to_string()))
    };
    Engine {
        providers: ai.providers.clone(),
        default,
        favorites: ai.favorite_models.iter().map(pair).collect(),
        recents: ai.recent_models.iter().map(pair).collect(),
        web_fetch: s.access.ask_ai_web_fetch_enabled,
        connectors: ai.mcp_servers.iter().filter(|m| m.enabled).count(),
    }
}

/// `get_conversation`: `Ok(None)` = deleted.
pub async fn open(_core: &AppCore, _id: &str) -> Result<Option<Conversation>, String> {
    // shortcut: get_conversation (with parse-on-read) not in app-core until S3; wire it then.
    Err("Chats can't be opened in this build yet.".into())
}

/// `ask_ai_snapshot`: the in-flight turn's live view, if one is running.
pub async fn snapshot(_core: &AppCore, _id: &str) -> Option<TurnSnapshot> {
    // shortcut: ask_ai_snapshot not in app-core until S3 (ask_ai); wire it then.
    None
}

pub struct SendRequest {
    pub conversation_id: String,
    pub question: String,
    pub title: Option<String>,
    pub first: bool,
    pub scope: AskAiScope,
}

/// `ask_ai_start` (first turn) / `ask_ai_followup`.
pub async fn send(_core: &AppCore, _req: SendRequest) -> Result<(), String> {
    // shortcut: ask_ai_start / ask_ai_followup not in app-core until S3; wire them then.
    Err("ask_ai_unavailable".into())
}

pub async fn cancel(_core: &AppCore, _id: &str) {
    // shortcut: ask_ai_cancel not in app-core until S3; wire it then.
}

pub async fn set_pinned(_core: &AppCore, _id: &str, _pinned: bool) -> Result<(), String> {
    // shortcut: set_conversation_pinned not in app-core until S3; wire it then.
    Err("not wired until S3".into())
}

pub async fn set_title(_core: &AppCore, _id: &str, _title: &str) -> Result<(), String> {
    // shortcut: set_conversation_title not in app-core until S3; wire it then.
    Err("not wired until S3".into())
}

pub async fn delete(_core: &AppCore, _id: &str) -> Result<(), String> {
    // shortcut: delete_conversation not in app-core until S3; wire it then.
    Err("not wired until S3".into())
}

pub async fn delete_last_turn(_core: &AppCore, _id: &str, _turn: i64) -> Result<(), String> {
    // shortcut: delete_last_turn not in app-core until S3; wire it then.
    Err("not wired until S3".into())
}

pub async fn set_engine(_core: &AppCore, _id: &str, _engine: Option<(String, String)>) {
    // shortcut: set_conversation_engine not in app-core until S3; wire it then.
}

/// `ai_runtime_list_models` (in app-core): the merged pool plus per-provider failures.
pub async fn list_models(core: &AppCore) -> (Vec<PoolModel>, Vec<ProviderFailure>) {
    #[derive(Deserialize)]
    struct Listed {
        models: Vec<PoolModel>,
        failures: Vec<ProviderFailure>,
    }
    match app_core::ai_runtime::ai_runtime_list_models(core, None).await {
        // The result's fields are private to app-core; its serde shape is the contract.
        Ok(result) => serde_json::to_value(result)
            .and_then(serde_json::from_value::<Listed>)
            .map(|l| (l.models, l.failures))
            .unwrap_or_default(),
        Err(e) => {
            log::warn!("chat: model listing failed: {e}");
            Default::default()
        }
    }
}
