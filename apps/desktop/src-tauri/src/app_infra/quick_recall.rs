//! Data commands only the Quick Recall window needs.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::AppInfraState;
use crate::semantic_search_query::SemanticQueryEmbedderState;

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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountSearchFacetRequest {
    /// The same query `search_capture` ran, operators included.
    pub query: String,
    pub refinements: Option<::app_infra::SearchCaptureRefinements>,
    pub facet: ::app_infra::SearchFacet,
}

/// Result counts per pill option: the same search (meaning tier included) with
/// `facet`'s refinement swapped out, counted per bucket. Counts only.
#[tauri::command]
pub async fn count_search_facet(
    request: CountSearchFacetRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
    query_embedder: tauri::State<'_, SemanticQueryEmbedderState>,
) -> Result<::app_infra::SearchFacetCounts, String> {
    // Same embed as `search_capture`, so counts match what the search shows.
    let residual_query = ::app_infra::semantic_search_residual_query(&request.query);
    let query_embedding = crate::semantic_search_query::embed_search_query(
        &app_handle,
        &query_embedder,
        &residual_query,
    )
    .await;
    state
        .search()
        .count_search_facet(
            ::app_infra::SearchCaptureRequest {
                query: request.query,
                frame_limit: None,
                frame_offset: None,
                audio_limit: None,
                audio_offset: None,
                snapshot_document_id: None,
                refinements: request.refinements,
                query_embedding,
            },
            request.facet,
        )
        .await
        .map_err(|error| format!("failed to count search results: {error}"))
}

/// What Quick Recall's no-results / partial states say about indexing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickRecallIndexStatusDto {
    /// Frames captured but not yet read (OCR queued or running).
    pub queued_frames: i64,
    /// Capture time of the oldest unread frame, unix ms.
    pub oldest_queued_ms: Option<i64>,
    /// Search by meaning is on (model installed) but still embedding history.
    pub semantic_building: bool,
    /// How much of the history has meaning vectors, 0–100. `None` when meaning
    /// search is off (no model) — that is "keyword only", not "building".
    pub semantic_indexed_percent: Option<u8>,
}

/// User-facing indexing status: the OCR backlog plus the Semantic Index
/// backfill (`get_semantic_index_status`'s counts, without its debug half).
#[tauri::command]
pub async fn get_quick_recall_index_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
) -> Result<QuickRecallIndexStatusDto, String> {
    let backlog = state
        .search()
        .frame_reading_backlog()
        .await
        .map_err(|error| format!("failed to read the indexing backlog: {error}"))?;
    let settings = crate::semantic_search_worker::effective_semantic_search_settings(&app_handle);
    let meaning_on = app_handle
        .path()
        .app_data_dir()
        .is_ok_and(|dir| crate::semantic_search_worker::selected_model_available(&dir, &settings));
    let semantic_indexed_percent = if meaning_on {
        let store = state.semantic_search();
        let indexed = store
            .count_vectors()
            .await
            .map_err(|error| format!("failed to count semantic vectors: {error}"))?;
        let waiting = store
            .count_anchors_missing_vector()
            .await
            .map_err(|error| format!("failed to count semantic index backlog: {error}"))?;
        Some(indexed_percent(indexed, waiting))
    } else {
        None
    };
    Ok(QuickRecallIndexStatusDto {
        queued_frames: backlog.queued_frames,
        oldest_queued_ms: backlog.oldest_queued_ms,
        semantic_building: semantic_indexed_percent.is_some_and(|percent| percent < 100),
        semantic_indexed_percent,
    })
}

/// Rounded down, so a single waiting anchor never reads as "100%".
fn indexed_percent(indexed: i64, waiting: i64) -> u8 {
    let total = indexed.max(0) + waiting.max(0);
    if total == 0 {
        return 100;
    }
    (indexed.max(0) * 100 / total) as u8
}

#[cfg(test)]
mod tests {
    #[test]
    fn indexed_percent_rounds_down_and_treats_empty_as_done() {
        assert_eq!(super::indexed_percent(0, 0), 100);
        assert_eq!(super::indexed_percent(62, 38), 62);
        assert_eq!(super::indexed_percent(999, 1), 99);
    }
}
