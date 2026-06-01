use super::{
    ProcessingJob, ProcessingJobCompletion, ProcessingJobStatus, ProcessingStore, ProcessorRegistry,
};
use crate::{AppInfraError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingJobRunOutcome {
    Completed(ProcessingJobCompletion),
    Failed(ProcessingJob),
}

#[derive(Clone)]
pub struct ProcessingRuntime {
    store: ProcessingStore,
    registry: ProcessorRegistry,
}

impl ProcessingRuntime {
    pub fn new(store: ProcessingStore, registry: ProcessorRegistry) -> Self {
        Self { store, registry }
    }

    pub async fn process_next_queued_job(&self) -> Result<Option<ProcessingJobRunOutcome>> {
        let Some(job) = self.store.claim_next_queued_job().await? else {
            return Ok(None);
        };

        self.process_claimed_job(job).await.map(Some)
    }

    pub async fn process_next_queued_job_for_processor(
        &self,
        processor: &str,
    ) -> Result<Option<ProcessingJobRunOutcome>> {
        let Some(job) = self
            .store
            .claim_next_queued_job_for_processor(processor)
            .await?
        else {
            return Ok(None);
        };

        self.process_claimed_job(job).await.map(Some)
    }

    pub async fn process_next_queued_job_excluding_processor(
        &self,
        excluded_processor: &str,
    ) -> Result<Option<ProcessingJobRunOutcome>> {
        self.process_next_queued_job_excluding_processors(&[excluded_processor])
            .await
    }

    pub async fn process_next_queued_job_excluding_processors(
        &self,
        excluded_processors: &[&str],
    ) -> Result<Option<ProcessingJobRunOutcome>> {
        let Some(job) = self
            .store
            .claim_next_queued_job_excluding_processors(excluded_processors)
            .await?
        else {
            return Ok(None);
        };

        self.process_claimed_job(job).await.map(Some)
    }

    pub async fn process_job(&self, job_id: i64) -> Result<ProcessingJobRunOutcome> {
        let job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;

        let runnable = match job.status {
            ProcessingJobStatus::Queued => self.store.claim_queued_job(job_id).await?.ok_or(
                AppInfraError::ProcessingJobNotRunnable {
                    job_id,
                    status: job.status.as_str().to_string(),
                },
            )?,
            ProcessingJobStatus::Running => job,
            status => {
                return Err(AppInfraError::ProcessingJobNotRunnable {
                    job_id,
                    status: status.as_str().to_string(),
                });
            }
        };

        self.process_claimed_job(runnable).await
    }

    async fn process_claimed_job(&self, job: ProcessingJob) -> Result<ProcessingJobRunOutcome> {
        let backend = match self.registry.backend_for(&job.processor) {
            Ok(backend) => backend,
            Err(error) => {
                return Ok(ProcessingJobRunOutcome::Failed(
                    self.store
                        .mark_job_failed(job.id, Some(&error.to_string()))
                        .await?,
                ));
            }
        };

        match backend.process(&self.store, &job).await {
            Ok(result) => Ok(ProcessingJobRunOutcome::Completed(
                self.store.complete_job(job.id, &result).await?,
            )),
            Err(error) => {
                let failed = self
                    .store
                    .mark_job_failed(job.id, Some(&error.to_string()))
                    .await?;
                // Bounded-retry genuinely failed work so a transient failure can still recover:
                // a failed OCR job can leave its whole equivalent group textless (later frames
                // that deferred to it via OCR Fallback Eligibility only receive text via
                // back-projection on completion), and a failed audio job loses a segment's
                // transcript and chained speaker labels. Each processor gives up after its own
                // failure cap so a genuinely poison segment stays failed rather than looping.
                self.store
                    .requeue_failed_job_within_attempt_cap(failed.id)
                    .await?;
                Ok(ProcessingJobRunOutcome::Failed(failed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{
        db::Database,
        processing::{
            NewFrame, OcrProcessorBackend, ProcessingJobDraft, ProcessingResultDraft,
            ProcessingSubject, AUDIO_TRANSCRIPTION_PROCESSOR,
        },
    };
    use ocr::{
        OcrBoundingBox, OcrObservation, OcrOutput, OcrProvider, OcrRequest, OcrStructuredPayload,
    };

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
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    #[derive(Debug)]
    struct RecordingBackend {
        processor: &'static str,
        result: ProcessingResultDraft,
        processed_job_ids: Mutex<Vec<i64>>,
    }

    impl RecordingBackend {
        fn successful(processor: &'static str, result_text: &str) -> Self {
            Self {
                processor,
                result: ProcessingResultDraft::new().with_result_text(result_text),
                processed_job_ids: Mutex::new(Vec::new()),
            }
        }

        fn processed_job_ids(&self) -> Vec<i64> {
            self.processed_job_ids
                .lock()
                .expect("processed job ids should be readable")
                .clone()
        }
    }

    #[async_trait]
    impl crate::ProcessorBackend for RecordingBackend {
        fn processor(&self) -> &'static str {
            self.processor
        }

        async fn process(
            &self,
            _store: &ProcessingStore,
            job: &crate::ProcessingJob,
        ) -> Result<ProcessingResultDraft> {
            self.processed_job_ids
                .lock()
                .expect("processed job ids should be writable")
                .push(job.id);

            Ok(self.result.clone())
        }
    }

    /// A backend that always fails, used to exercise the bounded failure-retry path for a given
    /// processor (notably the audio processors, which are now retried like OCR).
    #[derive(Debug)]
    struct FailingBackend {
        processor: &'static str,
    }

    impl FailingBackend {
        fn new(processor: &'static str) -> Self {
            Self { processor }
        }
    }

    #[async_trait]
    impl crate::ProcessorBackend for FailingBackend {
        fn processor(&self) -> &'static str {
            self.processor
        }

        async fn process(
            &self,
            _store: &ProcessingStore,
            _job: &crate::ProcessingJob,
        ) -> Result<ProcessingResultDraft> {
            Err(AppInfraError::AudioTranscriptionEngine(
                "audio engine failed".to_string(),
            ))
        }
    }

    #[derive(Debug)]
    struct MockOcrEngine {
        response: MockOcrResponse,
    }

    #[derive(Debug, Clone)]
    enum MockOcrResponse {
        Success(OcrOutput),
        Failure(String),
    }

    #[async_trait]
    impl OcrProvider for MockOcrEngine {
        fn provider(&self) -> &'static str {
            ocr::APPLE_VISION_PROVIDER_ID
        }

        async fn recognize(&self, _request: OcrRequest) -> ocr::OcrResult<OcrOutput> {
            match &self.response {
                MockOcrResponse::Success(output) => Ok(output.clone()),
                MockOcrResponse::Failure(message) => Err(ocr::OcrError::Provider(message.clone())),
            }
        }
    }

    #[test]
    fn runtime_dispatches_to_backend_matching_processor() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-dispatch");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let alpha = Arc::new(RecordingBackend::successful("alpha", "alpha result"));
            let beta = Arc::new(RecordingBackend::successful("beta", "beta result"));
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new()
                    .register_arc(alpha.clone())
                    .register_arc(beta.clone()),
            );

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    "beta",
                ))
                .await
                .expect("job should enqueue");

            let outcome = runtime
                .process_job(job.id)
                .await
                .expect("runtime should process job");

            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected completed outcome");
            };

            assert_eq!(completion.job.processor, "beta");
            assert_eq!(
                completion.result.result_text.as_deref(),
                Some("beta result")
            );
            assert!(alpha.processed_job_ids().is_empty());
            assert_eq!(beta.processed_job_ids(), vec![job.id]);
        });
    }

    #[test]
    fn runtime_claims_next_queued_job_for_requested_processor() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-processor-claim");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let alpha = Arc::new(RecordingBackend::successful("alpha", "alpha result"));
            let beta = Arc::new(RecordingBackend::successful("beta", "beta result"));
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new()
                    .register_arc(alpha.clone())
                    .register_arc(beta.clone()),
            );

            let alpha_job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    "alpha",
                ))
                .await
                .expect("alpha job should enqueue");
            let beta_job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 2),
                    "beta",
                ))
                .await
                .expect("beta job should enqueue");

            let outcome = runtime
                .process_next_queued_job_for_processor("beta")
                .await
                .expect("runtime should process requested processor")
                .expect("requested processor job should exist");

            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected completed outcome");
            };
            assert_eq!(completion.job.id, beta_job.id);
            assert!(alpha.processed_job_ids().is_empty());
            assert_eq!(beta.processed_job_ids(), vec![beta_job.id]);
            assert_eq!(
                store
                    .get_job(alpha_job.id)
                    .await
                    .expect("alpha job should be readable")
                    .expect("alpha job should exist")
                    .status,
                ProcessingJobStatus::Queued
            );
        });
    }

    #[test]
    fn runtime_processes_queued_ocr_jobs_and_round_trips_structured_results() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-ocr-success");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register(OcrProcessorBackend::new(MockOcrEngine {
                    response: MockOcrResponse::Success(
                        OcrOutput::new(
                            "recognized text",
                            OcrStructuredPayload::new(
                                ocr::APPLE_VISION_PROVIDER_ID,
                                None,
                                vec![OcrObservation::new(
                                    "recognized text",
                                    0.95,
                                    OcrBoundingBox::new(0.1, 0.2, 0.3, 0.4),
                                )],
                            ),
                        )
                        .with_provider_version("vision-1.0"),
                    ),
                })),
            );

            let frame = store
                .insert_frame(&NewFrame::new(
                    "session-runtime",
                    "/tmp/frame-runtime-success.png",
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            let job = store
                .enqueue_job(
                    &ProcessingJobDraft::for_frame_ocr(frame.id)
                        .with_payload_json("{\"language\":\"eng\"}"),
                )
                .await
                .expect("job should persist");

            let outcome = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should process queued job")
                .expect("queued job should exist");

            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected completed outcome");
            };

            assert_eq!(completion.job.id, job.id);
            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);
            assert_eq!(completion.job.attempt_count, 1);
            assert_eq!(
                completion.result.result_text.as_deref(),
                Some("recognized text")
            );
            let structured: serde_json::Value = serde_json::from_str(
                completion
                    .result
                    .structured_payload_json
                    .as_deref()
                    .expect("structured payload should persist"),
            )
            .expect("structured payload should parse");
            assert_eq!(structured["provider"], ocr::APPLE_VISION_PROVIDER_ID);
            assert_eq!(structured["observations"][0]["text"], "recognized text");
            assert_eq!(
                completion.result.processor_version.as_deref(),
                Some("apple_vision:vision-1.0")
            );

            let stored_job = store
                .get_job(job.id)
                .await
                .expect("completed job should be readable")
                .expect("completed job should exist");
            assert_eq!(stored_job.status, ProcessingJobStatus::Completed);

            let stored_result = store
                .get_result_for_job(job.id)
                .await
                .expect("result should be readable")
                .expect("result should exist");
            assert_eq!(stored_result, completion.result);

            let subject_results = store
                .list_results_for_subject(&ProcessingSubject::frame(frame.id))
                .await
                .expect("subject results should be readable");
            assert_eq!(subject_results, vec![completion.result]);
        });
    }

    #[test]
    fn runtime_can_skip_excluded_processors_without_starving_later_work() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-exclude");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let blocked = Arc::new(RecordingBackend::successful("blocked", "blocked result"));
            let allowed = Arc::new(RecordingBackend::successful("allowed", "allowed result"));
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new()
                    .register_arc(blocked.clone())
                    .register_arc(allowed.clone()),
            );

            let blocked_job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    "blocked",
                ))
                .await
                .expect("blocked job should enqueue");
            let allowed_job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 2),
                    "allowed",
                ))
                .await
                .expect("allowed job should enqueue");

            let outcome = runtime
                .process_next_queued_job_excluding_processor("blocked")
                .await
                .expect("runtime should process non-excluded job")
                .expect("non-excluded job should exist");

            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected completed outcome");
            };
            assert_eq!(completion.job.id, allowed_job.id);
            assert!(blocked.processed_job_ids().is_empty());
            assert_eq!(allowed.processed_job_ids(), vec![allowed_job.id]);
            assert_eq!(
                store
                    .get_job(blocked_job.id)
                    .await
                    .expect("blocked job should be readable")
                    .expect("blocked job should exist")
                    .status,
                ProcessingJobStatus::Queued
            );
        });
    }

    #[test]
    fn runtime_marks_failed_jobs_when_backend_errors() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-ocr-failure");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register(OcrProcessorBackend::new(MockOcrEngine {
                    response: MockOcrResponse::Failure("vision bridge failed".to_string()),
                })),
            );

            let frame = store
                .insert_frame(&NewFrame::new(
                    "session-runtime",
                    "/tmp/frame-runtime-failure.png",
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            let queued_job = store
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("job should persist");

            let outcome = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should attempt queued job")
                .expect("queued job should exist");

            let ProcessingJobRunOutcome::Failed(failed_job) = outcome else {
                panic!("expected failed outcome");
            };

            assert_eq!(failed_job.id, queued_job.id);
            assert_eq!(failed_job.status, ProcessingJobStatus::Failed);
            assert_eq!(failed_job.attempt_count, 1);
            assert_eq!(
                failed_job.last_error.as_deref(),
                Some("ocr engine error: ocr provider error: vision bridge failed")
            );

            assert!(store
                .get_result_for_job(queued_job.id)
                .await
                .expect("result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn failed_ocr_jobs_are_bounded_retried_then_left_failed() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-ocr-bounded-retry");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register(OcrProcessorBackend::new(MockOcrEngine {
                    response: MockOcrResponse::Failure("vision bridge failed".to_string()),
                })),
            );

            let frame = store
                .insert_frame(&NewFrame::new(
                    "session-runtime",
                    "/tmp/frame-runtime-bounded-retry.png",
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            let queued_job = store
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("job should persist");

            // First failure: the job is requeued (eligible again) rather than left terminally
            // failed, so the equivalent group can still recover text on a later attempt.
            let outcome = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should attempt queued job")
                .expect("queued job should exist");
            let ProcessingJobRunOutcome::Failed(failed_job) = outcome else {
                panic!("expected failed outcome on first attempt");
            };
            assert_eq!(failed_job.id, queued_job.id);
            assert_eq!(failed_job.attempt_count, 1);

            let after_first = store
                .get_job(queued_job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(
                after_first.status,
                ProcessingJobStatus::Queued,
                "a failed OCR job under the attempt cap should be requeued for retry"
            );

            // The requeued job is deferred by a retry backoff window, so it is not
            // immediately re-claimable even though it is queued.
            assert!(
                runtime
                    .process_next_queued_job()
                    .await
                    .expect("runtime poll should succeed")
                    .is_none(),
                "a requeued OCR job should be deferred by its retry backoff"
            );

            // Drive remaining attempts; expire the backoff before each retry to simulate
            // the wait elapsing, then re-claim the requeued job until the cap is hit.
            let mut last_attempt_count = after_first.attempt_count;
            loop {
                store
                    .expire_processing_job_retry_backoff_for_test(queued_job.id)
                    .await
                    .expect("retry backoff should expire for test");
                let Some(outcome) = runtime
                    .process_next_queued_job()
                    .await
                    .expect("runtime should keep retrying the requeued ocr job")
                else {
                    break;
                };
                let ProcessingJobRunOutcome::Failed(failed) = outcome else {
                    panic!("expected failed outcome on retry");
                };
                last_attempt_count = failed.attempt_count;
            }

            // Once the cap is reached, the job is left terminally failed and not requeued again.
            assert_eq!(
                last_attempt_count,
                super::super::OCR_FAILED_JOB_MAX_ATTEMPTS
            );
            let terminal = store
                .get_job(queued_job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(terminal.status, ProcessingJobStatus::Failed);
            assert_eq!(
                terminal.attempt_count,
                super::super::OCR_FAILED_JOB_MAX_ATTEMPTS
            );

            // No further queued OCR work remains.
            assert!(runtime
                .process_next_queued_job()
                .await
                .expect("runtime poll should succeed")
                .is_none());
        });
    }

    #[test]
    fn requeued_failed_ocr_job_yields_to_fresh_work_during_backoff() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-ocr-retry-backoff-yields");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register(OcrProcessorBackend::new(MockOcrEngine {
                    response: MockOcrResponse::Failure("vision bridge failed".to_string()),
                })),
            );

            let first_frame = store
                .insert_frame(&NewFrame::new(
                    "session-runtime",
                    "/tmp/frame-retry-backoff-first.png",
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("first frame should persist");
            let failing_job = store
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(first_frame.id))
                .await
                .expect("failing job should persist");

            let second_frame = store
                .insert_frame(&NewFrame::new(
                    "session-runtime",
                    "/tmp/frame-retry-backoff-second.png",
                    "2026-04-12T10:00:01Z",
                ))
                .await
                .expect("second frame should persist");
            let fresh_job = store
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(second_frame.id))
                .await
                .expect("fresh job should persist");

            // First poll claims the oldest job, which fails and is requeued with a backoff.
            let ProcessingJobRunOutcome::Failed(first_failed) = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should attempt the oldest job")
                .expect("a queued job should exist")
            else {
                panic!("expected the oldest job to fail");
            };
            assert_eq!(first_failed.id, failing_job.id);

            // The next poll must claim the FRESH job rather than re-claiming the backed-off
            // failing job (which has the lower id), so fresh capture indexing is not starved.
            let ProcessingJobRunOutcome::Failed(second_failed) = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should claim the fresh job during backoff")
                .expect("the fresh job should be claimable")
            else {
                panic!("expected the fresh job to be claimed");
            };
            assert_eq!(
                second_failed.id, fresh_job.id,
                "the requeued failing job must not monopolize the queue during its backoff"
            );

            // The failing job is preserved (queued, awaiting its backoff), not dropped.
            let backed_off = store
                .get_job(failing_job.id)
                .await
                .expect("failing job should be readable")
                .expect("failing job should exist");
            assert_eq!(backed_off.status, ProcessingJobStatus::Queued);
        });
    }

    #[test]
    fn runtime_rejects_direct_processing_for_terminal_jobs() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-terminal");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register_arc(Arc::new(RecordingBackend::successful(
                    "alpha",
                    "alpha result",
                ))),
            );

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    "alpha",
                ))
                .await
                .expect("job should enqueue");
            let claimed = store
                .claim_queued_job(job.id)
                .await
                .expect("job claim should succeed")
                .expect("job should claim");
            let completion = store
                .complete_job(
                    claimed.id,
                    &ProcessingResultDraft::new().with_result_text("done"),
                )
                .await
                .expect("job should complete");
            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);

            let error = runtime
                .process_job(job.id)
                .await
                .expect_err("completed jobs should not be runnable");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobNotRunnable { job_id, ref status }
                    if job_id == job.id && status == "completed"
            ));
        });
    }

    #[test]
    fn orphaned_audio_job_is_requeued_not_failed_and_reruns_to_completion() {
        run_async_test(async {
            let dir = TestDir::new("processing-reclaim-audio-requeue");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let backend = Arc::new(RecordingBackend::successful(
                AUDIO_TRANSCRIPTION_PROCESSOR,
                "recovered transcript",
            ));
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register_arc(backend.clone()),
            );

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                ))
                .await
                .expect("audio job should enqueue");

            // Simulate a worker claiming the job and being aborted mid-job (quit/crash): the row is
            // left running with no live executor.
            let claimed = store
                .claim_queued_job(job.id)
                .await
                .expect("job claim should succeed")
                .expect("job should claim");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
            assert_eq!(claimed.attempt_count, 1);

            // Reclamation requeues the orphaned job rather than failing it.
            let summary = store
                .reconcile_orphaned_running_jobs()
                .await
                .expect("reclamation should succeed");
            assert_eq!(summary.requeued, 1);
            assert_eq!(summary.failed_on_ceiling, 0);

            let reclaimed = store
                .get_job(job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(
                reclaimed.status,
                ProcessingJobStatus::Queued,
                "an orphaned audio job must be requeued, not failed"
            );
            assert_eq!(
                reclaimed.failure_count, 0,
                "abandonment must not spend a failure attempt"
            );
            assert_eq!(
                reclaimed.attempt_count, 1,
                "the abandoned run still counts toward the total attempt ceiling"
            );

            // The reclaimed job re-runs to completion on the next drain.
            let outcome = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should process the reclaimed job")
                .expect("the reclaimed job should be claimable");
            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected the reclaimed job to complete");
            };
            assert_eq!(completion.job.id, job.id);
            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);
            assert_eq!(completion.job.attempt_count, 2);
            assert_eq!(backend.processed_job_ids(), vec![job.id]);
        });
    }

    #[test]
    fn repeated_abandonment_requeues_without_spending_a_failure_attempt() {
        run_async_test(async {
            let dir = TestDir::new("processing-reclaim-no-failure-spend");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                ))
                .await
                .expect("audio job should enqueue");

            // Quit-mid-job several times in a row. Each cycle claims (running) then reclaims
            // (requeue); the failure cap must never be touched by abandonment.
            for expected_attempts in 1..=5 {
                store
                    .claim_queued_job(job.id)
                    .await
                    .expect("job claim should succeed")
                    .expect("job should claim");
                let summary = store
                    .reconcile_orphaned_running_jobs()
                    .await
                    .expect("reclamation should succeed");
                assert_eq!(summary.requeued, 1);
                assert_eq!(summary.failed_on_ceiling, 0);

                let reclaimed = store
                    .get_job(job.id)
                    .await
                    .expect("job should be readable")
                    .expect("job should exist");
                assert_eq!(reclaimed.status, ProcessingJobStatus::Queued);
                assert_eq!(reclaimed.attempt_count, expected_attempts);
                assert_eq!(
                    reclaimed.failure_count, 0,
                    "repeated quits must never exhaust the failure cap"
                );
            }
        });
    }

    #[test]
    fn reclamation_fails_only_after_attempt_ceiling_is_reached() {
        run_async_test(async {
            let dir = TestDir::new("processing-reclaim-ceiling");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                ))
                .await
                .expect("audio job should enqueue");

            // Claim-then-abandon until the absolute attempt ceiling is reached. The ceiling-th
            // claim leaves attempt_count == RECLAIM_ATTEMPT_CEILING, after which reclamation gives
            // up and leaves the job failed instead of requeueing it forever.
            for attempt in 1..=super::super::RECLAIM_ATTEMPT_CEILING {
                store
                    .claim_queued_job(job.id)
                    .await
                    .expect("job claim should succeed")
                    .expect("job should claim while under the ceiling");
                let summary = store
                    .reconcile_orphaned_running_jobs()
                    .await
                    .expect("reclamation should succeed");
                if attempt < super::super::RECLAIM_ATTEMPT_CEILING {
                    assert_eq!(summary.requeued, 1, "under the ceiling the job is requeued");
                    assert_eq!(summary.failed_on_ceiling, 0);
                } else {
                    assert_eq!(
                        summary.requeued, 0,
                        "at the ceiling the job is not requeued again"
                    );
                    assert_eq!(summary.failed_on_ceiling, 1);
                }
            }

            let terminal = store
                .get_job(job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(terminal.status, ProcessingJobStatus::Failed);
            assert_eq!(
                terminal.attempt_count,
                super::super::RECLAIM_ATTEMPT_CEILING
            );
            assert_eq!(
                terminal.failure_count, 0,
                "reaching the reclaim ceiling is not a failure attempt"
            );
        });
    }

    #[test]
    fn failed_audio_jobs_are_bounded_retried_then_left_failed() {
        run_async_test(async {
            let dir = TestDir::new("processing-audio-bounded-retry");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new()
                    .register_arc(Arc::new(FailingBackend::new(AUDIO_TRANSCRIPTION_PROCESSOR))),
            );

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                ))
                .await
                .expect("audio job should enqueue");

            // First failure: requeued for retry rather than left terminally failed.
            let ProcessingJobRunOutcome::Failed(first_failed) = runtime
                .process_next_queued_job()
                .await
                .expect("runtime should attempt the audio job")
                .expect("a queued audio job should exist")
            else {
                panic!("expected the audio job to fail");
            };
            assert_eq!(first_failed.id, job.id);
            assert_eq!(first_failed.failure_count, 1);

            let after_first = store
                .get_job(job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(
                after_first.status,
                ProcessingJobStatus::Queued,
                "a failed audio job under the cap should be requeued for retry"
            );

            // The requeued job is deferred by its backoff window (audio backoff is longer than
            // OCR's), so it is not immediately re-claimable.
            assert!(
                runtime
                    .process_next_queued_job()
                    .await
                    .expect("runtime poll should succeed")
                    .is_none(),
                "a requeued audio job should be deferred by its retry backoff"
            );

            // Drive remaining attempts, expiring the backoff before each retry.
            let mut last_failure_count = after_first.failure_count;
            loop {
                store
                    .expire_processing_job_retry_backoff_for_test(job.id)
                    .await
                    .expect("retry backoff should expire for test");
                let Some(outcome) = runtime
                    .process_next_queued_job()
                    .await
                    .expect("runtime should keep retrying the requeued audio job")
                else {
                    break;
                };
                let ProcessingJobRunOutcome::Failed(failed) = outcome else {
                    panic!("expected failed outcome on retry");
                };
                last_failure_count = failed.failure_count;
            }

            // Once the failure cap is reached, the job stays terminally failed.
            assert_eq!(
                last_failure_count,
                super::super::AUDIO_FAILED_JOB_MAX_ATTEMPTS
            );
            let terminal = store
                .get_job(job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(terminal.status, ProcessingJobStatus::Failed);
            assert_eq!(
                terminal.failure_count,
                super::super::AUDIO_FAILED_JOB_MAX_ATTEMPTS
            );
            assert!(runtime
                .process_next_queued_job()
                .await
                .expect("runtime poll should succeed")
                .is_none());
        });
    }

    #[test]
    fn newly_enqueued_job_starts_with_zero_failure_count() {
        run_async_test(async {
            let dir = TestDir::new("processing-failure-count-default");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());

            let job = store
                .enqueue_job(&ProcessingJobDraft::new(
                    ProcessingSubject::new("document", 1),
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                ))
                .await
                .expect("audio job should enqueue");

            assert_eq!(
                job.failure_count, 0,
                "the failure_count column should default to 0 after migration"
            );
        });
    }
}
