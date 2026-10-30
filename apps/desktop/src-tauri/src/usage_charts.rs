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

/// Which capture families have anything in `[start_ms, end_ms)`. Overview uses
/// it to say "audio only" instead of "0h tracked" (app time is screen-only).
#[derive(serde::Serialize)]
pub struct CapturePresence {
    pub screen: bool,
    pub audio: bool,
}

#[tauri::command]
pub async fn capture_presence(
    infra: tauri::State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
) -> Result<CapturePresence, String> {
    let (screen, audio) = infra
        .usage_charts()
        .capture_presence(start_ms, end_ms)
        .await
        .map_err(|e| e.to_string())?;
    Ok(CapturePresence { screen, audio })
}

/// Local hour starts in `[start_ms, end_ms)` holding screen capture: the
/// Overview date picker's density, without `get_usage_charts`' per-frame load.
/// `offset_minutes` is the frontend's UTC offset (east-positive, IST = 330).
#[tauri::command]
pub async fn get_capture_hours(
    infra: tauri::State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
    offset_minutes: i32,
) -> Result<Vec<i64>, String> {
    infra
        .usage_charts()
        .capture_hours(start_ms, end_ms, offset_minutes)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::CapturePresence;

    /// The wire shape `Overview.svelte` reads (`{ screen, audio }`).
    #[test]
    fn capture_presence_serializes_to_the_ts_shape() {
        let value = serde_json::to_value(CapturePresence {
            screen: true,
            audio: false,
        })
        .unwrap();
        assert_eq!(value, serde_json::json!({ "screen": true, "audio": false }));
    }
}
