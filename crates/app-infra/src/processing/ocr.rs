use std::sync::Arc;

use async_trait::async_trait;

use crate::{AppInfraError, Result};

use super::{
    engine::{OcrEngine, OcrRequest},
    Frame, ProcessingJob, ProcessingResultDraft, ProcessingStore, FRAME_SUBJECT_TYPE,
    OCR_PROCESSOR,
};

#[derive(Clone)]
pub struct OcrProcessorBackend {
    engine: Arc<dyn OcrEngine>,
}

impl OcrProcessorBackend {
    pub fn new<E>(engine: E) -> Self
    where
        E: OcrEngine + 'static,
    {
        Self {
            engine: Arc::new(engine),
        }
    }

    pub fn from_arc(engine: Arc<dyn OcrEngine>) -> Self {
        Self { engine }
    }

    async fn load_frame(&self, store: &ProcessingStore, job: &ProcessingJob) -> Result<Frame> {
        if job.subject_type != FRAME_SUBJECT_TYPE {
            return Err(AppInfraError::UnsupportedProcessingSubject {
                processor: job.processor.clone(),
                subject_type: job.subject_type.clone(),
            });
        }

        store
            .get_frame(job.subject_id)
            .await?
            .ok_or(AppInfraError::FrameNotFound(job.subject_id))
    }
}

#[async_trait]
impl super::ProcessorBackend for OcrProcessorBackend {
    fn processor(&self) -> &'static str {
        OCR_PROCESSOR
    }

    async fn process(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<ProcessingResultDraft> {
        let frame = self.load_frame(store, job).await?;
        let request = OcrRequest::new(frame.file_path).with_payload_json(job.payload_json.clone());
        let output = self.engine.recognize(request).await?;
        let provider = self.engine.provider().as_str();

        let processor_version = output
            .engine_version
            .map(|engine_version| format!("{provider}:{engine_version}"))
            .unwrap_or_else(|| provider.to_string());

        Ok(ProcessingResultDraft::new()
            .with_result_text(output.text)
            .with_processor_version(processor_version)
            .with_optional_structured_payload_json(output.structured_payload_json))
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

    use super::*;
    use crate::{db::Database, AppInfraError, OcrOutput, ProcessorBackend};

    #[derive(Debug, Clone)]
    enum MockOcrResponse {
        Success(OcrOutput),
        Failure(String),
    }

    #[derive(Debug)]
    struct MockOcrEngine {
        response: MockOcrResponse,
        requests: Mutex<Vec<OcrRequest>>,
    }

    impl MockOcrEngine {
        fn succeed(output: OcrOutput) -> Self {
            Self {
                response: MockOcrResponse::Success(output),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn recorded_requests(&self) -> Vec<OcrRequest> {
            self.requests
                .lock()
                .expect("mock ocr requests should be readable")
                .clone()
        }
    }

    #[async_trait]
    impl OcrEngine for MockOcrEngine {
        fn provider(&self) -> super::super::OcrProvider {
            super::super::OcrProvider::AppleVision
        }

        async fn recognize(&self, request: OcrRequest) -> Result<OcrOutput> {
            self.requests
                .lock()
                .expect("mock ocr requests should be writable")
                .push(request);

            match &self.response {
                MockOcrResponse::Success(output) => Ok(output.clone()),
                MockOcrResponse::Failure(message) => Err(AppInfraError::OcrEngine(message.clone())),
            }
        }
    }

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

    #[test]
    fn ocr_backend_builds_request_and_result_through_engine_abstraction() {
        run_async_test(async {
            let dir = TestDir::new("ocr-backend-success");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let frame = store
                .insert_frame(&super::super::NewFrame::new(
                    "session-ocr",
                    "/tmp/frame-ocr-success.png",
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            let job = store
                .enqueue_job(
                    &super::super::ProcessingJobDraft::for_frame_ocr(frame.id)
                        .with_payload_json("{\"language\":\"eng\"}"),
                )
                .await
                .expect("job should persist");

            let engine = Arc::new(MockOcrEngine::succeed(
                OcrOutput::new("recognized text")
                    .with_structured_payload_json("{\"blocks\":[]}")
                    .with_engine_version("macOS-14.4"),
            ));
            let backend = OcrProcessorBackend::from_arc(engine.clone());

            let result = backend
                .process(&store, &job)
                .await
                .expect("ocr backend should produce a result");

            assert_eq!(result.result_text.as_deref(), Some("recognized text"));
            assert_eq!(
                result.structured_payload_json.as_deref(),
                Some("{\"blocks\":[]}")
            );
            assert_eq!(
                result.processor_version.as_deref(),
                Some("apple_vision:macOS-14.4")
            );

            let requests = engine.recorded_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].image_path,
                PathBuf::from("/tmp/frame-ocr-success.png")
            );
            assert_eq!(
                requests[0].payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );
        });
    }

    #[test]
    fn ocr_backend_rejects_unsupported_subject_types() {
        run_async_test(async {
            let dir = TestDir::new("ocr-backend-subject");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(database.pool().clone());
            let job = store
                .enqueue_job(&super::super::ProcessingJobDraft::new(
                    super::super::ProcessingSubject::new("document", 42),
                    OCR_PROCESSOR,
                ))
                .await
                .expect("job should persist");

            let backend = OcrProcessorBackend::from_arc(Arc::new(MockOcrEngine {
                response: MockOcrResponse::Failure("should not run".to_string()),
                requests: Mutex::new(Vec::new()),
            }));

            let error = backend
                .process(&store, &job)
                .await
                .expect_err("ocr backend should reject unsupported subjects");

            assert!(matches!(
                error,
                AppInfraError::UnsupportedProcessingSubject {
                    ref processor,
                    ref subject_type,
                } if processor == OCR_PROCESSOR && subject_type == "document"
            ));
        });
    }
}
