use std::sync::Arc;

use app_core::debug_status::{
    self as status, AppLogTailDto, SemanticIndexStatusDto, SemanticWorkerHealthState,
};
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub async fn get_semantic_index_status(
    infra: State<'_, AppInfraState>,
    worker_health: State<'_, SemanticWorkerHealthState>,
) -> Result<SemanticIndexStatusDto, String> {
    status::get_semantic_index_status(infra.inner(), worker_health.inner()).await
}

#[tauri::command]
pub async fn list_frame_batches(
    infra: State<'_, AppInfraState>,
    limit: Option<i64>,
) -> Result<Vec<::app_infra::FrameBatch>, String> {
    status::list_frame_batches(infra.inner(), limit).await
}

#[tauri::command]
pub async fn list_user_context_derivation_runs(
    infra: State<'_, AppInfraState>,
    limit: Option<i64>,
) -> Result<Vec<::app_infra::DerivationRun>, String> {
    status::list_user_context_derivation_runs(infra.inner(), limit).await
}

#[tauri::command]
pub async fn tail_app_log(
    core: State<'_, Arc<AppCore>>,
    lines: Option<usize>,
) -> Result<AppLogTailDto, String> {
    status::tail_app_log(&core, lines).await
}
