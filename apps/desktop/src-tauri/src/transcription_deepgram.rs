use app_core::transcription_deepgram::{
    self as deepgram, DeepgramAuthStatusState, DeepgramProbeResult,
};
use tauri::State;

#[tauri::command]
pub async fn transcription_set_deepgram_key(
    key: String,
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<(), String> {
    deepgram::transcription_set_deepgram_key(key, auth.inner()).await
}

#[tauri::command]
pub async fn transcription_has_deepgram_key() -> Result<bool, String> {
    deepgram::transcription_has_deepgram_key().await
}

#[tauri::command]
pub async fn transcription_clear_deepgram_key(
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<(), String> {
    deepgram::transcription_clear_deepgram_key(auth.inner()).await
}

#[tauri::command]
pub async fn transcription_test_deepgram(
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<DeepgramProbeResult, String> {
    deepgram::transcription_test_deepgram(auth.inner()).await
}

#[tauri::command]
pub async fn transcription_deepgram_auth_status(
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<Option<String>, String> {
    deepgram::transcription_deepgram_auth_status(auth.inner()).await
}
