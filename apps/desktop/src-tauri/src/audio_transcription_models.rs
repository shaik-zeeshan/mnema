use std::sync::Arc;

use app_core::audio_transcription_models::{
    self as models, AudioTranscriptionModelDownloadProgressDto,
    AudioTranscriptionModelDownloadRequestDto, AudioTranscriptionModelStatusResponseDto,
    DeleteUnusedAudioTranscriptionModelsResponseDto,
};
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub fn get_audio_transcription_model_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<AudioTranscriptionModelStatusResponseDto, String> {
    models::get_audio_transcription_model_status(&core)
}

#[tauri::command]
pub fn start_audio_transcription_model_download(
    core: State<'_, Arc<AppCore>>,
    request: AudioTranscriptionModelDownloadRequestDto,
    infra: State<'_, AppInfraState>,
) -> Result<AudioTranscriptionModelDownloadProgressDto, String> {
    models::start_audio_transcription_model_download(&core, request, &infra)
}

#[tauri::command]
pub fn cancel_audio_transcription_model_download(
    core: State<'_, Arc<AppCore>>,
) -> Result<(), String> {
    models::cancel_audio_transcription_model_download(&core)
}

#[tauri::command]
pub async fn delete_unused_audio_transcription_models(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
) -> Result<DeleteUnusedAudioTranscriptionModelsResponseDto, String> {
    models::delete_unused_audio_transcription_models(&core, &infra).await
}

#[tauri::command]
pub fn request_apple_speech_recognition_permission(
    core: State<'_, Arc<AppCore>>,
) -> Result<AudioTranscriptionModelStatusResponseDto, String> {
    models::request_apple_speech_recognition_permission(&core)
}

#[tauri::command]
pub fn open_apple_speech_recognition_privacy_settings(
    core: State<'_, Arc<AppCore>>,
) -> Result<(), String> {
    models::open_apple_speech_recognition_privacy_settings(&core)
}
