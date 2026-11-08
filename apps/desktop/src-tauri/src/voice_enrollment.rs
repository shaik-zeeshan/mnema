//! Tauri commands for the voice enrollment door; the adapter lives in
//! `app_core::voice_enrollment`.

use std::sync::Arc;

use app_core::voice_enrollment::{
    self as enrollment, EnrollAccountOwnerVoiceRequest, VoiceEnrollmentOutcomeDto,
};
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub async fn enroll_account_owner_voice(
    request: EnrollAccountOwnerVoiceRequest,
    core: State<'_, Arc<AppCore>>,
    state: State<'_, AppInfraState>,
) -> Result<VoiceEnrollmentOutcomeDto, String> {
    enrollment::enroll_account_owner_voice(request, &core, &state).await
}

#[tauri::command]
pub async fn get_account_owner_person_id(
    state: State<'_, AppInfraState>,
) -> Result<Option<i64>, String> {
    enrollment::get_account_owner_person_id(&state).await
}
