use app_core::usage_charts::{self as charts, CapturePresence};
use app_core::AppInfraState;
use capture_types::UsageChartsResponse;
use tauri::State;

#[tauri::command]
pub async fn get_usage_charts(
    infra: State<'_, AppInfraState>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<UsageChartsResponse, String> {
    charts::get_usage_charts(infra.inner(), start_ms, end_ms).await
}

#[tauri::command]
pub async fn capture_presence(
    infra: State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
) -> Result<CapturePresence, String> {
    charts::capture_presence(infra.inner(), start_ms, end_ms).await
}

#[tauri::command]
pub async fn get_capture_hours(
    infra: State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
    offset_minutes: i32,
) -> Result<Vec<i64>, String> {
    charts::get_capture_hours(infra.inner(), start_ms, end_ms, offset_minutes).await
}
