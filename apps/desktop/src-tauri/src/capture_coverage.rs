//! Tauri command surface for the timeline jump menu's per-day capture coverage
//! (round-4 decision G6).
//!
//! Thin adapter over `app_infra::CaptureCoverageStore` (app-infra owns SQLite;
//! the handler aggregates nothing itself). The store caches the result behind a
//! validity stamp, so calling this every time the menu opens is cheap.

use capture_types::DayCoverage;

use crate::app_infra::AppInfraState;

/// Every local day that holds capture, ascending. Days with no recording are
/// absent — that absence is what disables them in the jump menu.
#[tauri::command]
pub async fn list_day_coverage(
    infra: tauri::State<'_, AppInfraState>,
) -> Result<Vec<DayCoverage>, String> {
    infra
        .capture_coverage()
        .day_coverage()
        .await
        .map(|days| (*days).clone())
        .map_err(|error| format!("failed to load capture coverage: {error}"))
}
