use std::sync::Arc;

use app_core::speaker_analysis_models::{
    self as models, SpeakerAnalysisModelDownloadProgressDto,
    SpeakerAnalysisModelDownloadRequestDto, SpeakerAnalysisModelStatusResponseDto,
};
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub fn get_speaker_analysis_model_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<SpeakerAnalysisModelStatusResponseDto, String> {
    models::get_speaker_analysis_model_status(&core)
}

#[tauri::command]
pub fn start_speaker_analysis_model_download(
    core: State<'_, Arc<AppCore>>,
    request: SpeakerAnalysisModelDownloadRequestDto,
) -> Result<SpeakerAnalysisModelDownloadProgressDto, String> {
    models::start_speaker_analysis_model_download(&core, request)
}

#[tauri::command]
pub fn cancel_speaker_analysis_model_download(core: State<'_, Arc<AppCore>>) -> Result<(), String> {
    models::cancel_speaker_analysis_model_download(&core)
}

#[tauri::command]
pub async fn delete_speaker_analysis_model(
    core: State<'_, Arc<AppCore>>,
    request: SpeakerAnalysisModelDownloadRequestDto,
    infra: State<'_, AppInfraState>,
) -> Result<(), String> {
    models::delete_speaker_analysis_model(&core, request, &infra).await
}
