//! Tauri command surface for the engine-free **Overview free-tier Usage
//! Charts** (issue #104).
//!
//! This is a thin adapter over `app_infra::UsageChartsStore` (per the CLAUDE.md
//! boundary rule: app-infra owns SQLite; Tauri handlers stay thin). It performs
//! no aggregation itself — it just forwards the optional range bounds and maps
//! the store error into the `Result<_, String>` Tauri seam.

use capture_types::UsageChartsResponse;

use crate::app_infra::AppInfraState;

/// Returns the engine-free Overview usage aggregations over an optional
/// `[start_ms, end_ms]` unix-millis range. Omit either bound (or both) to cover
/// the full retained history; the response echoes the resolved range in
/// `rangeStartMs` / `rangeEndMs`.
#[tauri::command]
pub async fn get_usage_charts(
    infra: tauri::State<'_, AppInfraState>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<UsageChartsResponse, String> {
    infra
        .usage_charts()
        .usage_charts(start_ms, end_ms)
        .await
        .map_err(|e| e.to_string())
}
