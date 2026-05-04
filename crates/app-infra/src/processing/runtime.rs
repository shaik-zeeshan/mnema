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
            Err(error) => Ok(ProcessingJobRunOutcome::Failed(
                self.store
                    .mark_job_failed(job.id, Some(&error.to_string()))
                    .await?,
            )),
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
            NewFrame, OcrOutput, OcrProcessorBackend, OcrProvider, OcrRequest, ProcessingJobDraft,
            ProcessingResultDraft, ProcessingSubject,
        },
        OcrEngine,
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
    impl OcrEngine for MockOcrEngine {
        fn provider(&self) -> OcrProvider {
            OcrProvider::AppleVision
        }

        async fn recognize(&self, _request: OcrRequest) -> Result<OcrOutput> {
            match &self.response {
                MockOcrResponse::Success(output) => Ok(output.clone()),
                MockOcrResponse::Failure(message) => Err(AppInfraError::OcrEngine(message.clone())),
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
    fn runtime_processes_queued_ocr_jobs_and_round_trips_structured_results() {
        run_async_test(async {
            let dir = TestDir::new("processing-runtime-ocr-success");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let structured_payload_json = r#"{"blocks":[{"text":"recognized text"}]}"#;
            let runtime = ProcessingRuntime::new(
                store.clone(),
                ProcessorRegistry::new().register(OcrProcessorBackend::new(MockOcrEngine {
                    response: MockOcrResponse::Success(
                        OcrOutput::new("recognized text")
                            .with_structured_payload_json(structured_payload_json)
                            .with_engine_version("vision-1.0"),
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
            assert_eq!(
                completion.result.structured_payload_json.as_deref(),
                Some(structured_payload_json)
            );
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
                Some("ocr engine error: vision bridge failed")
            );

            assert!(store
                .get_result_for_job(queued_job.id)
                .await
                .expect("result lookup should succeed")
                .is_none());
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
}
