//! Tauri commands for the Semantic Search Model catalog; the behaviour lives in
//! `app_core::semantic_search_models`.

use std::sync::Arc;

use app_core::semantic_search_models::{
    self as models, SemanticSearchModelDownloadProgressDto, SemanticSearchModelDownloadRequestDto,
    SemanticSearchModelDownloadState, SemanticSearchModelStatusResponseDto, SupportedModelDto,
};
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub fn get_semantic_search_model_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<SemanticSearchModelStatusResponseDto, String> {
    models::get_semantic_search_model_status(&core)
}

#[tauri::command]
pub fn list_semantic_search_supported_models() -> Result<Vec<SupportedModelDto>, String> {
    models::list_semantic_search_supported_models()
}

#[tauri::command]
pub fn cancel_semantic_search_model_download(core: State<'_, Arc<AppCore>>) -> Result<(), String> {
    models::cancel_semantic_search_model_download(&core.state::<SemanticSearchModelDownloadState>())
}

#[tauri::command]
pub fn delete_semantic_search_model(
    core: State<'_, Arc<AppCore>>,
    request: SemanticSearchModelDownloadRequestDto,
) -> Result<(), String> {
    models::delete_semantic_search_model(
        &core,
        request,
        &core.state::<SemanticSearchModelDownloadState>(),
    )
}

#[tauri::command]
pub fn start_semantic_search_model_download(
    core: State<'_, Arc<AppCore>>,
    request: SemanticSearchModelDownloadRequestDto,
) -> Result<SemanticSearchModelDownloadProgressDto, String> {
    models::start_semantic_search_model_download(&core, request)
}

#[tauri::command]
pub async fn select_semantic_search_model(
    model_id: String,
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, app_core::AppInfraState>,
    settings_state: State<'_, app_core::recording_settings::RecordingSettingsState>,
) -> Result<u64, String> {
    models::select_semantic_search_model(model_id, &core, &infra, &settings_state).await
}
