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
