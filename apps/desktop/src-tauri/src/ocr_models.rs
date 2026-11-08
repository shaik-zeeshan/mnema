use std::sync::Arc;

use app_core::ocr_models::{
    self as models, DeleteUnusedOcrModelsResponseDto, OcrModelDownloadProgressDto,
    OcrModelDownloadRequestDto, OcrModelStatusResponseDto,
};
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub fn get_ocr_model_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<OcrModelStatusResponseDto, String> {
    models::get_ocr_model_status(&core)
}

#[tauri::command]
pub fn start_ocr_model_download(
    core: State<'_, Arc<AppCore>>,
    request: OcrModelDownloadRequestDto,
) -> Result<OcrModelDownloadProgressDto, String> {
    models::start_ocr_model_download(&core, request)
}

#[tauri::command]
pub fn cancel_ocr_model_download(core: State<'_, Arc<AppCore>>) -> Result<(), String> {
    models::cancel_ocr_model_download(&core)
}

#[tauri::command]
pub async fn delete_unused_ocr_models(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
) -> Result<DeleteUnusedOcrModelsResponseDto, String> {
    models::delete_unused_ocr_models(&core, &infra).await
}
