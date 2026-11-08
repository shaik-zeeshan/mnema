//! Tauri commands for Ask AI (Quick Recall + Chat); the turn driver, the live
//! `TurnView` registry and the MCP connectors live in `app_core::ask_ai`.

pub(crate) mod mcp_commands;

use std::sync::Arc;

use app_core::ask_ai::{
    self as core_ask_ai, AskAiAvailability, AskAiCancelRequest, AskAiFollowupRequest,
    AskAiSnapshotRequest, AskAiStartRequest, AskAiTokenUsage,
};
use app_core::AppCore;
use capture_types::TurnSnapshot;
use tauri::State;

#[tauri::command]
pub fn get_ask_ai_last_turn_usage() -> Option<AskAiTokenUsage> {
    core_ask_ai::get_ask_ai_last_turn_usage()
}

#[tauri::command]
pub async fn ask_ai_availability(
    core: State<'_, Arc<AppCore>>,
) -> Result<AskAiAvailability, String> {
    core_ask_ai::ask_ai_availability(&core).await
}

#[tauri::command]
pub async fn ask_ai_start(
    core: State<'_, Arc<AppCore>>,
    request: AskAiStartRequest,
) -> Result<(), String> {
    core_ask_ai::ask_ai_start(&core, request).await
}

#[tauri::command]
pub async fn ask_ai_followup(
    core: State<'_, Arc<AppCore>>,
    request: AskAiFollowupRequest,
) -> Result<(), String> {
    core_ask_ai::ask_ai_followup(&core, request).await
}

#[tauri::command]
pub async fn ask_ai_cancel(request: AskAiCancelRequest) -> Result<(), String> {
    core_ask_ai::ask_ai_cancel(request).await
}

#[tauri::command]
pub async fn ask_ai_snapshot(
    request: AskAiSnapshotRequest,
) -> Result<Option<TurnSnapshot>, String> {
    core_ask_ai::ask_ai_snapshot(request).await
}
