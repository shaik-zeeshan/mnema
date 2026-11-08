//! Tauri command for the Sensitive Capture Protection recommendations; the
//! logic lives in `app_core::sensitive_capture_recommendations`. The app list
//! comes from `native_capture::list_privacy_app_candidates` (moves with S3).

use std::sync::Arc;

use app_core::sensitive_capture_recommendations::{
    self as recommendations, SensitiveCaptureRecommendationsResponse,
};
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub async fn get_sensitive_capture_recommendations(
    core: State<'_, Arc<AppCore>>,
) -> Result<SensitiveCaptureRecommendationsResponse, String> {
    let candidates = crate::native_capture::list_privacy_app_candidates().await?;
    Ok(recommendations::get_sensitive_capture_recommendations(
        &core,
        &candidates,
    ))
}
