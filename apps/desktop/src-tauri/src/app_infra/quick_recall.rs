use std::sync::Arc;

use app_core::app_infra::quick_recall::{
    self as core_qr, CountSearchFacetRequest, GetFrameMatchBoxesRequest, QuickRecallIndexStatusDto,
};
use app_core::semantic_search_query::SemanticQueryEmbedderState;
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub async fn get_frame_match_boxes(
    request: GetFrameMatchBoxesRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::MatchBox>, String> {
    core_qr::get_frame_match_boxes(request, &state).await
}

#[tauri::command]
pub async fn get_quick_recall_recent_queries(
    state: State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::RecentQuery>, String> {
    core_qr::get_quick_recall_recent_queries(&state).await
}

#[tauri::command]
pub async fn set_quick_recall_recent_queries(
    queries: Vec<::app_infra::RecentQuery>,
    state: State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::RecentQuery>, String> {
    core_qr::set_quick_recall_recent_queries(queries, &state).await
}

#[tauri::command]
pub async fn count_search_facet(
    request: CountSearchFacetRequest,
    core: State<'_, Arc<AppCore>>,
    state: State<'_, AppInfraState>,
    query_embedder: State<'_, SemanticQueryEmbedderState>,
) -> Result<::app_infra::SearchFacetCounts, String> {
    core_qr::count_search_facet(request, &core, &state, &query_embedder).await
}

#[tauri::command]
pub async fn get_quick_recall_index_status(
    core: State<'_, Arc<AppCore>>,
    state: State<'_, AppInfraState>,
) -> Result<QuickRecallIndexStatusDto, String> {
    core_qr::get_quick_recall_index_status(&core, &state).await
}
