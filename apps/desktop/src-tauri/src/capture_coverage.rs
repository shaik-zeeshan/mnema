//! Tauri command surface for the timeline jump menu's per-day capture coverage
//! (round-4 decision G6).
//!
//! Thin adapter over `app_infra::CaptureCoverageStore` (app-infra owns SQLite;
//! the handler aggregates nothing itself). The store caches the result behind a
//! validity stamp, so calling this every time the menu opens is cheap.
//!
//! The Overview's **This Week** tile (round-4 decision G11: "same query family
//! as G6") reads the SAME command and keeps the last seven local days — seven
//! `covered_ms` totals is exactly what a 7-bar tile draws, and a second
//! aggregation over `capture_segments` would only be this one with a `WHERE`.
//! Absent days are absent here too: a week with a gap draws a zero bar.

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
