mod audio_segments;
mod captured_frame_equivalence;
mod captured_frame_pipeline;
mod db;
pub mod error;
mod frame_batch_artifact_cleanup;
mod frame_batch_runtime;
mod frame_batch_store;
mod hidden_segment_workspace;
pub mod jobs;
pub mod processing;
pub mod status;

use std::path::Path;

use sqlx::SqlitePool;

pub use audio_segments::{
    AudioSegment, AudioSegmentSourceKind, AudioSegmentStore, NewAudioSegment,
};
pub use captured_frame_equivalence::{
    CapturedFrameEquivalenceResolver, CapturedFrameEquivalenceScope,
};
pub use captured_frame_pipeline::{
    CapturedFramePipeline, CapturedFramePipelineResult, CapturedFrameReprocessingOutcome,
    CapturedFrameReprocessingResult, ClosedFrameBatchSummary,
};
pub use error::{AppInfraError, Result};
pub use frame_batch_store::{
    FrameBatch, FrameBatchFinalizePayload, FrameBatchFinalizeResult, FrameBatchStatus,
    FrameBatchStore, FrameBatchWindow, SegmentWorkspaceBatchReference,
    FRAME_BATCH_DURATION_MINUTES, FRAME_BATCH_FINALIZE_JOB_KIND,
};
pub use frame_batch_runtime::FrameBatchRuntime;
pub use hidden_segment_workspace::{
    HiddenSegmentWorkspacePaths, HiddenSegmentWorkspaceRepairResult,
    SegmentWorkspaceCleanupDebugInfo, SegmentWorkspaceCleanupDisposition,
};
pub use jobs::{
    default_worker_thread_count, BackgroundJob, BackgroundJobStatus, CpuJobHandle, CpuJobResult,
    CpuJobSuccess, DebugCpuJobRequest, JobCounts, JobDescriptor, JobRuntime, JobStore,
};
pub use processing::{
    AppleVisionOcrEngine, FocusedFrameWindow, Frame, FrameEquivalence, FrameEquivalenceStatus,
    FrameProcessingJob, FrameSummary, NewFrame, OcrEngine, OcrOutput, OcrProcessorBackend,
    OcrProvider, OcrRequest, ProcessingJob, ProcessingJobCompletion, ProcessingJobDraft,
    ProcessingJobRunOutcome, ProcessingJobStatus, ProcessingResult, ProcessingResultDraft,
    ProcessingRuntime, ProcessingStore, ProcessingSubject, ProcessorBackend, ProcessorRegistry,
    SegmentWorkspaceOcrReference, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};
pub use status::AppInfraStatus;

#[derive(Clone)]
pub struct AppInfra {
    database: db::Database,
    jobs: JobStore,
    audio_segments: AudioSegmentStore,
    frame_batches: FrameBatchStore,
    processing: ProcessingStore,
    captured_frame_equivalence: CapturedFrameEquivalenceResolver,
    captured_frame_pipeline: CapturedFramePipeline,
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
        let audio_segments = AudioSegmentStore::new(database.pool().clone());
        let frame_batches = FrameBatchStore::new(database.pool().clone());
        let processing = ProcessingStore::new(database.pool().clone());
        let captured_frame_equivalence =
            CapturedFrameEquivalenceResolver::new(processing.clone());
        let captured_frame_pipeline =
            CapturedFramePipeline::new(processing.clone(), frame_batches.clone());
        processing.backfill_frame_equivalence().await?;
        jobs.reconcile_orphaned_running_jobs().await?;
        processing.reconcile_orphaned_running_jobs().await?;
        frame_batches
            .reconcile_closed_batches_without_finalize_jobs()
            .await?;
        frame_batches
            .reconcile_open_batches_without_active_capture()
            .await?;
        let runtime = JobRuntime::new(default_worker_thread_count())?;
        let frame_batch_runtime = FrameBatchRuntime::new(frame_batches.clone());
        let processing_runtime = ProcessingRuntime::new(processing.clone(), processing_registry);

        Ok(Self {
            database,
            jobs,
            audio_segments,
            frame_batches,
            processing,
            captured_frame_equivalence,
            captured_frame_pipeline,
            runtime,
            frame_batch_runtime,
            processing_runtime,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    #[cfg(test)]
    pub(crate) fn jobs(&self) -> &JobStore {
        &self.jobs
    }

    #[cfg(test)]
    pub(crate) fn audio_segments(&self) -> &AudioSegmentStore {
        &self.audio_segments
    }

    #[cfg(test)]
    pub(crate) fn processing(&self) -> &ProcessingStore {
        &self.processing
    }

    #[cfg(test)]
    pub(crate) fn frame_batches(&self) -> &FrameBatchStore {
        &self.frame_batches
    }

    #[cfg(test)]
    pub(crate) fn captured_frame_pipeline(&self) -> &CapturedFramePipeline {
        &self.captured_frame_pipeline
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

    pub async fn debug_insert_frame_and_enqueue_processing_job(
        &self,
        frame: &NewFrame,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        self.captured_frame_pipeline
            .debug_insert_frame_and_enqueue_processor_job(frame, processor, payload_json)
            .await
    }

    pub async fn debug_insert_frame_and_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        self.debug_insert_frame_and_enqueue_processing_job(frame, OCR_PROCESSOR, payload_json)
            .await
    }

    pub async fn capture_frame(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<CapturedFramePipelineResult> {
        self.captured_frame_pipeline
            .capture_frame(frame, payload_json)
            .await
    }

    pub async fn reprocess_captured_frame_ocr(
        &self,
        frame_id: i64,
        payload_json: Option<&str>,
    ) -> Result<CapturedFrameReprocessingResult> {
        self.captured_frame_pipeline
            .reprocess_captured_frame_ocr(frame_id, payload_json)
            .await
    }

    pub async fn list_frames(
        &self,
        session_id: Option<&str>,
        before_id: Option<i64>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Frame>> {
        self.processing
            .list_frames(session_id, before_id, limit, offset)
            .await
    }

    pub async fn list_frame_summaries_in_range(
        &self,
        captured_at_start: &str,
        captured_at_end: &str,
    ) -> Result<Vec<FrameSummary>> {
        self.processing
            .list_frame_summaries_in_range(captured_at_start, captured_at_end)
            .await
    }

    pub async fn get_timeline_window_around_frame(
        &self,
        frame_id: i64,
        newer_limit: u32,
        older_limit: u32,
    ) -> Result<FocusedFrameWindow> {
        self.processing
            .get_timeline_window_around_frame(frame_id, newer_limit, older_limit)
            .await
    }

    pub async fn get_latest_frame_in_range(
        &self,
        captured_at_start: &str,
        captured_at_end: &str,
    ) -> Result<Option<Frame>> {
        self.processing
            .get_latest_frame_in_range(captured_at_start, captured_at_end)
            .await
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

    pub async fn close_and_schedule_all_frame_batches_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<FrameBatch>> {
        self.frame_batches
            .close_and_schedule_all_batches_for_session(session_id)
            .await
    }

    pub async fn get_frame(&self, frame_id: i64) -> Result<Option<Frame>> {
        self.processing.get_frame(frame_id).await
    }

    pub async fn get_nearest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        self.captured_frame_equivalence
            .get_frame_and_find_nearest_earlier_equivalent_frame_in_default_scope(frame_id)
            .await
    }

    pub async fn get_earliest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        self.captured_frame_equivalence
            .get_frame_and_find_earliest_earlier_equivalent_frame_in_default_scope(frame_id)
            .await
    }

    pub async fn list_frames_for_segment_workspace(
        &self,
        session_id: &str,
        workspace_prefix: &str,
    ) -> Result<Vec<Frame>> {
        self.processing
            .list_frames_for_segment_workspace(session_id, workspace_prefix)
            .await
    }

    pub async fn classify_hidden_segment_workspace(
        &self,
        workspace_dir: &Path,
    ) -> Result<Option<SegmentWorkspaceCleanupDebugInfo>> {
        self.frame_batches
            .classify_hidden_segment_workspace(workspace_dir)
            .await
    }

    pub async fn repair_hidden_segment_workspaces(
        &self,
        recordings_root: &Path,
    ) -> Result<HiddenSegmentWorkspaceRepairResult> {
        self.frame_batches
            .repair_hidden_segment_workspaces(recordings_root)
            .await
    }

    pub async fn upsert_audio_segment(&self, segment: &NewAudioSegment) -> Result<AudioSegment> {
        self.audio_segments.upsert(segment).await
    }

    pub async fn get_audio_segment(&self, audio_segment_id: i64) -> Result<Option<AudioSegment>> {
        self.audio_segments.get(audio_segment_id).await
    }

    pub async fn list_audio_segments_overlapping_range(
        &self,
        range_start: &str,
        range_end: &str,
        source_kind: Option<AudioSegmentSourceKind>,
        source_session_id: Option<&str>,
    ) -> Result<Vec<AudioSegment>> {
        self.audio_segments
            .list_overlapping_range(range_start, range_end, source_kind, source_session_id)
            .await
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

    fn test_frame_at(session_id: &str, file_name: &str, captured_at: &str) -> NewFrame {
        NewFrame::new(session_id, format!("/tmp/{file_name}"), captured_at)
            .with_dimensions(1920, 1080)
    }

    fn write_test_png_rgba(
        dir: &TestDir,
        file_name: &str,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> String {
        let path = dir.path().join(file_name);
        image::save_buffer(&path, pixels, width, height, image::ColorType::Rgba8)
            .expect("test png should be written");
        path.to_string_lossy().into_owned()
    }

    fn solid_rgba(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width as usize * height as usize) {
            pixels.extend_from_slice(&rgba);
        }
        pixels
    }

    fn set_pixel_rgba(pixels: &mut [u8], width: u32, x: u32, y: u32, rgba: [u8; 4]) {
        let offset = ((y * width + x) * 4) as usize;
        pixels[offset..offset + 4].copy_from_slice(&rgba);
    }

    fn test_frame_with_equivalent_image(
        dir: &TestDir,
        session_id: &str,
        file_name: &str,
        captured_at: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> NewFrame {
        let file_path = write_test_png_rgba(dir, file_name, width, height, pixels);
        let equivalence = match capture_screen::captured_frame_equivalence_from_image_path(
            Path::new(&file_path),
        ) {
            capture_screen::CapturedFrameEquivalenceOutcome::Ready(equivalence) => {
                FrameEquivalence::ready(equivalence.hint, equivalence.proof, equivalence.version)
            }
            capture_screen::CapturedFrameEquivalenceOutcome::Quarantined(error) => {
                panic!("test image equivalence should compute: {error}");
            }
        };

        NewFrame::new(session_id, file_path, captured_at)
            .with_dimensions(width as i64, height as i64)
            .with_equivalence(equivalence)
    }

    fn test_segment_frame_with_equivalent_image(
        dir: &TestDir,
        session_id: &str,
        segment_index: u64,
        file_name: &str,
        captured_at: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> NewFrame {
        let frames_dir = dir.path().join(format!(
            "2026/04/12/.{session_id}-segment-{segment_index:04}/frames"
        ));
        fs::create_dir_all(&frames_dir).expect("segment frames dir should exist");
        let relative_name = format!("2026/04/12/.{session_id}-segment-{segment_index:04}/frames/{file_name}");
        test_frame_with_equivalent_image(
            dir,
            session_id,
            &relative_name,
            captured_at,
            pixels,
            width,
            height,
        )
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
                .debug_insert_frame_and_enqueue_processing_job(
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
                .debug_insert_frame_and_enqueue_processing_job(
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
                .debug_insert_frame_and_enqueue_processing_job(
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
                .list_frames(Some("session-a"), None, None, None)
                .await
                .expect("session frames should list");
            assert_eq!(session_a_frames, vec![first.clone()]);

            let all_frames = infra
                .list_frames(None, None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(all_frames.len(), 2);
            assert_eq!(all_frames[0].id, second.id);
            assert_eq!(all_frames[1].id, first.id);
        });
    }

    #[test]
    fn audio_segments_upsert_is_idempotent_and_lists_overlapping_ranges() {
        run_async_test(async {
            let dir = TestDir::new("audio-segments");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                "/tmp/mic-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );

            let inserted = infra
                .upsert_audio_segment(&segment)
                .await
                .expect("audio segment should insert");
            let updated = infra
                .upsert_audio_segment(&segment)
                .await
                .expect("duplicate audio segment should upsert");

            assert_eq!(inserted.id, updated.id);

            let fetched = infra
                .get_audio_segment(inserted.id)
                .await
                .expect("audio segment should be readable")
                .expect("audio segment should exist");
            assert_eq!(fetched, updated);

            let missing = infra
                .get_audio_segment(inserted.id + 10_000)
                .await
                .expect("missing audio segment lookup should succeed");
            assert!(missing.is_none());

            infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::SystemAudio,
                    "system-session",
                    1,
                    "/tmp/system-1.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("system audio segment should insert");

            let overlapping = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:00:30Z",
                    "2026-04-12T10:01:30Z",
                    None,
                    None,
                )
                .await
                .expect("overlapping audio segments should list");
            assert_eq!(overlapping.len(), 2);
            assert_eq!(
                overlapping[0].source_kind,
                AudioSegmentSourceKind::Microphone
            );
            assert_eq!(
                overlapping[1].source_kind,
                AudioSegmentSourceKind::SystemAudio
            );

            let microphone_only = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:00:30Z",
                    "2026-04-12T10:01:30Z",
                    Some(AudioSegmentSourceKind::Microphone),
                    Some("mic-session"),
                )
                .await
                .expect("filtered audio segments should list");
            assert_eq!(microphone_only, vec![updated]);

            let touching_boundary = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:01:00Z",
                    None,
                    None,
                )
                .await
                .expect("boundary-touching audio segments should list");
            assert_eq!(touching_boundary.len(), 2);
            assert_eq!(
                touching_boundary[0].source_kind,
                AudioSegmentSourceKind::Microphone
            );
            assert_eq!(
                touching_boundary[1].source_kind,
                AudioSegmentSourceKind::SystemAudio
            );

            let outside = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:02:01Z",
                    "2026-04-12T10:03:00Z",
                    None,
                    None,
                )
                .await
                .expect("outside range should list");
            assert!(outside.is_empty());
        });
    }

    #[test]
    fn frames_can_be_listed_with_limit_and_offset() {
        run_async_test(async {
            let dir = TestDir::new("processing-frames-pagination");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-1.png"))
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-a", "frame-2.png"))
                .await
                .expect("second frame should persist");
            let third = infra
                .insert_frame(&test_frame("session-b", "frame-3.png"))
                .await
                .expect("third frame should persist");

            let limited = infra
                .list_frames(None, None, Some(2), None)
                .await
                .expect("limited frames should list");
            assert_eq!(
                limited.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![third.id, second.id]
            );

            let paged = infra
                .list_frames(None, None, Some(1), Some(1))
                .await
                .expect("paged frames should list");
            assert_eq!(
                paged.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![second.id]
            );

            let session_paged = infra
                .list_frames(Some("session-a"), None, Some(1), Some(1))
                .await
                .expect("session paged frames should list");
            assert_eq!(
                session_paged
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![first.id]
            );

            let zero_limit = infra
                .list_frames(None, None, Some(0), None)
                .await
                .expect("zero-limit frames should list");
            assert!(zero_limit.is_empty());
        });
    }

    #[test]
    fn frames_can_be_listed_with_stable_before_id_cursor() {
        run_async_test(async {
            let dir = TestDir::new("processing-frames-before-id-pagination");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-1.png"))
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-a", "frame-2.png"))
                .await
                .expect("second frame should persist");
            let third = infra
                .insert_frame(&test_frame("session-a", "frame-3.png"))
                .await
                .expect("third frame should persist");

            let first_page = infra
                .list_frames(Some("session-a"), None, Some(2), None)
                .await
                .expect("first page should list");
            assert_eq!(
                first_page.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![third.id, second.id]
            );

            let inserted_after_first_page = infra
                .insert_frame(&test_frame("session-a", "frame-4.png"))
                .await
                .expect("newest frame should persist");

            let second_page = infra
                .list_frames(Some("session-a"), Some(second.id), Some(2), None)
                .await
                .expect("cursor page should list");
            assert_eq!(
                second_page.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![first.id]
            );

            let offset_page = infra
                .list_frames(Some("session-a"), None, Some(2), Some(2))
                .await
                .expect("offset page should list");
            assert_eq!(
                offset_page.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![second.id, first.id]
            );

            assert!(offset_page.iter().any(|frame| frame.id == second.id));
            assert!(first_page
                .iter()
                .all(|frame| frame.id != inserted_after_first_page.id));
        });
    }

    #[test]
    fn frame_summaries_in_range_are_filtered_and_sorted_newest_first() {
        run_async_test(async {
            let dir = TestDir::new("processing-frame-summaries-range");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-before.png",
                    "2026-04-11T23:59:59Z",
                ))
                .await
                .expect("earlier frame should persist");
            let start = infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-start.png",
                    "2026-04-12T00:00:00Z",
                ))
                .await
                .expect("start frame should persist");
            let middle = infra
                .insert_frame(&test_frame_at(
                    "session-b",
                    "frame-middle.png",
                    "2026-04-12T12:30:00Z",
                ))
                .await
                .expect("middle frame should persist");
            let end_first = infra
                .insert_frame(&test_frame_at(
                    "session-c",
                    "frame-end-first.png",
                    "2026-04-12T23:59:59Z",
                ))
                .await
                .expect("end frame should persist");
            let end_second = infra
                .insert_frame(&test_frame_at(
                    "session-d",
                    "frame-end-second.png",
                    "2026-04-12T23:59:59Z",
                ))
                .await
                .expect("second end frame should persist");
            infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-after.png",
                    "2026-04-13T00:00:00Z",
                ))
                .await
                .expect("later frame should persist");

            let summaries = infra
                .list_frame_summaries_in_range("2026-04-12T00:00:00Z", "2026-04-12T23:59:59Z")
                .await
                .expect("frame summaries should list");

            assert_eq!(
                summaries.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![end_second.id, end_first.id, middle.id, start.id]
            );
            assert_eq!(summaries[0].captured_at, "2026-04-12T23:59:59Z");
            assert_eq!(summaries[1].captured_at, "2026-04-12T23:59:59Z");
        });
    }

    #[test]
    fn latest_frame_in_range_returns_newest_match_or_none() {
        run_async_test(async {
            let dir = TestDir::new("processing-latest-frame-range");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let earliest = infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-early.png",
                    "2026-04-12T08:00:00Z",
                ))
                .await
                .expect("early frame should persist");
            let tied_first = infra
                .insert_frame(&test_frame_at(
                    "session-b",
                    "frame-tied-first.png",
                    "2026-04-12T09:30:00Z",
                ))
                .await
                .expect("first tied frame should persist");
            let tied_second = infra
                .insert_frame(&test_frame_at(
                    "session-c",
                    "frame-tied-second.png",
                    "2026-04-12T09:30:00Z",
                ))
                .await
                .expect("second tied frame should persist");

            let latest = infra
                .get_latest_frame_in_range("2026-04-12T08:30:00Z", "2026-04-12T09:30:00Z")
                .await
                .expect("latest frame should resolve")
                .expect("latest frame should exist");

            assert_eq!(latest.id, tied_second.id);
            assert_eq!(latest.captured_at, tied_first.captured_at);

            let missing = infra
                .get_latest_frame_in_range("2026-04-12T07:00:00Z", "2026-04-12T07:59:59Z")
                .await
                .expect("empty latest frame lookup should succeed");

            assert!(missing.is_none());
            assert!(latest.id > earliest.id);
        });
    }

    #[test]
    fn timeline_window_around_frame_is_newest_first_and_reports_older_history() {
        run_async_test(async {
            let dir = TestDir::new("processing-timeline-window");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-1.png"))
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-a", "frame-2.png"))
                .await
                .expect("second frame should persist");
            let third = infra
                .insert_frame(&test_frame("session-a", "frame-3.png"))
                .await
                .expect("third frame should persist");
            let fourth = infra
                .insert_frame(&test_frame("session-a", "frame-4.png"))
                .await
                .expect("fourth frame should persist");
            let fifth = infra
                .insert_frame(&test_frame("session-a", "frame-5.png"))
                .await
                .expect("fifth frame should persist");

            let window = infra
                .get_timeline_window_around_frame(third.id, 1, 1)
                .await
                .expect("timeline window should resolve");

            assert_eq!(
                window
                    .frames
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![fourth.id, third.id, second.id]
            );
            assert_eq!(window.target_index, 1);
            assert!(window.has_newer);
            assert!(window.has_older);

            let oldest_window = infra
                .get_timeline_window_around_frame(first.id, 2, 2)
                .await
                .expect("oldest timeline window should resolve");

            assert_eq!(
                oldest_window
                    .frames
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![third.id, second.id, first.id]
            );
            assert_eq!(oldest_window.target_index, 2);
            assert!(oldest_window.has_newer);
            assert!(!oldest_window.has_older);

            let newest_window = infra
                .get_timeline_window_around_frame(fifth.id, 2, 1)
                .await
                .expect("newest timeline window should resolve");

            assert_eq!(
                newest_window
                    .frames
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![fifth.id, fourth.id]
            );
            assert_eq!(newest_window.target_index, 0);
            assert!(!newest_window.has_newer);
            assert!(newest_window.has_older);
        });
    }

    #[test]
    fn list_frames_for_segment_workspace_escapes_like_wildcards() {
        run_async_test(async {
            let dir = TestDir::new("segment-workspace-like-escape");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let matching = infra
                .insert_frame(
                    &NewFrame::new(
                        "session-a",
                        "/tmp/workspaces/segment_%/frame-1.png",
                        "2026-04-12T10:00:00Z",
                    )
                    .with_dimensions(1920, 1080),
                )
                .await
                .expect("matching frame should persist");

            infra
                .insert_frame(
                    &NewFrame::new(
                        "session-a",
                        "/tmp/workspaces/segment-xx/frame-2.png",
                        "2026-04-12T10:00:01Z",
                    )
                    .with_dimensions(1920, 1080),
                )
                .await
                .expect("wildcard frame should persist");

            infra
                .insert_frame(
                    &NewFrame::new(
                        "session-a",
                        "/tmp/workspaces/segment_%extra/frame-3.png",
                        "2026-04-12T10:00:02Z",
                    )
                    .with_dimensions(1920, 1080),
                )
                .await
                .expect("prefix frame should persist");

            let frames = infra
                .list_frames_for_segment_workspace("session-a", "/tmp/workspaces/segment_%/")
                .await
                .expect("segment workspace frames should list");

            assert_eq!(frames, vec![matching]);
        });
    }

    #[test]
    fn captured_frame_pipeline_persists_frame_batch_and_ocr_job() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-pipeline");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-pipeline", "frame-pipeline.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");

            let job = persisted.job.expect("ocr job should be queued");
            assert_eq!(job.subject_type, FRAME_SUBJECT_TYPE);
            assert_eq!(job.subject_id, persisted.frame.id);
            assert_eq!(job.processor, OCR_PROCESSOR);
            assert_eq!(job.status, ProcessingJobStatus::Queued);
            assert_eq!(job.payload_json.as_deref(), Some("{\"language\":\"eng\"}"));
            assert_eq!(persisted.active_batch.session_id, "session-pipeline");
            assert_eq!(persisted.active_batch.frame_count, 1);
            assert!(persisted.closed_batches.is_empty());

            let batch_frames = infra
                .list_frames_for_batch(persisted.active_batch.id)
                .await
                .expect("batch frames should list");
            assert_eq!(batch_frames, vec![persisted.frame]);
        });
    }

    #[test]
    fn stopping_session_closes_and_schedules_last_open_frame_batch() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-stop-close");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-stop-close", "frame-final.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            assert_eq!(persisted.active_batch.status, FrameBatchStatus::Open);
            assert!(persisted.active_batch.finalize_job_id.is_none());

            let closed = infra
                .close_and_schedule_all_frame_batches_for_session("session-stop-close")
                .await
                .expect("stopped session should close final batch");
            assert_eq!(closed.len(), 1);

            let batch = infra
                .get_frame_batch(persisted.active_batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(batch.status, FrameBatchStatus::Closed);
            assert!(
                batch.finalize_job_id.is_some(),
                "closed final batch should schedule cleanup finalization"
            );
        });
    }

    #[test]
    fn debug_insert_frame_and_enqueue_ocr_job_persists_linked_subject() {
        run_async_test(async {
            let dir = TestDir::new("processing-enqueue");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
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
            let width = 32;
            let height = 32;
            let repeated_pixels = solid_rgba(width, height, [64, 64, 64, 255]);
            let mut changed_pixels = repeated_pixels.clone();
            for y in 8..20 {
                for x in 8..20 {
                    set_pixel_rgba(&mut changed_pixels, width, x, y, [240, 240, 240, 255]);
                }
            }

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let duplicate = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("duplicate frame should persist");
            let changed = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe",
                        "frame-3.png",
                        "2026-04-12T10:00:02Z",
                        &changed_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("changed frame should persist");

            assert!(first.job.is_some());
            assert!(duplicate.job.is_none());
            assert!(changed.job.is_some());

            let frames = infra
                .list_frames(Some("session-dedupe"), None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[0].equivalence.hint, changed.frame.equivalence.hint);
            assert_eq!(frames[1].equivalence.hint, duplicate.frame.equivalence.hint);
            assert_eq!(frames[2].equivalence.hint, first.frame.equivalence.hint);

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
            let width = 32;
            let height = 32;
            let repeated_pixels = solid_rgba(width, height, [64, 64, 64, 255]);
            let mut changed_pixels = repeated_pixels.clone();
            for y in 8..20 {
                for x in 8..20 {
                    set_pixel_rgba(&mut changed_pixels, width, x, y, [240, 240, 240, 255]);
                }
            }

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-repeat",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let changed = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-repeat",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &changed_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("changed frame should persist");
            let repeated = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-repeat",
                        "frame-3.png",
                        "2026-04-12T10:00:02Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("repeated frame should persist");

            assert!(first.job.is_some());
            assert!(changed.job.is_some());
            assert!(repeated.job.is_none());

            let frames = infra
                .list_frames(Some("session-dedupe-repeat"), None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[0].equivalence.hint, repeated.frame.equivalence.hint);
            assert_eq!(frames[1].equivalence.hint, changed.frame.equivalence.hint);
            assert_eq!(frames[2].equivalence.hint, first.frame.equivalence.hint);

            let repeated_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(repeated.frame.id))
                .await
                .expect("repeated jobs should list");
            assert!(repeated_jobs.is_empty());

            let resolved = infra
                .get_nearest_earlier_equivalent_frame(repeated.frame.id)
                .await
                .expect("nearest earlier equivalent frame should resolve");
            assert_eq!(resolved, Some(first.frame));
        });
    }

    #[test]
    fn equivalent_frames_in_different_segment_workspaces_do_not_skip_ocr() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-segment-scoped");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [88, 88, 88, 255]);

            let first = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-scope",
                        1,
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-scope",
                        2,
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(
                second.job.is_some(),
                "equivalent frames in different segment workspaces must not skip OCR"
            );
        });
    }

    #[test]
    fn earlier_equivalent_frame_lookup_does_not_cross_segment_workspaces() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-ui-fallback-segment-scoped");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [104, 104, 104, 255]);

            let first = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-ui-scope",
                        1,
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-ui-scope",
                        2,
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(second.job.is_some());

            let resolved = infra
                .get_nearest_earlier_equivalent_frame(second.frame.id)
                .await
                .expect("cross-segment equivalent frame lookup should succeed");
            assert_eq!(resolved, None);
        });
    }

    #[test]
    fn same_fingerprint_but_different_frame_bytes_skip_ocr() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-byte-confirmation");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [96, 96, 96, 255]);

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-confirmed",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-confirmed",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(
                second.job.is_none(),
                "same-fingerprint frames should skip OCR even when bytes differ"
            );
        });
    }

    #[test]
    fn same_fingerprint_but_different_frame_bytes_stay_skipped_after_time_passes() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-same-fingerprint-late");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [112, 112, 112, 255]);

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-same-fingerprint-late",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-same-fingerprint-late",
                        "frame-2.png",
                        "2026-04-12T10:00:06Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(
                second.job.is_none(),
                "same-fingerprint frames should stay skipped until the fingerprint changes"
            );
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
                .capture_frame(
                    &NewFrame::new(
                        "session-batch-reconcile",
                        "/tmp/session-batch-reconcile-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("frame and OCR state should persist");
            assert_eq!(persisted.active_batch.id, batch.id);

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
    fn startup_reconciles_open_batches_from_stopped_sessions() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-startup-open-reconcile");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let batch = initial
                .frame_batches()
                .upsert_open_batch_for_frame("session-open-reconcile", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should persist");
            let frame = initial
                .processing()
                .insert_frame(&test_frame("session-open-reconcile", "frame-open.png"))
                .await
                .expect("frame should persist");
            initial
                .frame_batches()
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");

            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should recover open batches");
            let reconciled = recovered
                .get_frame_batch(batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(reconciled.status, FrameBatchStatus::Closed);
            assert!(
                reconciled.finalize_job_id.is_some(),
                "startup should schedule finalization for orphaned open batch"
            );
        });
    }

    #[test]
    fn startup_retries_failed_finalize_jobs_and_repairs_processing_batches() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-startup-finalize-retry");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = initial
                .capture_frame(
                    &NewFrame::new(
                        "session-finalize-retry",
                        "/tmp/session-finalize-retry-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("frame should persist");

            let closed = initial
                .close_and_schedule_all_frame_batches_for_session("session-finalize-retry")
                .await
                .expect("batch should close and schedule");
            assert_eq!(closed.len(), 1);

            let scheduled_batch = initial
                .get_frame_batch(closed[0].id)
                .await
                .expect("scheduled batch should be readable")
                .expect("scheduled batch should exist");
            let first_job_id = scheduled_batch
                .finalize_job_id
                .expect("closed batch should have finalize job");
            initial
                .jobs()
                .mark_failed(first_job_id, Some("expected finalize failure"))
                .await
                .expect("finalize job should fail");

            initial
                .frame_batches()
                .mark_batch_processing(persisted.active_batch.id)
                .await
                .expect("batch should enter processing to simulate interrupted finalization");

            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let repaired = recovered
                .get_frame_batch(persisted.active_batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(repaired.status, FrameBatchStatus::Closed);
            let retried_job_id = repaired
                .finalize_job_id
                .expect("startup should schedule replacement finalize job");
            assert_ne!(retried_job_id, first_job_id);

            let retried_job = recovered
                .get_job(retried_job_id)
                .await
                .expect("replacement finalize job should be readable")
                .expect("replacement finalize job should exist");
            assert_eq!(retried_job.kind, FRAME_BATCH_FINALIZE_JOB_KIND);
            assert_eq!(retried_job.status, BackgroundJobStatus::Queued);

            let original_job = recovered
                .get_job(first_job_id)
                .await
                .expect("original finalize job should be readable")
                .expect("original finalize job should exist");
            assert_eq!(original_job.status, BackgroundJobStatus::Failed);

            let finalize_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs WHERE kind = ?1")
                    .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
                    .fetch_one(recovered.pool())
                    .await
                    .expect("finalize jobs should count");
            assert_eq!(finalize_job_count, 2);
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
                .captured_frame_pipeline()
                .capture_frame_in_transaction(
                    &mut transaction,
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-batch-atomic",
                        "frame-atomic.png",
                        "2026-04-12T10:01:00Z",
                        &solid_rgba(32, 32, [72, 72, 72, 255]),
                        32,
                        32,
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
                .list_frames(Some("session-batch-atomic"), None, None, None)
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
                .capture_frame(
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
                .capture_frame(
                    &NewFrame::new(
                        "session-batches",
                        "/tmp/session-batches-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");
            assert_eq!(second.closed_batches.len(), 1);
            assert_eq!(second.closed_batches[0].id, first.active_batch.id);

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
    fn batch_insert_rolls_back_frame_and_batch_when_finalize_scheduling_fails() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-finalize-atomic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "session-finalize-atomic",
                        "/tmp/session-finalize-atomic-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");

            sqlx::query(
                "CREATE TRIGGER fail_frame_batch_finalize_job \
                 BEFORE INSERT ON background_jobs \
                 WHEN NEW.kind = 'frame_batch_combine' \
                 BEGIN \
                     SELECT RAISE(FAIL, 'forced finalize scheduling failure'); \
                 END",
            )
            .execute(infra.pool())
            .await
            .expect("failure trigger should install");

            let error = infra
                .capture_frame(
                    &NewFrame::new(
                        "session-finalize-atomic",
                        "/tmp/session-finalize-atomic-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect_err("finalize scheduling failure should abort batch insert");
            assert!(matches!(error, AppInfraError::Sqlx(_)));

            sqlx::query("DROP TRIGGER fail_frame_batch_finalize_job")
                .execute(infra.pool())
                .await
                .expect("failure trigger should drop");

            let batches = infra
                .list_frame_batches(Some("session-finalize-atomic"))
                .await
                .expect("batches should list");
            assert_eq!(batches.len(), 1);
            assert_eq!(batches[0].status, FrameBatchStatus::Open);
            assert_eq!(batches[0].frame_count, 1);
            assert!(batches[0].finalize_job_id.is_none());

            let frames = infra
                .list_frames(Some("session-finalize-atomic"), None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].id, first.frame.id);

            let finalize_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs WHERE kind = ?1")
                    .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
                    .fetch_one(infra.pool())
                    .await
                    .expect("finalize jobs should count");
            assert_eq!(finalize_job_count, 0);
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
                .capture_frame(&test_frame("session-no-fingerprint", "frame-1.png"), None)
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(&test_frame("session-no-fingerprint", "frame-2.png"), None)
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(second.job.is_some());
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_creates_job_when_none_exists() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-create");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [80, 80, 80, 255]);

            infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-reprocess-create",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let duplicate = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-reprocess-create",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("duplicate frame should persist");
            assert!(duplicate.job.is_none());

            let reprocessed = infra
                .reprocess_captured_frame_ocr(duplicate.frame.id, Some("{\"language\":\"eng\"}"))
                .await
                .expect("reprocessing should create an OCR job");

            assert_eq!(
                reprocessed.outcome,
                CapturedFrameReprocessingOutcome::Created
            );
            assert_eq!(reprocessed.job.subject_id, duplicate.frame.id);
            assert_eq!(reprocessed.job.processor, OCR_PROCESSOR);
            assert_eq!(reprocessed.job.status, ProcessingJobStatus::Queued);
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );

            let subject_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(duplicate.frame.id))
                .await
                .expect("subject jobs should list");
            assert_eq!(subject_jobs, vec![reprocessed.job.clone()]);
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_ignores_queued_job_and_keeps_payload() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-ignore");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-reprocess-ignore", "frame-queued.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            let queued_job = persisted.job.expect("ocr job should be queued");

            let reprocessed = infra
                .reprocess_captured_frame_ocr(queued_job.subject_id, Some("{\"language\":\"fra\"}"))
                .await
                .expect("queued reprocessing should be ignored");

            assert_eq!(
                reprocessed.outcome,
                CapturedFrameReprocessingOutcome::Ignored
            );
            assert_eq!(reprocessed.job.id, queued_job.id);
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_requeues_terminal_job_and_clears_results() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-requeue");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-reprocess-requeue", "frame-terminal.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            let queued_job = persisted.job.expect("ocr job should be queued");

            infra
                .claim_queued_processing_job(queued_job.id)
                .await
                .expect("job should start")
                .expect("job should claim successfully");
            infra
                .complete_processing_job(
                    queued_job.id,
                    &ProcessingResultDraft::new().with_result_text("first pass"),
                )
                .await
                .expect("job should complete");
            assert!(infra
                .get_processing_result_for_job(queued_job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());

            let reprocessed = infra
                .reprocess_captured_frame_ocr(persisted.frame.id, Some("{\"language\":\"fra\"}"))
                .await
                .expect("terminal job should requeue");

            assert_eq!(
                reprocessed.outcome,
                CapturedFrameReprocessingOutcome::Requeued
            );
            assert_eq!(reprocessed.job.id, queued_job.id);
            assert_eq!(reprocessed.job.status, ProcessingJobStatus::Queued);
            assert_eq!(reprocessed.job.attempt_count, 1);
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some("{\"language\":\"fra\"}")
            );
            assert_eq!(reprocessed.job.last_error, None);
            assert!(reprocessed.job.started_at.is_none());
            assert!(reprocessed.job.finished_at.is_none());
            assert!(infra
                .get_processing_result_for_job(queued_job.id)
                .await
                .expect("requeued result lookup should succeed")
                .is_none());

            let subject_results = infra
                .list_processing_results_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject results should list");
            assert!(subject_results.is_empty());
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_rejects_running_job() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-running");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-reprocess-running", "frame-running.png"),
                    None,
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            let queued_job = persisted.job.expect("ocr job should be queued");

            infra
                .claim_queued_processing_job(queued_job.id)
                .await
                .expect("job should start")
                .expect("job should claim successfully");

            let error = infra
                .reprocess_captured_frame_ocr(persisted.frame.id, None)
                .await
                .expect_err("running jobs should reject reprocessing");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == queued_job.id && from == "running" && to == "queued"
            ));
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_requires_existing_frame() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-missing-frame");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let error = infra
                .reprocess_captured_frame_ocr(404, None)
                .await
                .expect_err("missing frames should fail");

            assert!(matches!(error, AppInfraError::FrameNotFound(404)));
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
                .debug_insert_frame_and_enqueue_ocr_job(
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
                .debug_insert_frame_and_enqueue_ocr_job(
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
                .debug_insert_frame_and_enqueue_ocr_job(
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
                .debug_insert_frame_and_enqueue_ocr_job(
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
                .debug_insert_frame_and_enqueue_ocr_job(
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
