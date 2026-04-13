mod db;
pub mod error;
mod frame_batches;
pub mod jobs;
pub mod processing;
pub mod status;

use std::path::Path;

use sqlx::SqlitePool;

pub use error::{AppInfraError, Result};
pub use frame_batches::{
    FrameBatch, FrameBatchFinalizePayload, FrameBatchFinalizeResult, FrameBatchRuntime,
    FrameBatchStatus, FrameBatchStore, FrameBatchWindow,
    FRAME_BATCH_FINALIZE_JOB_KIND, FRAME_BATCH_DURATION_MINUTES,
};
pub use jobs::{
    default_worker_thread_count, BackgroundJob, BackgroundJobStatus, CpuJobHandle, CpuJobResult,
    CpuJobSuccess, DebugCpuJobRequest, JobCounts, JobDescriptor, JobRuntime, JobStore,
};
pub use processing::{
    AppleVisionOcrEngine, Frame, FrameOcrEnqueueResult, FramePipeline, FramePipelineRequest,
    FrameProcessingJob, NewFrame, OcrEngine, OcrOutput, OcrProcessorBackend, OcrProvider,
    OcrRequest, ProcessingJob, ProcessingJobCompletion, ProcessingJobDraft,
    ProcessingJobRunOutcome, ProcessingJobStatus, ProcessingResult, ProcessingResultDraft,
    ProcessingRuntime, ProcessingStore, ProcessingSubject, ProcessorBackend, ProcessorRegistry,
    FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};
pub use status::AppInfraStatus;

#[derive(Clone)]
pub struct AppInfra {
    database: db::Database,
    jobs: JobStore,
    frame_batches: FrameBatchStore,
    processing: ProcessingStore,
    frame_pipeline: FramePipeline,
    runtime: JobRuntime,
    frame_batch_runtime: FrameBatchRuntime,
    processing_runtime: ProcessingRuntime,
}

impl AppInfra {
    pub async fn initialize<P: AsRef<Path>>(base_dir: P) -> Result<Self> {
        Self::initialize_with_processing_registry(base_dir, default_processing_registry()).await
    }

    pub async fn initialize_with_processing_registry<P: AsRef<Path>>(
        base_dir: P,
        processing_registry: ProcessorRegistry,
    ) -> Result<Self> {
        let database = db::Database::initialize(base_dir.as_ref()).await?;
        let jobs = JobStore::new(database.pool().clone());
        let frame_batches = FrameBatchStore::new(database.pool().clone());
        let processing = ProcessingStore::new(database.pool().clone());
        let frame_pipeline = FramePipeline::new(processing.clone());
        jobs.reconcile_orphaned_running_jobs().await?;
        processing.reconcile_orphaned_running_jobs().await?;
        frame_batches
            .reconcile_closed_batches_without_finalize_jobs()
            .await?;
        let runtime = JobRuntime::new(default_worker_thread_count())?;
        let frame_batch_runtime = FrameBatchRuntime::new(frame_batches.clone());
        let processing_runtime = ProcessingRuntime::new(processing.clone(), processing_registry);

        Ok(Self {
            database,
            jobs,
            frame_batches,
            processing,
            frame_pipeline,
            runtime,
            frame_batch_runtime,
            processing_runtime,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    pub fn database_path(&self) -> &Path {
        self.database.database_path()
    }

    pub fn jobs(&self) -> &JobStore {
        &self.jobs
    }

    pub fn processing(&self) -> &ProcessingStore {
        &self.processing
    }

    pub fn frame_batches(&self) -> &FrameBatchStore {
        &self.frame_batches
    }

    pub fn frame_pipeline(&self) -> &FramePipeline {
        &self.frame_pipeline
    }

    pub fn runtime(&self) -> &JobRuntime {
        &self.runtime
    }

    pub fn processing_runtime(&self) -> &ProcessingRuntime {
        &self.processing_runtime
    }

    pub fn frame_batch_runtime(&self) -> &FrameBatchRuntime {
        &self.frame_batch_runtime
    }

    pub async fn enqueue_job(
        &self,
        descriptor: &JobDescriptor,
        payload_json: Option<&str>,
    ) -> Result<BackgroundJob> {
        self.jobs.enqueue(descriptor, payload_json).await
    }

    pub async fn list_jobs(&self) -> Result<Vec<BackgroundJob>> {
        self.jobs.list(None).await
    }

    pub async fn get_job(&self, job_id: i64) -> Result<Option<BackgroundJob>> {
        self.jobs.get(job_id).await
    }

    pub async fn insert_frame(&self, frame: &NewFrame) -> Result<Frame> {
        self.processing.insert_frame(frame).await
    }

    pub async fn insert_frame_and_enqueue_processing_job(
        &self,
        frame: &NewFrame,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        let mut request = FramePipelineRequest::new(frame.clone(), processor);

        if let Some(payload_json) = payload_json {
            request = request.with_payload_json(payload_json);
        }

        self.frame_pipeline.enqueue(&request).await
    }

    pub async fn insert_frame_and_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        let mut request = FramePipelineRequest::for_ocr(frame.clone());

        if let Some(payload_json) = payload_json {
            request = request.with_payload_json(payload_json);
        }

        self.frame_pipeline.enqueue(&request).await
    }

    pub async fn insert_frame_and_maybe_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameOcrEnqueueResult> {
        self.processing
            .insert_frame_and_maybe_enqueue_ocr_job(frame, payload_json)
            .await
    }

    pub async fn insert_frame_into_batch_and_maybe_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameOcrEnqueueResult> {
        let mut transaction = self.pool().begin().await?;

        let batch = self
            .frame_batches
            .upsert_open_batch_for_frame_in_transaction(
                &mut transaction,
                &frame.session_id,
                &frame.captured_at,
            )
            .await?;
        let persisted = self
            .processing
            .insert_frame_and_maybe_enqueue_ocr_job_in_transaction(
                &mut transaction,
                frame,
                payload_json,
            )
            .await?;
        self.frame_batches
            .attach_frame_to_batch_in_transaction(
                &mut transaction,
                persisted.frame.id,
                batch.id,
                &persisted.frame.captured_at,
            )
            .await?;
        transaction.commit().await?;

        let _ = self
            .frame_batches
            .close_and_schedule_completed_batches_for_frame(&frame.session_id, batch.id)
            .await?;

        Ok(persisted)
    }

    pub async fn list_frames(&self, session_id: Option<&str>) -> Result<Vec<Frame>> {
        self.processing.list_frames(session_id).await
    }

    pub async fn list_frame_batches(&self, session_id: Option<&str>) -> Result<Vec<FrameBatch>> {
        self.frame_batches.list_batches(session_id).await
    }

    pub async fn get_frame_batch(&self, batch_id: i64) -> Result<Option<FrameBatch>> {
        self.frame_batches.get(batch_id).await
    }

    pub async fn list_frames_for_batch(&self, batch_id: i64) -> Result<Vec<Frame>> {
        self.frame_batches.list_frames_for_batch(batch_id).await
    }

    pub async fn get_frame(&self, frame_id: i64) -> Result<Option<Frame>> {
        self.processing.get_frame(frame_id).await
    }

    pub async fn enqueue_processing_job(
        &self,
        draft: &ProcessingJobDraft,
    ) -> Result<ProcessingJob> {
        self.processing.enqueue_job(draft).await
    }

    pub async fn get_processing_job(&self, job_id: i64) -> Result<Option<ProcessingJob>> {
        self.processing.get_job(job_id).await
    }

    pub async fn list_processing_jobs_for_subject(
        &self,
        subject: &ProcessingSubject,
    ) -> Result<Vec<ProcessingJob>> {
        self.processing.list_jobs_for_subject(subject).await
    }

    pub async fn claim_queued_processing_job(&self, job_id: i64) -> Result<Option<ProcessingJob>> {
        self.processing.claim_queued_job(job_id).await
    }

    pub async fn mark_processing_job_running(&self, job_id: i64) -> Result<ProcessingJob> {
        self.processing.mark_job_running(job_id).await
    }

    pub async fn mark_processing_job_failed(
        &self,
        job_id: i64,
        error_text: Option<&str>,
    ) -> Result<ProcessingJob> {
        self.processing.mark_job_failed(job_id, error_text).await
    }

    pub async fn complete_processing_job(
        &self,
        job_id: i64,
        result: &ProcessingResultDraft,
    ) -> Result<ProcessingJobCompletion> {
        self.processing.complete_job(job_id, result).await
    }

    pub async fn get_processing_result_for_job(
        &self,
        job_id: i64,
    ) -> Result<Option<ProcessingResult>> {
        self.processing.get_result_for_job(job_id).await
    }

    pub async fn list_processing_results_for_subject(
        &self,
        subject: &ProcessingSubject,
    ) -> Result<Vec<ProcessingResult>> {
        self.processing.list_results_for_subject(subject).await
    }

    pub async fn process_processing_job(&self, job_id: i64) -> Result<ProcessingJobRunOutcome> {
        self.processing_runtime.process_job(job_id).await
    }

    pub async fn process_next_processing_job(&self) -> Result<Option<ProcessingJobRunOutcome>> {
        self.processing_runtime.process_next_queued_job().await
    }

    pub async fn process_next_frame_batch_job(&self) -> Result<Option<FrameBatchFinalizeResult>> {
        self.frame_batch_runtime.process_next_queued_job().await
    }

    pub async fn submit_debug_cpu_job(&self, request: DebugCpuJobRequest) -> Result<BackgroundJob> {
        let request = request.normalized();
        let payload_json = serde_json::to_string(&request)?;
        let task_request = request.clone();
        let handle = self
            .spawn_cpu_job(
                JobDescriptor::new(jobs::DEBUG_CPU_JOB_KIND),
                Some(&payload_json),
                move || {
                    let result_text = task_request.simulated_result_text();
                    Ok(CpuJobSuccess::new(result_text.clone()).with_result_text(result_text))
                },
            )
            .await?;

        self.get_job(handle.job_id())
            .await?
            .ok_or(AppInfraError::JobNotFound(handle.job_id()))
    }

    pub async fn spawn_cpu_job<F, T>(
        &self,
        descriptor: JobDescriptor,
        payload_json: Option<&str>,
        task: F,
    ) -> Result<CpuJobHandle<T>>
    where
        F: FnOnce() -> CpuJobResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let job = self.jobs.enqueue(&descriptor, payload_json).await?;
        self.runtime.spawn_cpu(self.jobs.clone(), job, task)
    }

    pub async fn status(&self) -> Result<AppInfraStatus> {
        Ok(AppInfraStatus {
            database_path: self.database.database_path().display().to_string(),
            migrations_ran: self.database.migrations_ran(),
            worker_thread_count: self.runtime.worker_thread_count(),
            job_counts: self.jobs.counts().await?,
        })
    }
}

fn default_processing_registry() -> ProcessorRegistry {
    ProcessorRegistry::new().register(OcrProcessorBackend::new(AppleVisionOcrEngine::new()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::mpsc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{db::Database, jobs::ORPHANED_RUNNING_JOB_ERROR};

    const TEST_PROCESSOR: &str = "mock-recovery";
    const ORPHANED_RUNNING_PROCESSING_JOB_MESSAGE: &str =
        "processing job was marked failed during startup recovery after the app shut down while it was running";

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("app-infra-{label}-{unique}"));

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
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    fn build_test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
    }

    fn test_frame(session_id: &str, file_name: &str) -> NewFrame {
        NewFrame::new(
            session_id,
            format!("/tmp/{file_name}"),
            "2026-04-12T10:00:00Z",
        )
        .with_dimensions(1920, 1080)
    }

    fn test_frame_with_fingerprint(
        session_id: &str,
        file_name: &str,
        content_fingerprint: &str,
    ) -> NewFrame {
        test_frame(session_id, file_name).with_content_fingerprint(content_fingerprint)
    }

    #[derive(Debug)]
    struct SuccessfulProcessingBackend {
        processor: &'static str,
        result: ProcessingResultDraft,
    }

    impl SuccessfulProcessingBackend {
        fn new(processor: &'static str, result_text: &str) -> Self {
            Self {
                processor,
                result: ProcessingResultDraft::new().with_result_text(result_text),
            }
        }
    }

    #[async_trait]
    impl ProcessorBackend for SuccessfulProcessingBackend {
        fn processor(&self) -> &'static str {
            self.processor
        }

        async fn process(
            &self,
            _store: &ProcessingStore,
            _job: &ProcessingJob,
        ) -> Result<ProcessingResultDraft> {
            Ok(self.result.clone())
        }
    }

    fn test_processing_registry(result_text: &str) -> ProcessorRegistry {
        ProcessorRegistry::new().register(SuccessfulProcessingBackend::new(
            TEST_PROCESSOR,
            result_text,
        ))
    }

    #[test]
    fn database_reports_when_embedded_migrations_ran() {
        run_async_test(async {
            let dir = TestDir::new("migrations-ran");

            let first = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            assert!(first.migrations_ran());

            drop(first);

            let second = Database::initialize(dir.path())
                .await
                .expect("database should re-initialize");
            assert!(!second.migrations_ran());
        });
    }

    #[test]
    fn cpu_jobs_persist_running_and_completed_transitions() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-success");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();

            let handle = infra
                .spawn_cpu_job(
                    JobDescriptor::new("ocr"),
                    Some("{\"documentId\":1}"),
                    move || {
                        started_tx
                            .send(())
                            .expect("job should notify when it starts");
                        release_rx
                            .recv()
                            .expect("job should wait until the test releases it");

                        Ok(CpuJobSuccess::new("finished".to_string())
                            .with_result_text("recognized text"))
                    },
                )
                .await
                .expect("cpu job should spawn");

            started_rx.recv().expect("job should reach the worker pool");

            let running = infra
                .jobs()
                .get(handle.job_id())
                .await
                .expect("running job should be readable")
                .expect("running job should exist");
            assert_eq!(running.status, BackgroundJobStatus::Running);
            assert_eq!(running.attempt_count, 1);
            assert!(running.started_at.is_some());
            assert!(running.finished_at.is_none());

            release_tx.send(()).expect("test should release the job");

            let outcome = handle.join().await.expect("job join should succeed");
            assert_eq!(
                outcome,
                Ok(CpuJobSuccess::new("finished".to_string()).with_result_text("recognized text"))
            );

            let completed = infra
                .jobs()
                .get(running.id)
                .await
                .expect("completed job should be readable")
                .expect("completed job should exist");
            assert_eq!(completed.status, BackgroundJobStatus::Completed);
            assert_eq!(completed.result_text.as_deref(), Some("recognized text"));
            assert!(completed.finished_at.is_some());
            assert_eq!(completed.last_error, None);
        });
    }

    #[test]
    fn enqueued_jobs_are_persisted_as_queued() {
        run_async_test(async {
            let dir = TestDir::new("queued-job");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let queued = infra
                .enqueue_job(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                .await
                .expect("job should enqueue");

            assert_eq!(queued.status, BackgroundJobStatus::Queued);
            assert_eq!(queued.payload_json.as_deref(), Some("{\"documentId\":1}"));
            assert_eq!(queued.attempt_count, 0);
            assert!(queued.started_at.is_none());
            assert!(queued.finished_at.is_none());
        });
    }

    #[test]
    fn cpu_jobs_persist_failed_transitions() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-failure");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let handle: CpuJobHandle<String> = infra
                .spawn_cpu_job(JobDescriptor::new("transcription"), None, || {
                    Err("transcription failed".to_string())
                })
                .await
                .expect("cpu job should spawn");

            let job_id = handle.job_id();
            let outcome = handle.join().await.expect("job join should complete");
            assert_eq!(outcome, Err("transcription failed".to_string()));

            let failed = infra
                .jobs()
                .get(job_id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(failed.last_error.as_deref(), Some("transcription failed"));
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert_eq!(failed.result_text, None);
        });
    }

    #[test]
    fn cpu_job_panics_are_persisted_as_failed() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-panic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let handle: CpuJobHandle<String> = infra
                .spawn_cpu_job(JobDescriptor::new("transcription"), None, || {
                    panic!("worker panic");
                })
                .await
                .expect("cpu job should spawn");

            let job_id = handle.job_id();
            let outcome = handle.join().await.expect("job join should complete");
            assert_eq!(outcome, Err("cpu job panicked: worker panic".to_string()));

            let failed = infra
                .jobs()
                .get(job_id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("cpu job panicked: worker panic")
            );
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert_eq!(failed.result_text, None);
        });
    }

    #[test]
    fn startup_reconciles_orphaned_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("orphaned-running-job");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let queued = infra
                .enqueue_job(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                .await
                .expect("job should enqueue");

            let running = infra
                .jobs()
                .mark_running(queued.id)
                .await
                .expect("job should be marked running");
            assert_eq!(running.status, BackgroundJobStatus::Running);

            drop(infra);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let failed = recovered
                .jobs()
                .get(queued.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some(ORPHANED_RUNNING_JOB_ERROR)
            );
            assert!(failed.finished_at.is_some());
        });
    }

    #[test]
    fn queued_processing_jobs_persist_across_restart_and_stay_processable() {
        run_async_test(async {
            let dir = TestDir::new("processing-queued-restart");
            let initial = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("recovered text"),
            )
            .await
            .expect("app infra should initialize");

            let persisted = initial
                .insert_frame_and_enqueue_processing_job(
                    &test_frame("session-processing-restart", "frame-processing-restart.png"),
                    TEST_PROCESSOR,
                    Some("{\"mode\":\"queued\"}"),
                )
                .await
                .expect("frame and job should persist");

            drop(initial);

            let recovered = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("recovered text"),
            )
            .await
            .expect("app infra should re-initialize");

            let queued = recovered
                .get_processing_job(persisted.job.id)
                .await
                .expect("queued job should be readable")
                .expect("queued job should exist");
            assert_eq!(queued.status, ProcessingJobStatus::Queued);
            assert_eq!(queued.attempt_count, 0);
            assert_eq!(
                queued.payload_json.as_deref(),
                Some("{\"mode\":\"queued\"}")
            );

            let outcome = recovered
                .process_next_processing_job()
                .await
                .expect("queued job should process after restart")
                .expect("queued job should exist");

            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected completed outcome");
            };

            assert_eq!(completion.job.id, persisted.job.id);
            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);
            assert_eq!(completion.job.attempt_count, 1);
            assert_eq!(
                completion.result.result_text.as_deref(),
                Some("recovered text")
            );

            let stored_result = recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("processing result should be readable")
                .expect("processing result should exist");
            assert_eq!(stored_result, completion.result);
        });
    }

    #[test]
    fn startup_reconciles_orphaned_running_processing_jobs() {
        run_async_test(async {
            let dir = TestDir::new("processing-running-restart");
            let initial = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("unused result"),
            )
            .await
            .expect("app infra should initialize");

            let persisted = initial
                .insert_frame_and_enqueue_processing_job(
                    &test_frame("session-processing-running", "frame-processing-running.png"),
                    TEST_PROCESSOR,
                    None,
                )
                .await
                .expect("frame and job should persist");

            let running = initial
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job claim should succeed")
                .expect("job should claim");
            assert_eq!(running.status, ProcessingJobStatus::Running);
            assert_eq!(running.attempt_count, 1);

            drop(initial);

            let recovered = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("unused result"),
            )
            .await
            .expect("app infra should re-initialize");

            let failed = recovered
                .get_processing_job(persisted.job.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(failed.status, ProcessingJobStatus::Failed);
            assert_eq!(failed.attempt_count, 1);
            assert_eq!(
                failed.last_error.as_deref(),
                Some(ORPHANED_RUNNING_PROCESSING_JOB_MESSAGE)
            );
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert!(recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn processing_results_stay_clean_when_retrying_after_restart_recovery() {
        run_async_test(async {
            let dir = TestDir::new("processing-retry-recovery");
            let initial = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("first pass"),
            )
            .await
            .expect("app infra should initialize");

            let persisted = initial
                .insert_frame_and_enqueue_processing_job(
                    &test_frame("session-processing-retry", "frame-processing-retry.png"),
                    TEST_PROCESSOR,
                    None,
                )
                .await
                .expect("frame and job should persist");

            let first_outcome = initial
                .process_processing_job(persisted.job.id)
                .await
                .expect("initial processing should succeed");
            let ProcessingJobRunOutcome::Completed(first_completion) = first_outcome else {
                panic!("expected completed outcome");
            };
            assert_eq!(
                first_completion.result.result_text.as_deref(),
                Some("first pass")
            );

            let rerunning = initial
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect("completed job should restart");
            assert_eq!(rerunning.status, ProcessingJobStatus::Running);
            assert_eq!(rerunning.attempt_count, 2);
            assert!(initial
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("stale result lookup should succeed")
                .is_none());

            drop(initial);

            let recovered = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("second pass"),
            )
            .await
            .expect("app infra should re-initialize");

            let failed = recovered
                .get_processing_job(persisted.job.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(failed.status, ProcessingJobStatus::Failed);
            assert_eq!(failed.attempt_count, 2);
            assert_eq!(
                failed.last_error.as_deref(),
                Some(ORPHANED_RUNNING_PROCESSING_JOB_MESSAGE)
            );
            assert!(recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("recovered result lookup should succeed")
                .is_none());

            let retried = recovered
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect("recovered failed job should restart");
            assert_eq!(retried.status, ProcessingJobStatus::Running);
            assert_eq!(retried.attempt_count, 3);

            let retried_outcome = recovered
                .process_processing_job(persisted.job.id)
                .await
                .expect("retried processing should succeed");
            let ProcessingJobRunOutcome::Completed(retried_completion) = retried_outcome else {
                panic!("expected completed outcome");
            };

            assert_eq!(
                retried_completion.result.result_text.as_deref(),
                Some("second pass")
            );
            assert_ne!(retried_completion.result.id, first_completion.result.id);

            let stored_result = recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("final result lookup should succeed")
                .expect("final result should exist");
            assert_eq!(stored_result, retried_completion.result);

            let subject_results = recovered
                .list_processing_results_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject results should list");
            assert_eq!(subject_results, vec![retried_completion.result]);
        });
    }

    #[test]
    fn spawn_setup_failures_mark_jobs_failed() {
        let dir = TestDir::new("spawn-setup-failure");

        let (jobs, job, runtime) = {
            let setup_runtime = build_test_runtime();
            setup_runtime.block_on(async {
                let database = Database::initialize(dir.path())
                    .await
                    .expect("database should initialize");
                let jobs = JobStore::new(database.pool().clone());
                let job = jobs
                    .enqueue(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                    .await
                    .expect("job should enqueue");
                let runtime = JobRuntime::new(1).expect("job runtime should initialize");

                (jobs, job, runtime)
            })
        };

        let error = runtime
            .spawn_cpu(jobs.clone(), job.clone(), || {
                Ok(CpuJobSuccess::new("done".to_string()))
            })
            .err()
            .expect("spawning without a tokio runtime should fail");
        assert!(matches!(error, AppInfraError::AsyncRuntimeUnavailable));

        let verify_runtime = build_test_runtime();
        verify_runtime.block_on(async {
            let failed = jobs
                .get(job.id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("background jobs require an active Tokio runtime")
            );
            assert!(failed.started_at.is_none());
            assert!(failed.finished_at.is_some());
        });
    }

    #[test]
    fn frames_are_persisted_and_listable() {
        run_async_test(async {
            let dir = TestDir::new("processing-frames");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-a.png"))
                .await
                .expect("frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-b", "frame-b.png"))
                .await
                .expect("second frame should persist");

            let fetched = infra
                .get_frame(first.id)
                .await
                .expect("frame should be readable")
                .expect("frame should exist");
            assert_eq!(fetched, first);

            let session_a_frames = infra
                .list_frames(Some("session-a"))
                .await
                .expect("session frames should list");
            assert_eq!(session_a_frames, vec![first.clone()]);

            let all_frames = infra.list_frames(None).await.expect("frames should list");
            assert_eq!(all_frames.len(), 2);
            assert_eq!(all_frames[0].id, second.id);
            assert_eq!(all_frames[1].id, first.id);
        });
    }

    #[test]
    fn frame_pipeline_enqueues_frame_and_processing_job() {
        run_async_test(async {
            let dir = TestDir::new("frame-pipeline");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .frame_pipeline()
                .enqueue(
                    &FramePipelineRequest::for_ocr(test_frame(
                        "session-pipeline",
                        "frame-pipeline.png",
                    ))
                    .with_payload_json("{\"language\":\"eng\"}"),
                )
                .await
                .expect("frame pipeline should persist frame and job");

            assert_eq!(persisted.job.subject_type, FRAME_SUBJECT_TYPE);
            assert_eq!(persisted.job.subject_id, persisted.frame.id);
            assert_eq!(persisted.job.processor, OCR_PROCESSOR);
            assert_eq!(persisted.job.status, ProcessingJobStatus::Queued);
            assert_eq!(
                persisted.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );
        });
    }

    #[test]
    fn insert_frame_and_enqueue_ocr_job_persists_linked_subject() {
        run_async_test(async {
            let dir = TestDir::new("processing-enqueue");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-ocr", "frame-ocr.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("frame and job should persist");

            assert_eq!(persisted.job.subject_type, FRAME_SUBJECT_TYPE);
            assert_eq!(persisted.job.subject_id, persisted.frame.id);
            assert_eq!(persisted.job.processor, OCR_PROCESSOR);
            assert_eq!(persisted.job.status, ProcessingJobStatus::Queued);
            assert_eq!(
                persisted.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject jobs should list");
            assert_eq!(jobs, vec![persisted.job.clone()]);
        });
    }

    #[test]
    fn duplicate_fingerprinted_frames_skip_redundant_ocr_jobs() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame_with_fingerprint("session-dedupe", "frame-1.png", "abc123"),
                    None,
                )
                .await
                .expect("first frame should persist");
            let duplicate = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame_with_fingerprint("session-dedupe", "frame-2.png", "abc123"),
                    None,
                )
                .await
                .expect("duplicate frame should persist");
            let changed = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame_with_fingerprint("session-dedupe", "frame-3.png", "def456"),
                    None,
                )
                .await
                .expect("changed frame should persist");

            assert!(first.job.is_some());
            assert!(duplicate.job.is_none());
            assert!(changed.job.is_some());

            let frames = infra
                .list_frames(Some("session-dedupe"))
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[0].content_fingerprint.as_deref(), Some("def456"));
            assert_eq!(frames[1].content_fingerprint.as_deref(), Some("abc123"));
            assert_eq!(frames[2].content_fingerprint.as_deref(), Some("abc123"));

            let duplicate_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(duplicate.frame.id))
                .await
                .expect("duplicate jobs should list");
            assert!(duplicate_jobs.is_empty());
        });
    }

    #[test]
    fn non_consecutive_duplicate_fingerprinted_frames_skip_redundant_ocr_jobs() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-non-consecutive");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame_with_fingerprint("session-dedupe-repeat", "frame-1.png", "abc123"),
                    None,
                )
                .await
                .expect("first frame should persist");
            let changed = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame_with_fingerprint("session-dedupe-repeat", "frame-2.png", "def456"),
                    None,
                )
                .await
                .expect("changed frame should persist");
            let repeated = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame_with_fingerprint("session-dedupe-repeat", "frame-3.png", "abc123"),
                    None,
                )
                .await
                .expect("repeated frame should persist");

            assert!(first.job.is_some());
            assert!(changed.job.is_some());
            assert!(repeated.job.is_none());

            let frames = infra
                .list_frames(Some("session-dedupe-repeat"))
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[0].content_fingerprint.as_deref(), Some("abc123"));
            assert_eq!(frames[1].content_fingerprint.as_deref(), Some("def456"));
            assert_eq!(frames[2].content_fingerprint.as_deref(), Some("abc123"));

            let repeated_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(repeated.frame.id))
                .await
                .expect("repeated jobs should list");
            assert!(repeated_jobs.is_empty());
        });
    }

    #[test]
    fn startup_reconciles_closed_batches_missing_finalize_jobs() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-startup-reconcile");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let batch = initial
                .frame_batches()
                .upsert_open_batch_for_frame("session-batch-reconcile", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should persist");
            let persisted = initial
                .processing()
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &NewFrame::new(
                        "session-batch-reconcile",
                        "/tmp/session-batch-reconcile-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("frame and OCR state should persist");

            initial
                .frame_batches()
                .attach_frame_to_batch(persisted.frame.id, batch.id, &persisted.frame.captured_at)
                .await
                .expect("frame should attach");

            let closed = initial
                .frame_batches()
                .close_completed_batches_for_session("session-batch-reconcile", None)
                .await
                .expect("batch should close without scheduling");
            assert_eq!(closed.len(), 1);
            assert!(closed[0].finalize_job_id.is_none());

            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let reconciled = recovered
                .get_frame_batch(batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(reconciled.status, FrameBatchStatus::Closed);
            let finalize_job_id = reconciled
                .finalize_job_id
                .expect("startup should schedule finalize job");

            let finalize_job = recovered
                .get_job(finalize_job_id)
                .await
                .expect("finalize job should be readable")
                .expect("finalize job should exist");
            assert_eq!(finalize_job.kind, FRAME_BATCH_FINALIZE_JOB_KIND);
            assert_eq!(finalize_job.status, BackgroundJobStatus::Queued);
        });
    }

    #[test]
    fn batch_insert_rolls_back_frame_and_job_when_attachment_fails() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-atomic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let mut transaction = infra
                .pool()
                .begin()
                .await
                .expect("transaction should start");
            let batch = infra
                .frame_batches()
                .upsert_open_batch_for_frame_in_transaction(
                    &mut transaction,
                    "session-batch-atomic",
                    "2026-04-12T10:01:00Z",
                )
                .await
                .expect("batch should persist in transaction");
            let persisted = infra
                .processing()
                .insert_frame_and_maybe_enqueue_ocr_job_in_transaction(
                    &mut transaction,
                    &test_frame_with_fingerprint(
                        "session-batch-atomic",
                        "frame-atomic.png",
                        "atomic-fingerprint",
                    ),
                    None,
                )
                .await
                .expect("frame and OCR state should persist in transaction");

            let error = infra
                .frame_batches()
                .attach_frame_to_batch_in_transaction(
                    &mut transaction,
                    persisted.frame.id,
                    i64::MAX,
                    &persisted.frame.captured_at,
                )
                .await
                .expect_err("invalid batch attachment should fail");
            assert!(matches!(error, AppInfraError::Sqlx(_)));

            transaction
                .rollback()
                .await
                .expect("failed transaction should roll back");

            let frames = infra
                .list_frames(Some("session-batch-atomic"))
                .await
                .expect("frames should list");
            assert!(frames.is_empty());

            let batches = infra
                .list_frame_batches(Some("session-batch-atomic"))
                .await
                .expect("batches should list");
            assert!(batches.is_empty());

            let processing_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM processing_jobs WHERE subject_id = ?1")
                    .bind(persisted.frame.id)
                    .fetch_one(infra.pool())
                    .await
                    .expect("processing job count should query");
            assert_eq!(processing_job_count, 0);

            assert!(infra
                .get_frame_batch(batch.id)
                .await
                .expect("batch lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn batched_frame_insert_assigns_ten_minute_windows_and_schedules_closed_batch() {
        run_async_test(async {
            let dir = TestDir::new("frame-batches");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame_into_batch_and_maybe_enqueue_ocr_job(
                    &NewFrame::new(
                        "session-batches",
                        "/tmp/session-batches-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame_into_batch_and_maybe_enqueue_ocr_job(
                    &NewFrame::new(
                        "session-batches",
                        "/tmp/session-batches-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            let first_batches = infra
                .list_frame_batches(Some("session-batches"))
                .await
                .expect("frame batches should list");
            assert_eq!(first_batches.len(), 2);
            assert_eq!(first_batches[0].status, FrameBatchStatus::Open);
            assert_eq!(first_batches[0].frame_count, 1);
            assert_eq!(first_batches[1].status, FrameBatchStatus::Closed);
            assert_eq!(first_batches[1].frame_count, 1);
            assert!(first_batches[1].finalize_job_id.is_some());

            let first_batch_frames = infra
                .list_frames_for_batch(first_batches[1].id)
                .await
                .expect("batch frames should list");
            assert_eq!(first_batch_frames.len(), 1);
            assert_eq!(first_batch_frames[0].id, first.frame.id);

            let second_batch_frames = infra
                .list_frames_for_batch(first_batches[0].id)
                .await
                .expect("second batch frames should list");
            assert_eq!(second_batch_frames.len(), 1);
            assert_eq!(second_batch_frames[0].id, second.frame.id);
        });
    }

    #[test]
    fn frames_without_fingerprint_still_enqueue_ocr() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-no-fingerprint");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame("session-no-fingerprint", "frame-1.png"),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame_and_maybe_enqueue_ocr_job(
                    &test_frame("session-no-fingerprint", "frame-2.png"),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(second.job.is_some());
        });
    }

    #[test]
    fn processing_results_persist_separately_from_frames() {
        run_async_test(async {
            let dir = TestDir::new("processing-results");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-results", "frame-results.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let running = infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job should transition to running")
                .expect("job should claim successfully");
            assert_eq!(running.status, ProcessingJobStatus::Running);
            assert_eq!(running.attempt_count, 1);

            let completion = infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new()
                        .with_result_text("recognized text")
                        .with_structured_payload_json("{\"blocks\":[]}")
                        .with_processor_version("ocr-v1"),
                )
                .await
                .expect("job completion should persist result");

            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);
            assert_eq!(completion.result.job_id, persisted.job.id);
            assert_eq!(completion.result.subject_id, persisted.frame.id);
            assert_eq!(completion.result.processor, OCR_PROCESSOR);
            assert_eq!(
                completion.result.result_text.as_deref(),
                Some("recognized text")
            );

            let stored_result = infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("job result should be readable")
                .expect("job result should exist");
            assert_eq!(stored_result, completion.result);

            let subject_results = infra
                .list_processing_results_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject results should list");
            assert_eq!(subject_results, vec![completion.result.clone()]);

            let frame = infra
                .get_frame(persisted.frame.id)
                .await
                .expect("frame should still be readable")
                .expect("frame should exist");
            assert_eq!(frame.file_path, persisted.frame.file_path);
            assert_eq!(frame.width, Some(1920));
            assert_eq!(frame.height, Some(1080));
        });
    }

    #[test]
    fn processing_job_lifecycle_clears_stale_results_on_retry() {
        run_async_test(async {
            let dir = TestDir::new("processing-lifecycle");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-retry", "frame-retry.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("frame and job should persist");

            infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job should start")
                .expect("job should claim successfully");
            infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new().with_result_text("first pass"),
                )
                .await
                .expect("job should complete");
            assert!(infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());

            let retried = infra
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect("job should restart");
            assert_eq!(retried.status, ProcessingJobStatus::Running);
            assert_eq!(retried.attempt_count, 2);
            assert!(infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("retry result lookup should succeed")
                .is_none());

            let failed = infra
                .mark_processing_job_failed(persisted.job.id, Some("ocr retry failed"))
                .await
                .expect("job should fail");
            assert_eq!(failed.status, ProcessingJobStatus::Failed);
            assert_eq!(failed.last_error.as_deref(), Some("ocr retry failed"));
            assert!(infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("failed result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn processing_job_completion_requires_running_state() {
        run_async_test(async {
            let dir = TestDir::new("processing-complete-transition");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-complete", "frame-complete.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let error = infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new().with_result_text("recognized text"),
                )
                .await
                .expect_err("queued jobs should not complete directly");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == persisted.job.id && from == "queued" && to == "completed"
            ));
        });
    }

    #[test]
    fn processing_job_failure_requires_running_state() {
        run_async_test(async {
            let dir = TestDir::new("processing-fail-transition");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-fail", "frame-fail.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let error = infra
                .mark_processing_job_failed(persisted.job.id, Some("not running"))
                .await
                .expect_err("queued jobs should not fail directly");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == persisted.job.id && from == "queued" && to == "failed"
            ));
        });
    }

    #[test]
    fn processing_job_retry_requires_terminal_state() {
        run_async_test(async {
            let dir = TestDir::new("processing-retry-transition");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-running", "frame-running.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let claimed = infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job claim should succeed")
                .expect("job should claim");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);

            let error = infra
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect_err("running jobs should not restart");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == persisted.job.id && from == "running" && to == "running"
            ));
        });
    }
}
