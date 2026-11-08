//! Tauri commands for the Ask AI MCP connectors; the behaviour lives in
//! `app_core::ask_ai::mcp`.

use std::sync::Arc;

use app_core::ask_ai::mcp::manager::{self, McpToolDescriptor};
use app_core::ask_ai::mcp::oauth_flow::{self, McpOAuthStatus};
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub async fn mcp_warm_connectors(core: State<'_, Arc<AppCore>>) -> Result<(), String> {
    manager::mcp_warm_connectors(&core).await
}

#[tauri::command]
pub async fn mcp_list_server_tools(
    core: State<'_, Arc<AppCore>>,
    id: String,
) -> Result<Vec<McpToolDescriptor>, String> {
    manager::mcp_list_server_tools(&core, id).await
}

#[tauri::command]
pub async fn mcp_oauth_begin(core: State<'_, Arc<AppCore>>, id: String) -> Result<(), String> {
    oauth_flow::mcp_oauth_begin(&core, id).await
}

#[tauri::command]
pub async fn mcp_oauth_disconnect(core: State<'_, Arc<AppCore>>, id: String) -> Result<(), String> {
    oauth_flow::mcp_oauth_disconnect(&core, id).await
}

#[tauri::command]
pub async fn mcp_oauth_statuses(
    core: State<'_, Arc<AppCore>>,
) -> Result<Vec<McpOAuthStatus>, String> {
    oauth_flow::mcp_oauth_statuses(&core).await
}

#[tauri::command]
pub async fn mcp_check_node() -> Option<String> {
    app_core::ask_ai::mcp::node_check::mcp_check_node().await
}
