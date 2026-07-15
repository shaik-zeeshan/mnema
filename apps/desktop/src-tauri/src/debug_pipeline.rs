//! Read-side debug commands over the processing-job queue. These are the drill-in half of the
//! debug page: one aggregate per processor lane, and a paged listing of that lane's jobs.

use std::sync::Arc;

use serde::Deserialize;

use crate::app_infra::AppInfraState;

const DEFAULT_JOB_PAGE_LIMIT: i64 = 50;
/// Ceiling on one page of jobs. The debug listing is a paged drill-in, not an export, so a
/// caller asking for more is clamped rather than allowed to pull the whole queue into the webview.
const MAX_JOB_PAGE_LIMIT: i64 = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingJobsRequest {
    pub processor: String,
    /// `None` lists every status.
    pub status: Option<::app_infra::ProcessingJobStatus>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[tauri::command]
pub async fn get_processing_pipeline_status(
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::ProcessorPipelineStatus>, String> {
    let infra = Arc::clone(&*state);
    infra
        .processing_pipeline_status()
        .await
        .map_err(|error| format!("failed to read processing pipeline status: {error}"))
}

#[tauri::command]
pub async fn list_processing_jobs_by_processor(
    request: ListProcessingJobsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::ProcessingJobListing>, String> {
    let infra = Arc::clone(&*state);
    let limit = request
        .limit
        .unwrap_or(DEFAULT_JOB_PAGE_LIMIT)
        .clamp(0, MAX_JOB_PAGE_LIMIT);
    let offset = request.offset.unwrap_or(0).max(0);

    infra
        .list_processing_jobs_by_processor(&request.processor, request.status, limit, offset)
        .await
        .map_err(|error| format!("failed to list processing jobs: {error}"))
}
