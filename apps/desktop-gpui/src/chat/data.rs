//! Fills Chat's models from `AppCore`, always off the main thread
//! (`Tokio::spawn`). Views never call the backend; this file does.

use std::sync::Arc;

use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::{conversation as store, AppCore, AppInfraState};
use capture_types::{AiEngineRef, AskAiScope, Conversation, RecordingSettings, TurnSnapshot};
use serde::Deserialize;

use super::model::{Availability, ChatData, Engine, History};
use crate::ui::model_picker::{PoolModel, ProviderFailure};

/// `refreshHistory`'s cap, for the list and for a search.
const HISTORY_LIMIT: i64 = 60;

fn infra(core: &AppCore) -> Result<AppInfraState, String> {
    core.try_state::<AppInfraState>()
        .map(|i| i.as_ref().clone())
        .ok_or_else(|| "The capture index isn't open yet.".to_string())
}

/// The list (`search_conversations` when `query` isn't empty, else
/// `list_conversations`), `ask_ai_availability` and the engine facts.
pub async fn load(core: &AppCore, query: &str) -> ChatData {
    let settings = core
        .try_state::<RecordingSettingsState>()
        .map(|s| current_recording_settings(&s));
    let rows = match infra(core) {
        Ok(i) if query.is_empty() => {
            store::list_conversations(&i, Some(HISTORY_LIMIT), Some(0)).await
        }
        Ok(i) => store::search_conversations(&i, query.to_string(), Some(HISTORY_LIMIT)).await,
        Err(e) => Err(e),
    };
    // A failure is an error state, never an empty list ("no chats").
    let history = rows.map_or(History::Error, History::Rows);
    // None keeps Send disabled, like the Svelte composer while availability loads.
    let availability = match core.try_state::<RecordingSettingsState>() {
        Some(_) => app_core::ask_ai::ask_ai_availability(core)
            .await
            .ok()
            .map(|a| Availability {
                available: a.available,
                reason: a.reason,
            }),
        None => None,
    };
    ChatData {
        history,
        query: query.to_string(),
        availability,
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

/// `get_conversation` (parse-on-read for legacy turns): `Ok(None)` = deleted.
pub async fn open(core: &AppCore, id: &str) -> Result<Option<Conversation>, String> {
    store::get_conversation(&infra(core)?, id.to_string()).await
}

/// `ask_ai_snapshot`: the in-flight turn's live view, if one is running.
pub async fn snapshot(_core: &AppCore, id: &str) -> Option<TurnSnapshot> {
    let request = app_core::ask_ai::AskAiSnapshotRequest {
        conversation_id: id.to_string(),
    };
    app_core::ask_ai::ask_ai_snapshot(request)
        .await
        .ok()
        .flatten()
}

pub struct SendRequest {
    pub conversation_id: String,
    pub question: String,
    pub title: Option<String>,
    pub first: bool,
    pub scope: AskAiScope,
}

/// `ask_ai_start` (first turn) / `ask_ai_followup`, with the local clock the
/// Svelte doors send (`askAiClock.ts`).
pub async fn send(core: &Arc<AppCore>, req: SendRequest) -> Result<(), String> {
    let mut body = serde_json::json!({
        "conversationId": req.conversation_id,
        "question": req.question,
        "utcOffsetMinutes": crate::native::sys::local_offset().whole_minutes(),
        "scope": req.scope,
    });
    // The request types' fields are app-core's wire contract; build them as the wire does.
    if req.first {
        body["origin"] = "chat".into();
        body["title"] = req.title.into();
        let request = serde_json::from_value(body).map_err(|e| e.to_string())?;
        app_core::ask_ai::ask_ai_start(core, request).await
    } else {
        let request = serde_json::from_value(body).map_err(|e| e.to_string())?;
        app_core::ask_ai::ask_ai_followup(core, request).await
    }
}

pub async fn cancel(_core: &AppCore, id: &str) {
    let request = app_core::ask_ai::AskAiCancelRequest {
        conversation_id: id.to_string(),
        discard: false,
    };
    let _ = app_core::ask_ai::ask_ai_cancel(request).await;
}

/// `origin: "chat"` creates the row if the chat has no turn yet, as the Svelte store does.
pub async fn set_pinned(core: &AppCore, id: &str, pinned: bool) -> Result<(), String> {
    let origin = Some("chat".to_string());
    store::set_conversation_pinned(core, &infra(core)?, id.to_string(), pinned, origin).await
}

pub async fn set_title(core: &AppCore, id: &str, title: &str) -> Result<(), String> {
    let request = store::SetConversationTitleRequest {
        conversation_id: id.to_string(),
        title: title.to_string(),
        origin: Some("chat".to_string()),
    };
    store::set_conversation_title(core, &infra(core)?, request).await
}

pub async fn delete(core: &AppCore, id: &str) -> Result<(), String> {
    store::delete_conversation(core, &infra(core)?, id.to_string()).await
}

pub async fn delete_last_turn(core: &AppCore, id: &str, turn: i64) -> Result<(), String> {
    store::delete_last_turn(core, &infra(core)?, id.to_string(), turn).await
}

/// `set_conversation_engine`: `None` clears the pin.
pub async fn set_engine(core: &AppCore, id: &str, engine: Option<(String, String)>) {
    let (provider, model) = engine.unzip();
    let request = store::SetConversationEngineRequest {
        conversation_id: id.to_string(),
        origin: "chat".to_string(),
        provider,
        model,
    };
    let result = match infra(core) {
        Ok(i) => store::set_conversation_engine(core, &i, request).await,
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        log::warn!("chat: engine pin failed: {e}");
    }
}

/// `open_captured_url`: the backend resolves the raw URL behind the guarded
/// display text and opens it in the browser.
pub async fn open_captured_url(core: &AppCore, frame_id: i64) -> Result<bool, String> {
    app_core::app_infra::open_captured_url(frame_id, core, &infra(core)?).await
}

/// Star / unstar a model: the starred list is an AI setting, saved wholesale
/// (`update_ai_runtime_settings { favoriteModels }`).
pub fn toggle_favorite(core: &Arc<AppCore>, engine: (String, String)) -> Result<(), String> {
    let state = core
        .try_state::<RecordingSettingsState>()
        .ok_or("Settings aren't loaded yet.")?;
    let mut favorites = current_recording_settings(&state)
        .ai_runtime
        .favorite_models;
    let before = favorites.len();
    favorites.retain(|f| (f.provider.as_str(), f.model.as_str()) != (&*engine.0, &*engine.1));
    if favorites.len() == before {
        favorites.push(AiEngineRef {
            provider: engine.0,
            model: engine.1,
        });
    }
    let request = capture_types::UpdateAiRuntimeSettingsRequest {
        favorite_models: Some(favorites),
        ..Default::default()
    };
    app_core::native_capture::settings_update::update_ai_runtime_settings(request, core)
        .map(drop)
        .map_err(|e| e.message)
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
