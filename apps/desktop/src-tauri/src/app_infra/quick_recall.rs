//! Data commands only the Quick Recall window needs.

use std::sync::Arc;

use serde::Deserialize;

use super::AppInfraState;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameMatchBoxesRequest {
    pub frame_id: i64,
    pub query: String,
    /// The hit was meaning-only (`foundByMeaning`): boxes come back dashed.
    #[serde(default)]
    pub by_meaning: bool,
}

/// Match boxes for any frame — Quick Look's scrubbed neighbours, which are not
/// search hits themselves. Same rule `search_capture` fills `matchBoxes` with.
#[tauri::command]
pub async fn get_frame_match_boxes(
    request: GetFrameMatchBoxesRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::MatchBox>, String> {
    Arc::clone(&*state)
        .search()
        .frame_match_boxes(request.frame_id, &request.query, request.by_meaning)
        .await
        .map_err(|error| format!("failed to read match boxes: {error}"))
}

/// Quick Recall's recent queries, newest first (`app_settings`
/// `quick_recall.recent_queries`; never localStorage).
#[tauri::command]
pub async fn get_quick_recall_recent_queries(
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::RecentQuery>, String> {
    ::app_infra::load_recent_queries(state.read_pool())
        .await
        .map_err(|error| format!("failed to read recent queries: {error}"))
}

/// Replace the recent queries (add, re-run, ⌫-remove all go through here).
/// Returns what was kept: newest first, at most 8.
#[tauri::command]
pub async fn set_quick_recall_recent_queries(
    queries: Vec<::app_infra::RecentQuery>,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::RecentQuery>, String> {
    ::app_infra::store_recent_queries(state.write_pool(), queries)
        .await
        .map_err(|error| format!("failed to save recent queries: {error}"))
}
