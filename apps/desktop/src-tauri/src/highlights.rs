//! Tauri command surface for the read-time **day highlights**: conversations
//! and moments.
//!
//! Thin adapter over `app_infra::HighlightsStore` (CLAUDE.md boundary: app-infra
//! owns SQLite, Tauri handlers stay thin). No aggregation here — it forwards the
//! half-open `[startMs, endMs)` window and maps the store error onto the
//! `Result<_, String>` Tauri seam.

use capture_types::{ConversationCluster, Moment};

use crate::app_infra::AppInfraState;

/// Activities in `[start_ms, end_ms)` whose window overlaps recorded speech for
/// at least two minutes, newest first, with the number of distinct speakers
/// heard in each.
#[tauri::command]
pub async fn get_conversations(
    infra: tauri::State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<ConversationCluster>, String> {
    infra
        .highlights()
        .conversations(start_ms, end_ms)
        .await
        .map_err(|e| e.to_string())
}

/// Headline frames of Activities in `[start_ms, end_ms)`, ranked by the
/// Activity's focus band then its duration. `limit` defaults to the store's
/// default when omitted.
#[tauri::command]
pub async fn get_moments(
    infra: tauri::State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
    limit: Option<i64>,
) -> Result<Vec<Moment>, String> {
    infra
        .highlights()
        .moments(start_ms, end_ms, limit)
        .await
        .map_err(|e| e.to_string())
}
