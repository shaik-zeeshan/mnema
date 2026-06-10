//! Tauri commands for persistent conversations (issue #102, ADR 0031).
//!
//! ONE shared store backs both Quick Recall and Chat. Commands are thin
//! adapters over `app_infra::ConversationStore`; after a save/delete they emit
//! [`CONVERSATION_CHANGED_EVENT`] so any open conversation surface refreshes
//! (mirrors `user_context_changed`).

use capture_types::{Conversation, ConversationSummary};
use serde::Deserialize;
use tauri::Emitter;

use crate::app_infra::AppInfraState;

/// Frontend refresh event emitted after a conversation is saved or deleted.
pub const CONVERSATION_CHANGED_EVENT: &str = "conversation_changed";

/// "Now" in unix milliseconds (UTC), stamped Rust-side on save so the store
/// stays deterministic. Mirrors the User Context worker's `now_ms`.
fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Persist (or update) one conversation turn. Carries everything needed to
/// upsert the conversation row AND the turn in one call. `toolActivities` /
/// `sources` are opaque JSON the frontend round-trips.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConversationTurnRequest {
    /// Frontend-generated UUID (the stable cross-restart identity).
    pub conversation_id: String,
    /// Conversation title for the upsert (the first non-empty title wins).
    #[serde(default)]
    pub title: String,
    /// The door that created it: `"quick_recall"` | `"chat"`.
    pub origin: String,
    pub turn_index: i64,
    pub question: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default = "empty_json_array")]
    pub tool_activities: serde_json::Value,
    #[serde(default = "empty_json_array")]
    pub sources: serde_json::Value,
    pub phase: String,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub seeded_result_count: Option<i64>,
}

fn empty_json_array() -> serde_json::Value {
    serde_json::Value::Array(Vec::new())
}

/// List conversations newest-first (by last update), each as a lightweight
/// summary (turn count + first-question preview). Default limit 50.
#[tauri::command]
pub async fn list_conversations(
    infra: tauri::State<'_, AppInfraState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<ConversationSummary>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let offset = offset.unwrap_or(0).max(0);
    infra
        .conversation()
        .list_conversations(limit, offset)
        .await
        .map_err(|e| e.to_string())
}

/// Hydrate one conversation (with its turns in order) by its frontend UUID.
#[tauri::command]
pub async fn get_conversation(
    infra: tauri::State<'_, AppInfraState>,
    conversation_id: String,
) -> Result<Option<Conversation>, String> {
    infra
        .conversation()
        .get_conversation(&conversation_id)
        .await
        .map_err(|e| e.to_string())
}

/// Case-insensitive search across conversation titles and turn
/// questions/answers. Newest-first, deduped per conversation. Default limit 50.
#[tauri::command]
pub async fn search_conversations(
    infra: tauri::State<'_, AppInfraState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<ConversationSummary>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    infra
        .conversation()
        .search_conversations(&query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Persist (or update in place) one conversation turn, ensuring the conversation
/// row exists first. Emits [`CONVERSATION_CHANGED_EVENT`].
#[tauri::command]
pub async fn save_conversation_turn(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    request: SaveConversationTurnRequest,
) -> Result<(), String> {
    let tool_activities_json =
        serde_json::to_string(&request.tool_activities).map_err(|e| e.to_string())?;
    let sources_json = serde_json::to_string(&request.sources).map_err(|e| e.to_string())?;

    infra
        .conversation()
        .save_turn(
            &request.conversation_id,
            &request.title,
            &request.origin,
            request.turn_index,
            &request.question,
            &request.answer,
            &tool_activities_json,
            &sources_json,
            &request.phase,
            request.error_message.as_deref(),
            request.seeded_result_count,
            now_ms(),
        )
        .await
        .map_err(|e| e.to_string())?;

    let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
    Ok(())
}

/// Pin (or clear) the Reasoning Engine identity for a conversation. `provider` /
/// `model` both absent/`null` clears the pin → unpinned (use the global default
/// engine). The conversation row is ensured first (a pin may be set before the
/// first turn). Emits [`CONVERSATION_CHANGED_EVENT`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetConversationEngineRequest {
    /// Frontend-generated UUID (the stable cross-restart identity).
    pub conversation_id: String,
    /// The engine provider id to pin, or `None` to clear.
    #[serde(default)]
    pub provider: Option<String>,
    /// The model id within `provider` to pin, or `None` to clear.
    #[serde(default)]
    pub model: Option<String>,
}

/// Pin (or clear) the engine identity for a conversation. Emits
/// [`CONVERSATION_CHANGED_EVENT`].
#[tauri::command]
pub async fn set_conversation_engine(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    request: SetConversationEngineRequest,
) -> Result<(), String> {
    infra
        .conversation()
        .set_conversation_engine(
            &request.conversation_id,
            request.provider.as_deref(),
            request.model.as_deref(),
            now_ms(),
        )
        .await
        .map_err(|e| e.to_string())?;

    let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
    Ok(())
}

/// Delete a conversation (its turns cascade). Emits
/// [`CONVERSATION_CHANGED_EVENT`].
#[tauri::command]
pub async fn delete_conversation(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    conversation_id: String,
) -> Result<(), String> {
    infra
        .conversation()
        .delete_conversation(&conversation_id)
        .await
        .map_err(|e| e.to_string())?;

    let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
    Ok(())
}
