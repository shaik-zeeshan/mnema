use app_core::debug_pipeline::{self as pipeline, ListProcessingJobsRequest, ProcessingJobPage};
use app_core::AppInfraState;
use tauri::State;

#[tauri::command]
pub async fn get_processing_pipeline_status(
    state: State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::ProcessorPipelineStatus>, String> {
    pipeline::get_processing_pipeline_status(state.inner()).await
}

#[tauri::command]
pub async fn list_processing_jobs_by_processor(
    request: ListProcessingJobsRequest,
    state: State<'_, AppInfraState>,
) -> Result<ProcessingJobPage, String> {
    pipeline::list_processing_jobs_by_processor(request, state.inner()).await
}
