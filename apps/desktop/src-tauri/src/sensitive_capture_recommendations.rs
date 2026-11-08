//! Tauri command for the Sensitive Capture Protection recommendations; the
//! logic and the app list live in app-core.

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
    recommendations::sensitive_capture_recommendations_now(&core).await
}
