//! Tauri commands for the AI "Reasoning Engine"; the behaviour and the engine
//! resolver live in `app_core::ai_runtime`.

use std::sync::Arc;

use app_core::ai_runtime::{
    self as ai, AiRuntimeListModelsRequest, AiRuntimeModelsResult, AiRuntimeProviderKeyRequest,
    AiRuntimeProviderRequest, AiRuntimeStatus, AiRuntimeTestResult, AiRuntimeVerifyRequest,
    AiRuntimeVerifyResult, McpServerRequest, McpServerSecretRequest,
};
use app_core::chatgpt_auth::ChatgptLoginPrompt;
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub async fn ai_runtime_set_provider_key(
    request: AiRuntimeProviderKeyRequest,
) -> Result<(), String> {
    ai::ai_runtime_set_provider_key(request).await
}

#[tauri::command]
pub async fn ai_runtime_clear_provider_key(
    request: AiRuntimeProviderRequest,
) -> Result<(), String> {
    ai::ai_runtime_clear_provider_key(request).await
}

#[tauri::command]
pub async fn ai_runtime_has_provider_key(
    request: AiRuntimeProviderRequest,
) -> Result<bool, String> {
    ai::ai_runtime_has_provider_key(request).await
}

#[tauri::command]
pub async fn ai_runtime_provider_key_ending(
    request: AiRuntimeProviderRequest,
) -> Result<Option<String>, String> {
    ai::ai_runtime_provider_key_ending(request).await
}

#[tauri::command]
pub async fn ai_runtime_chatgpt_begin_login(
    core: State<'_, Arc<AppCore>>,
    request: AiRuntimeProviderRequest,
) -> Result<ChatgptLoginPrompt, String> {
    ai::ai_runtime_chatgpt_begin_login(&core, request).await
}

#[tauri::command]
pub async fn ai_runtime_chatgpt_cancel_login(
    request: AiRuntimeProviderRequest,
) -> Result<(), String> {
    ai::ai_runtime_chatgpt_cancel_login(request).await
}

#[tauri::command]
pub async fn mcp_set_server_secret(request: McpServerSecretRequest) -> Result<(), String> {
    ai::mcp_set_server_secret(request).await
}

#[tauri::command]
pub async fn mcp_clear_server_secret(request: McpServerRequest) -> Result<(), String> {
    ai::mcp_clear_server_secret(request).await
}

#[tauri::command]
pub async fn mcp_has_server_secret(request: McpServerRequest) -> Result<bool, String> {
    ai::mcp_has_server_secret(request).await
}

#[tauri::command]
pub async fn get_ai_runtime_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<AiRuntimeStatus, String> {
    ai::get_ai_runtime_status(&core).await
}

#[tauri::command]
pub async fn ai_runtime_test_connection(
    core: State<'_, Arc<AppCore>>,
) -> Result<AiRuntimeTestResult, String> {
    ai::ai_runtime_test_connection(&core).await
}

#[tauri::command]
pub async fn ai_runtime_list_models(
    core: State<'_, Arc<AppCore>>,
    request: Option<AiRuntimeListModelsRequest>,
) -> Result<AiRuntimeModelsResult, String> {
    ai::ai_runtime_list_models(&core, request).await
}

#[tauri::command]
pub async fn verify_ai_provider(
    core: State<'_, Arc<AppCore>>,
    request: AiRuntimeVerifyRequest,
) -> Result<AiRuntimeVerifyResult, String> {
    ai::verify_ai_provider(&core, request).await
}
