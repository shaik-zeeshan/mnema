use std::{collections::HashMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ocr::{FrozenOcrPayload, OcrProvider, OcrRequest};

use crate::{AppInfraError, Frame, Result};

use super::{
    ProcessingJob, ProcessingResultDraft, ProcessingStore, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};

const OCR_SOURCE_IMAGE_PATH_OPTION: &str = "mnemaSourceImagePath";

#[derive(Clone)]
pub struct OcrProcessorBackend {
    providers: HashMap<String, Arc<dyn OcrProvider>>,
}

impl OcrProcessorBackend {
    pub fn new<P>(provider: P) -> Self
    where
        P: OcrProvider + 'static,
    {
        Self::from_arc(Arc::new(provider))
    }

    pub fn from_arc(provider: Arc<dyn OcrProvider>) -> Self {
        let provider_id = provider.provider().to_string();
        Self {
            providers: HashMap::from([(provider_id, provider)]),
        }
    }

    pub fn from_provider_arcs<I>(providers: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn OcrProvider>>,
    {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.provider().to_string(), provider))
                .collect(),
        }
    }

    fn provider_for(&self, provider: &str) -> Result<Arc<dyn OcrProvider>> {
        self.providers.get(provider).cloned().ok_or_else(|| {
            AppInfraError::OcrEngine(format!("ocr provider is not registered for '{provider}'"))
        })
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

fn source_image_path_from_payload(payload: &FrozenOcrPayload) -> Option<PathBuf> {
    payload
        .options
        .get(OCR_SOURCE_IMAGE_PATH_OPTION)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
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
        let mut payload = FrozenOcrPayload::from_payload_json(job.payload_json.as_deref())
            .map_err(|error| AppInfraError::OcrEngine(error.to_string()))?;
        let provider = self.provider_for(&payload.provider)?;
        let image_path = source_image_path_from_payload(&payload)
            .unwrap_or_else(|| PathBuf::from(frame.file_path));
        payload.options.remove(OCR_SOURCE_IMAGE_PATH_OPTION);
        let request: OcrRequest = payload.to_request(image_path);
        let output = provider
            .recognize(request)
            .await
            .map_err(|error| AppInfraError::OcrEngine(error.to_string()))?;
        let provider_id = provider.provider();
        let processor_version = output
            .provider_version
            .clone()
            .map(|provider_version| format!("{provider_id}:{provider_version}"))
            .unwrap_or_else(|| provider_id.to_string());
        let structured_payload_json = output
            .structured_payload_json()
            .map_err(|error| AppInfraError::OcrEngine(error.to_string()))?;

        Ok(ProcessingResultDraft::new()
            .with_result_text(output.text)
            .with_processor_version(processor_version)
            .with_structured_payload_json(structured_payload_json))
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

    use ocr::{
        OcrBoundingBox, OcrObservation, OcrOutput, OcrStructuredPayload, TESSERACT_PROVIDER_ID,
    };

    use super::*;
    use crate::{
        db::{CaptureDb, Database},
        AppInfraError, ProcessorBackend,
    };

    #[derive(Debug, Clone)]
    enum MockOcrResponse {
        Success(OcrOutput),
        Failure(String),
    }

    #[derive(Debug)]
    struct MockOcrProvider {
        provider: &'static str,
        response: MockOcrResponse,
        requests: Mutex<Vec<OcrRequest>>,
    }

    impl MockOcrProvider {
        fn succeed(provider: &'static str, output: OcrOutput) -> Self {
            Self {
                provider,
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
    impl OcrProvider for MockOcrProvider {
        fn provider(&self) -> &'static str {
            self.provider
        }

        async fn recognize(&self, request: OcrRequest) -> ocr::OcrResult<OcrOutput> {
            self.requests
                .lock()
                .expect("mock ocr requests should be writable")
                .push(request);

            match &self.response {
                MockOcrResponse::Success(output) => Ok(output.clone()),
                MockOcrResponse::Failure(message) => Err(ocr::OcrError::Provider(message.clone())),
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
    fn ocr_backend_builds_request_and_result_through_provider_abstraction() {
        run_async_test(async {
            let dir = TestDir::new("ocr-backend-success");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(CaptureDb::single(database.pool().clone()));
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
                        .with_payload_json("{\"provider\":\"tesseract\",\"modelId\":\"tesseract-5.5.2\",\"language\":\"eng\"}"),
                )
                .await
                .expect("job should persist");

            let engine = Arc::new(MockOcrProvider::succeed(
                TESSERACT_PROVIDER_ID,
                OcrOutput::new(
                    "recognized text",
                    OcrStructuredPayload::new(
                        TESSERACT_PROVIDER_ID,
                        Some("tesseract-5.5.2".to_string()),
                        vec![OcrObservation::new(
                            "recognized text",
                            0.99,
                            OcrBoundingBox::new(0.1, 0.2, 0.3, 0.4),
                        )],
                    ),
                )
                .with_provider_version("5.5.2"),
            ));
            let backend = OcrProcessorBackend::from_arc(engine.clone());

            let result = backend
                .process(&store, &job)
                .await
                .expect("ocr backend should produce a result");

            assert_eq!(result.result_text.as_deref(), Some("recognized text"));
            assert_eq!(result.processor_version.as_deref(), Some("tesseract:5.5.2"));
            let structured: serde_json::Value = serde_json::from_str(
                result
                    .structured_payload_json
                    .as_deref()
                    .expect("structured payload"),
            )
            .expect("payload parses");
            assert_eq!(structured["provider"], TESSERACT_PROVIDER_ID);
            assert_eq!(structured["modelId"], "tesseract-5.5.2");

            let requests = engine.recorded_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].image_path,
                PathBuf::from("/tmp/frame-ocr-success.png")
            );
            assert_eq!(requests[0].provider, TESSERACT_PROVIDER_ID);
            assert_eq!(requests[0].model_id.as_deref(), Some("tesseract-5.5.2"));
            assert_eq!(requests[0].language.as_deref(), Some("eng"));
        });
    }

    #[test]
    fn ocr_backend_uses_materialized_source_image_path_when_original_frame_file_is_missing() {
        run_async_test(async {
            let dir = TestDir::new("ocr-backend-source-image");
            let source_image_path = dir.path().join("materialized-preview.jpg");
            fs::write(&source_image_path, b"preview bytes")
                .expect("materialized preview should exist");
            let missing_frame_path = dir.path().join("missing-hidden-frame.jpg");
            let payload_json = serde_json::json!({
                "provider": "tesseract",
                "modelId": "tesseract-5.5.2",
                "language": "eng",
                "options": {
                    OCR_SOURCE_IMAGE_PATH_OPTION: source_image_path,
                    "pageSegmentationMode": "single_block"
                }
            })
            .to_string();

            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(CaptureDb::single(database.pool().clone()));
            let frame = store
                .insert_frame(&super::super::NewFrame::new(
                    "session-ocr",
                    missing_frame_path.to_string_lossy(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            let job = store
                .enqueue_job(
                    &super::super::ProcessingJobDraft::for_frame_ocr(frame.id)
                        .with_payload_json(payload_json),
                )
                .await
                .expect("job should persist");

            let engine = Arc::new(MockOcrProvider::succeed(
                TESSERACT_PROVIDER_ID,
                OcrOutput::new(
                    "recognized text",
                    OcrStructuredPayload::new(TESSERACT_PROVIDER_ID, None, vec![]),
                ),
            ));
            let backend = OcrProcessorBackend::from_arc(engine.clone());

            backend
                .process(&store, &job)
                .await
                .expect("ocr backend should use materialized preview source");

            let requests = engine.recorded_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].image_path, source_image_path);
            assert!(!requests[0]
                .options
                .contains_key(OCR_SOURCE_IMAGE_PATH_OPTION));
        });
    }

    #[test]
    fn ocr_backend_rejects_unsupported_subject_types() {
        run_async_test(async {
            let dir = TestDir::new("ocr-backend-subject");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(CaptureDb::single(database.pool().clone()));
            let job = store
                .enqueue_job(&super::super::ProcessingJobDraft::new(
                    super::super::ProcessingSubject::new("document", 42),
                    OCR_PROCESSOR,
                ))
                .await
                .expect("job should persist");

            let backend = OcrProcessorBackend::from_arc(Arc::new(MockOcrProvider {
                provider: ocr::APPLE_VISION_PROVIDER_ID,
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
