//! Tauri commands for persistent conversations (issue #102, ADR 0031); the
//! adapters over the shared store live in `app_core::conversation`.

use std::sync::Arc;

use app_core::conversation::{
    self as core_conversation, SetConversationEngineRequest, SetConversationTitleRequest,
};
use app_core::{AppCore, AppInfraState};
use capture_types::{Conversation, ConversationSummary};
use tauri::State;

#[tauri::command]
pub async fn list_conversations(
    infra: State<'_, AppInfraState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<ConversationSummary>, String> {
    core_conversation::list_conversations(&infra, limit, offset).await
}

#[tauri::command]
pub async fn get_conversation(
    infra: State<'_, AppInfraState>,
    conversation_id: String,
) -> Result<Option<Conversation>, String> {
    core_conversation::get_conversation(&infra, conversation_id).await
}

#[tauri::command]
pub async fn search_conversations(
    infra: State<'_, AppInfraState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<ConversationSummary>, String> {
    core_conversation::search_conversations(&infra, query, limit).await
}

#[tauri::command]
pub async fn set_conversation_engine(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    request: SetConversationEngineRequest,
) -> Result<(), String> {
    core_conversation::set_conversation_engine(&core, &infra, request).await
}

#[tauri::command]
pub async fn set_conversation_title(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    request: SetConversationTitleRequest,
) -> Result<(), String> {
    core_conversation::set_conversation_title(&core, &infra, request).await
}

#[tauri::command]
pub async fn set_conversation_pinned(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    conversation_id: String,
    pinned: bool,
    origin: Option<String>,
) -> Result<(), String> {
    core_conversation::set_conversation_pinned(&core, &infra, conversation_id, pinned, origin).await
}

#[tauri::command]
pub async fn delete_last_turn(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    conversation_id: String,
    turn_index: i64,
) -> Result<(), String> {
    core_conversation::delete_last_turn(&core, &infra, conversation_id, turn_index).await
}

#[tauri::command]
pub async fn delete_conversation(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    conversation_id: String,
) -> Result<(), String> {
    core_conversation::delete_conversation(&core, &infra, conversation_id).await
}
