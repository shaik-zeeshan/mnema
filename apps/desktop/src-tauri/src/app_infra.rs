use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use tauri::Manager;

pub type AppInfraState = Arc<::app_infra::AppInfra>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitDebugCpuJobRequest {
    pub document_name: String,
    pub source_text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppJobRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppJobDto {
    pub id: i64,
    pub kind: String,
    pub status: ::app_infra::BackgroundJobStatus,
    pub payload_json: Option<String>,
    pub result_text: Option<String>,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl From<::app_infra::BackgroundJob> for AppJobDto {
    fn from(job: ::app_infra::BackgroundJob) -> Self {
        Self {
            id: job.id,
            kind: job.kind,
            status: job.status,
            payload_json: job.payload_json,
            result_text: job.result_text,
            attempt_count: job.attempt_count,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }
}

impl From<SubmitDebugCpuJobRequest> for ::app_infra::DebugCpuJobRequest {
    fn from(request: SubmitDebugCpuJobRequest) -> Self {
        Self {
            document_name: request.document_name,
            source_text: request.source_text,
        }
    }
}

pub fn initialize(app: &mut tauri::App) -> Result<(), String> {
    let base_dir = resolve_base_dir(app.handle())?;
    let infra = tauri::async_runtime::block_on(::app_infra::AppInfra::initialize(&base_dir))
        .map_err(|error| {
            format!(
                "failed to initialize app infrastructure at {}: {error}",
                base_dir.display()
            )
        })?;

    if !app.manage(Arc::new(infra)) {
        return Err("app infrastructure state was already initialized".to_string());
    }

    Ok(())
}

fn resolve_base_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app local data directory: {error}"))
}

#[tauri::command]
pub async fn get_app_infra_status(
    state: tauri::State<'_, AppInfraState>,
) -> Result<::app_infra::AppInfraStatus, String> {
    let infra = Arc::clone(&*state);
    infra
        .status()
        .await
        .map_err(|error| format!("failed to read app infrastructure status: {error}"))
}

#[tauri::command]
pub async fn submit_debug_cpu_job(
    request: SubmitDebugCpuJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<AppJobDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .submit_debug_cpu_job(request.into())
        .await
        .map(AppJobDto::from)
        .map_err(|error| format!("failed to submit debug cpu job: {error}"))
}

#[tauri::command]
pub async fn list_app_jobs(
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_jobs()
        .await
        .map(|jobs| jobs.into_iter().map(AppJobDto::from).collect())
        .map_err(|error| format!("failed to list app jobs: {error}"))
}

#[tauri::command]
pub async fn get_app_job(
    request: GetAppJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_job(request.job_id)
        .await
        .map(|job| job.map(AppJobDto::from))
        .map_err(|error| format!("failed to get app job {}: {error}", request.job_id))
}
