use std::{path::PathBuf, sync::Arc, time::Duration};

use capture_screen::ScreenFrameArtifact;
use serde::{Deserialize, Serialize};
use tauri::Manager;

pub type AppInfraState = Arc<::app_infra::AppInfra>;

const APP_INFRA_BASE_DIR_NAME: &str = ".z";
const PROCESSING_WORKER_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const PROCESSING_WORKER_ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(2);

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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertFrameAndEnqueueProcessingJobRequest {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub processor: String,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertFrameAndEnqueueOcrRequest {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFramesRequest {
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProcessingJobRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingJobsRequest {
    pub subject_type: String,
    pub subject_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProcessingResultRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingResultsRequest {
    pub subject_type: String,
    pub subject_id: i64,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameDto {
    pub id: i64,
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJobDto {
    pub id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub status: ::app_infra::ProcessingJobStatus,
    pub attempt_count: i64,
    pub payload_json: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingResultDto {
    pub id: i64,
    pub job_id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub result_text: Option<String>,
    pub structured_payload_json: Option<String>,
    pub processor_version: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameProcessingJobDto {
    pub frame: FrameDto,
    pub job: ProcessingJobDto,
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

impl From<::app_infra::Frame> for FrameDto {
    fn from(frame: ::app_infra::Frame) -> Self {
        Self {
            id: frame.id,
            session_id: frame.session_id,
            file_path: frame.file_path,
            captured_at: frame.captured_at,
            width: frame.width,
            height: frame.height,
            content_fingerprint: frame.content_fingerprint,
            created_at: frame.created_at,
            updated_at: frame.updated_at,
        }
    }
}

impl From<::app_infra::ProcessingJob> for ProcessingJobDto {
    fn from(job: ::app_infra::ProcessingJob) -> Self {
        Self {
            id: job.id,
            subject_type: job.subject_type,
            subject_id: job.subject_id,
            processor: job.processor,
            status: job.status,
            attempt_count: job.attempt_count,
            payload_json: job.payload_json,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }
}

impl From<::app_infra::ProcessingResult> for ProcessingResultDto {
    fn from(result: ::app_infra::ProcessingResult) -> Self {
        Self {
            id: result.id,
            job_id: result.job_id,
            subject_type: result.subject_type,
            subject_id: result.subject_id,
            processor: result.processor,
            result_text: result.result_text,
            structured_payload_json: result.structured_payload_json,
            processor_version: result.processor_version,
            created_at: result.created_at,
        }
    }
}

impl From<::app_infra::FrameProcessingJob> for FrameProcessingJobDto {
    fn from(value: ::app_infra::FrameProcessingJob) -> Self {
        Self {
            frame: value.frame.into(),
            job: value.job.into(),
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

impl InsertFrameAndEnqueueProcessingJobRequest {
    fn into_frame_pipeline_request(self) -> ::app_infra::FramePipelineRequest {
        let Self {
            session_id,
            file_path,
            captured_at,
            width,
            height,
            content_fingerprint,
            processor,
            payload_json,
        } = self;

        let mut frame = ::app_infra::NewFrame::new(session_id, file_path, captured_at);

        if let (Some(width), Some(height)) = (width, height) {
            frame = frame.with_dimensions(width, height);
        }

        if let Some(content_fingerprint) = content_fingerprint {
            frame = frame.with_content_fingerprint(content_fingerprint);
        }

        let mut request = ::app_infra::FramePipelineRequest::new(frame, processor);

        if let Some(payload_json) = payload_json {
            request = request.with_payload_json(payload_json);
        }

        request
    }
}

impl From<InsertFrameAndEnqueueOcrRequest> for InsertFrameAndEnqueueProcessingJobRequest {
    fn from(request: InsertFrameAndEnqueueOcrRequest) -> Self {
        Self {
            session_id: request.session_id,
            file_path: request.file_path,
            captured_at: request.captured_at,
            width: request.width,
            height: request.height,
            content_fingerprint: request.content_fingerprint,
            processor: ::app_infra::OCR_PROCESSOR.to_string(),
            payload_json: request.payload_json,
        }
    }
}

fn captured_at_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn fingerprint_string(content_fingerprint: Option<u64>) -> Option<String> {
    content_fingerprint.map(|value| format!("{value:016x}"))
}

pub async fn persist_screen_frame_artifact(
    infra: &::app_infra::AppInfra,
    session_id: &str,
    artifact: ScreenFrameArtifact,
) -> ::app_infra::Result<::app_infra::FrameOcrEnqueueResult> {
    let mut frame = ::app_infra::NewFrame::new(
        session_id,
        artifact.file_path,
        captured_at_from_unix_ms(artifact.captured_at_unix_ms),
    );

    if let (Some(width), Some(height)) = (artifact.width, artifact.height) {
        frame = frame.with_dimensions(i64::from(width), i64::from(height));
    }

    if let Some(content_fingerprint) = fingerprint_string(artifact.content_fingerprint) {
        frame = frame.with_content_fingerprint(content_fingerprint);
    }

    infra
        .insert_frame_into_batch_and_maybe_enqueue_ocr_job(&frame, None)
        .await
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
    let infra = Arc::new(infra);

    if !app.manage(Arc::clone(&infra)) {
        return Err("app infrastructure state was already initialized".to_string());
    }

    spawn_processing_worker(infra);

    Ok(())
}

fn spawn_processing_worker(infra: AppInfraState) {
    tauri::async_runtime::spawn(async move {
        loop {
            match process_pending_jobs_once(&infra).await {
                Ok(Some(_)) => continue,
                Ok(None) => tokio::time::sleep(PROCESSING_WORKER_IDLE_POLL_INTERVAL).await,
                Err(error) => {
                    eprintln!("processing worker loop failed: {error}");
                    tokio::time::sleep(PROCESSING_WORKER_ERROR_RETRY_INTERVAL).await;
                }
            }
        }
    });
}

async fn process_pending_jobs_once(
    infra: &::app_infra::AppInfra,
) -> ::app_infra::Result<Option<()>> {
    let did_processing = infra.process_next_processing_job().await?.is_some();

    let did_finalize = match infra.process_next_frame_batch_job().await {
        Ok(result) => result.is_some(),
        Err(error) => {
            eprintln!("finalize job failed (state already updated): {error}");
            true
        }
    };

    if did_processing || did_finalize {
        Ok(Some(()))
    } else {
        Ok(None)
    }
}

fn resolve_base_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let settings = crate::native_capture_settings::load_recording_settings_or_default(app_handle);

    Ok(resolve_base_dir_from_save_directory(
        &settings.save_directory,
    ))
}

fn resolve_base_dir_from_save_directory(save_directory: &str) -> PathBuf {
    PathBuf::from(save_directory).join(APP_INFRA_BASE_DIR_NAME)
}

fn processing_subject(subject_type: String, subject_id: i64) -> ::app_infra::ProcessingSubject {
    ::app_infra::ProcessingSubject::new(subject_type, subject_id)
}

async fn insert_frame_and_enqueue_processing_job_inner(
    infra: &::app_infra::AppInfra,
    request: InsertFrameAndEnqueueProcessingJobRequest,
) -> ::app_infra::Result<FrameProcessingJobDto> {
    let request = request.into_frame_pipeline_request();

    infra
        .frame_pipeline()
        .enqueue(&request)
        .await
        .map(FrameProcessingJobDto::from)
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

#[tauri::command]
pub async fn insert_frame_and_enqueue_processing_job(
    request: InsertFrameAndEnqueueProcessingJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    insert_frame_and_enqueue_processing_job_inner(&infra, request)
        .await
        .map_err(|error| format!("failed to insert frame and enqueue processing job: {error}"))
}

#[tauri::command]
pub async fn insert_frame_and_enqueue_ocr(
    request: InsertFrameAndEnqueueOcrRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    insert_frame_and_enqueue_processing_job_inner(&infra, request.into())
        .await
        .map_err(|error| format!("failed to insert frame and enqueue ocr job: {error}"))
}

#[tauri::command]
pub async fn list_frames(
    request: Option<ListFramesRequest>,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    let session_id = request.and_then(|request| request.session_id);

    infra
        .list_frames(session_id.as_deref())
        .await
        .map(|frames| frames.into_iter().map(FrameDto::from).collect())
        .map_err(|error| format!("failed to list frames: {error}"))
}

#[tauri::command]
pub async fn get_frame(
    request: GetFrameRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get frame {}: {error}", request.frame_id))
}

#[tauri::command]
pub async fn list_processing_jobs(
    request: ListProcessingJobsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    infra
        .list_processing_jobs_for_subject(&subject)
        .await
        .map(|jobs| jobs.into_iter().map(ProcessingJobDto::from).collect())
        .map_err(|error| format!("failed to list processing jobs: {error}"))
}

#[tauri::command]
pub async fn get_processing_job(
    request: GetProcessingJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_processing_job(request.job_id)
        .await
        .map(|job| job.map(ProcessingJobDto::from))
        .map_err(|error| format!("failed to get processing job {}: {error}", request.job_id))
}

#[tauri::command]
pub async fn get_processing_result(
    request: GetProcessingResultRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<ProcessingResultDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_processing_result_for_job(request.job_id)
        .await
        .map(|result| result.map(ProcessingResultDto::from))
        .map_err(|error| {
            format!(
                "failed to get processing result for job {}: {error}",
                request.job_id
            )
        })
}

#[tauri::command]
pub async fn list_processing_results(
    request: ListProcessingResultsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<ProcessingResultDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    infra
        .list_processing_results_for_subject(&subject)
        .await
        .map(|results| results.into_iter().map(ProcessingResultDto::from).collect())
        .map_err(|error| format!("failed to list processing results: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("desktop-tauri-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    #[test]
    fn insert_frame_processing_request_maps_optional_dimensions() {
        let request = InsertFrameAndEnqueueProcessingJobRequest {
            session_id: "session-a".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-04-12T10:00:00Z".to_string(),
            width: Some(1280),
            height: Some(720),
            content_fingerprint: Some("abcd".to_string()),
            processor: "custom-processor".to_string(),
            payload_json: Some("{\"language\":\"eng\"}".to_string()),
        }
        .into_frame_pipeline_request();

        assert_eq!(request.frame.session_id, "session-a");
        assert_eq!(request.frame.file_path, "/tmp/frame.png");
        assert_eq!(request.frame.width, Some(1280));
        assert_eq!(request.frame.height, Some(720));
        assert_eq!(request.frame.content_fingerprint.as_deref(), Some("abcd"));
        assert_eq!(request.processor, "custom-processor");
        assert_eq!(
            request.payload_json.as_deref(),
            Some("{\"language\":\"eng\"}")
        );
    }

    #[test]
    fn insert_frame_processing_request_ignores_partial_dimensions() {
        let request = InsertFrameAndEnqueueProcessingJobRequest {
            session_id: "session-b".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-04-12T10:00:00Z".to_string(),
            width: Some(1280),
            height: None,
            content_fingerprint: None,
            processor: "custom-processor".to_string(),
            payload_json: None,
        }
        .into_frame_pipeline_request();

        assert_eq!(request.frame.width, None);
        assert_eq!(request.frame.height, None);
        assert_eq!(request.processor, "custom-processor");
        assert_eq!(request.payload_json, None);
    }

    #[test]
    fn insert_frame_ocr_request_wraps_generic_processing_request() {
        let request =
            InsertFrameAndEnqueueProcessingJobRequest::from(InsertFrameAndEnqueueOcrRequest {
                session_id: "session-ocr".to_string(),
                file_path: "/tmp/frame-ocr.png".to_string(),
                captured_at: "2026-04-12T10:00:00Z".to_string(),
                width: Some(1920),
                height: Some(1080),
                content_fingerprint: Some("ef01".to_string()),
                payload_json: Some("{\"language\":\"eng\"}".to_string()),
            })
            .into_frame_pipeline_request();

        assert_eq!(request.frame.session_id, "session-ocr");
        assert_eq!(request.frame.file_path, "/tmp/frame-ocr.png");
        assert_eq!(request.frame.width, Some(1920));
        assert_eq!(request.frame.height, Some(1080));
        assert_eq!(request.frame.content_fingerprint.as_deref(), Some("ef01"));
        assert_eq!(request.processor, ::app_infra::OCR_PROCESSOR);
        assert_eq!(
            request.payload_json.as_deref(),
            Some("{\"language\":\"eng\"}")
        );
    }

    #[test]
    fn resolve_base_dir_from_save_directory_uses_hidden_subdirectory() {
        let save_directory = "/tmp/z-recordings";

        assert_eq!(
            resolve_base_dir_from_save_directory(save_directory),
            PathBuf::from(save_directory).join(APP_INFRA_BASE_DIR_NAME)
        );
    }

    #[test]
    fn resolve_base_dir_from_save_directory_keeps_database_out_of_segment_root() {
        let save_directory = "/tmp/z-recordings/session-output";
        let base_dir = resolve_base_dir_from_save_directory(save_directory);

        assert_eq!(base_dir.parent(), Some(Path::new(save_directory)));
        assert_eq!(
            base_dir.file_name().and_then(|value| value.to_str()),
            Some(".z")
        );
    }

    #[test]
    fn persist_screen_frame_artifact_maps_metadata_for_dedupable_ocr() {
        run_async_test(async {
            let dir = TestDir::new("screen-frame-artifact");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = persist_screen_frame_artifact(
                &infra,
                "session-artifact",
                ScreenFrameArtifact {
                    file_path: "/tmp/frame-artifact.png".to_string(),
                    captured_at_unix_ms: 1_744_539_600_123,
                    width: Some(1440),
                    height: Some(900),
                    content_fingerprint: Some(0xfeed_beef),
                },
            )
            .await
            .expect("artifact should persist");

            assert_eq!(persisted.frame.session_id, "session-artifact");
            assert_eq!(persisted.frame.file_path, "/tmp/frame-artifact.png");
            assert_eq!(persisted.frame.width, Some(1440));
            assert_eq!(persisted.frame.height, Some(900));
            assert_eq!(
                persisted.frame.content_fingerprint.as_deref(),
                Some("00000000feedbeef")
            );
            assert_eq!(
                persisted.job.as_ref().map(|job| job.processor.as_str()),
                Some("ocr")
            );

            let batches = infra
                .list_frame_batches(Some("session-artifact"))
                .await
                .expect("frame batches should list");
            assert_eq!(batches.len(), 1);
            assert_eq!(batches[0].status, ::app_infra::FrameBatchStatus::Open);
            assert_eq!(batches[0].frame_count, 1);
        });
    }

    #[test]
    fn process_pending_jobs_once_claims_and_processes_queued_work() {
        run_async_test(async {
            let dir = TestDir::new("processing-worker-once");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_processing_job(
                    &::app_infra::NewFrame::new(
                        "session-worker",
                        "/tmp/frame-worker.png",
                        "2026-04-12T10:00:00Z",
                    ),
                    "missing-processor",
                    None,
                )
                .await
                .expect("frame and job should persist");

            let processed = process_pending_jobs_once(&infra)
                .await
                .expect("worker iteration should succeed");
            assert_eq!(processed, Some(()));

            let job = infra
                .get_processing_job(persisted.job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(job.status, ::app_infra::ProcessingJobStatus::Failed);
            assert_eq!(job.attempt_count, 1);
            assert_eq!(
                job.last_error.as_deref(),
                Some("processor backend is not registered for 'missing-processor'")
            );
        });
    }

    #[test]
    fn process_pending_jobs_once_can_process_frame_batch_finalize_jobs() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-worker-once");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            infra
                .insert_frame_into_batch_and_maybe_enqueue_ocr_job(
                    &::app_infra::NewFrame::new(
                        "session-batch-worker",
                        "/tmp/session-batch-worker-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            infra
                .insert_frame_into_batch_and_maybe_enqueue_ocr_job(
                    &::app_infra::NewFrame::new(
                        "session-batch-worker",
                        "/tmp/session-batch-worker-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            let processed = process_pending_jobs_once(&infra)
                .await
                .expect("worker iteration should succeed");
            assert_eq!(processed, Some(()));

            let batches = infra
                .list_frame_batches(Some("session-batch-worker"))
                .await
                .expect("frame batches should list");

            // The first batch's OCR job is processed (fails: no backend) in the
            // same iteration, making the finalize job claimable.  Finalization
            // completes successfully (frame cleanup skips missing files), so the
            // batch ends up Completed — proving the worker now services finalize
            // jobs alongside processing jobs instead of starving them.
            assert!(batches
                .iter()
                .any(|batch| batch.status == ::app_infra::FrameBatchStatus::Completed));
        });
    }
}
