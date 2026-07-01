use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use audio_transcription::{
    TranscriptionProvider, TranscriptionRequest, TranscriptionResult as ProviderResult,
};
use serde::{Deserialize, Serialize};

use crate::{AppInfraError, AudioSegment, Result};

use super::{
    ProcessingJob, ProcessingResultDraft, ProcessingStore, AUDIO_SEGMENT_SUBJECT_TYPE,
    AUDIO_TRANSCRIPTION_PROCESSOR,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionJobPayload {
    pub provider: String,
    pub model_id: Option<String>,
    pub language: String,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub options: serde_json::Map<String, serde_json::Value>,
}

impl AudioTranscriptionJobPayload {
    pub fn new(
        provider: impl Into<String>,
        model_id: Option<String>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model_id,
            language: language.into(),
            options: serde_json::Map::new(),
        }
    }

    fn from_job(job: &ProcessingJob) -> Result<Self> {
        let Some(payload_json) = job.payload_json.as_deref() else {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "audio transcription job is missing frozen provider/model payload".to_string(),
            ));
        };
        Ok(serde_json::from_str(payload_json)?)
    }
}

#[derive(Clone)]
pub struct AudioTranscriptionProcessorBackend {
    providers: HashMap<String, Arc<dyn TranscriptionProvider>>,
}

impl AudioTranscriptionProcessorBackend {
    pub fn new<P>(provider: P) -> Self
    where
        P: TranscriptionProvider + 'static,
    {
        Self::from_arc(Arc::new(provider))
    }

    pub fn from_arc(provider: Arc<dyn TranscriptionProvider>) -> Self {
        let provider_id = provider.provider().to_string();
        Self {
            providers: HashMap::from([(provider_id, provider)]),
        }
    }

    pub fn from_provider_arcs<I>(providers: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn TranscriptionProvider>>,
    {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.provider().to_string(), provider))
                .collect(),
        }
    }

    fn provider_for(&self, provider: &str) -> Result<Arc<dyn TranscriptionProvider>> {
        self.providers.get(provider).cloned().ok_or_else(|| {
            AppInfraError::AudioTranscriptionEngine(format!(
                "audio transcription provider is not registered for '{provider}'"
            ))
        })
    }

    async fn load_audio_segment(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<AudioSegment> {
        if job.subject_type != AUDIO_SEGMENT_SUBJECT_TYPE {
            return Err(AppInfraError::UnsupportedProcessingSubject {
                processor: job.processor.clone(),
                subject_type: job.subject_type.clone(),
            });
        }

        store
            .get_audio_segment(job.subject_id)
            .await?
            .ok_or(AppInfraError::AudioSegmentNotFound(job.subject_id))
    }
}

#[async_trait]
impl super::ProcessorBackend for AudioTranscriptionProcessorBackend {
    fn processor(&self) -> &'static str {
        AUDIO_TRANSCRIPTION_PROCESSOR
    }

    async fn process(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<ProcessingResultDraft> {
        let segment = self.load_audio_segment(store, job).await?;
        let payload = AudioTranscriptionJobPayload::from_job(job)?;
        let provider = self.provider_for(&payload.provider)?;
        let request = TranscriptionRequest::new(
            segment.file_path,
            payload.provider,
            payload.model_id,
            payload.language,
        );
        let mut request = request;
        request.options = payload.options.into_iter().collect();

        let output = map_provider_result(provider.transcribe(request).await)?;
        let provider = provider.provider();
        let processor_version = output
            .provider_version
            .clone()
            .map(|version| format!("{provider}:{version}"))
            .unwrap_or_else(|| provider.to_string());
        let structured_payload_json = output.structured_payload_json()?;

        Ok(ProcessingResultDraft::new()
            .with_result_text(output.text)
            .with_processor_version(processor_version)
            .with_structured_payload_json(structured_payload_json))
    }
}

fn map_provider_result<T>(result: ProviderResult<T>) -> Result<T> {
    result.map_err(|error| AppInfraError::AudioTranscriptionEngine(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use audio_transcription::{
        TranscriptionMetadata, TranscriptionOutput, TranscriptionProvider, TranscriptionRequest,
        TranscriptionSegment, TranscriptionWord, LOCAL_WHISPER_PROVIDER_ID,
    };

    use super::*;
    use crate::{
        db::{CaptureDb, Database},
        AudioSegmentSourceKind, NewAudioSegment, ProcessingJobDraft, ProcessorBackend,
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
    struct MockTranscriptionProvider {
        requests: Mutex<Vec<TranscriptionRequest>>,
    }

    impl MockTranscriptionProvider {
        fn recorded_requests(&self) -> Vec<TranscriptionRequest> {
            self.requests
                .lock()
                .expect("requests should be readable")
                .clone()
        }
    }

    #[async_trait]
    impl TranscriptionProvider for MockTranscriptionProvider {
        fn provider(&self) -> &'static str {
            LOCAL_WHISPER_PROVIDER_ID
        }

        async fn transcribe(
            &self,
            request: TranscriptionRequest,
        ) -> audio_transcription::TranscriptionResult<TranscriptionOutput> {
            self.requests
                .lock()
                .expect("requests should be writable")
                .push(request.clone());

            let mut metadata = TranscriptionMetadata::from_request(&request);
            metadata.segments.push(TranscriptionSegment {
                start_ms: 250,
                end_ms: 1_250,
                text: "hello world".to_string(),
                confidence: Some(0.95),
            });
            metadata.words.push(TranscriptionWord {
                start_ms: 250,
                end_ms: 700,
                text: "hello".to_string(),
                confidence: Some(0.97),
            });

            Ok(TranscriptionOutput::new("hello world", metadata).with_provider_version("mock-1"))
        }
    }

    #[test]
    fn backend_loads_audio_segment_and_persists_transcription_contract() {
        run_async_test(async {
            let dir = TestDir::new("audio-transcription-backend");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = ProcessingStore::new(CaptureDb::single(database.pool().clone()));
            let segment = store
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/mic-1.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should persist");
            let payload = AudioTranscriptionJobPayload::new(
                LOCAL_WHISPER_PROVIDER_ID,
                Some("base".to_string()),
                "auto",
            );
            let job = store
                .enqueue_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(segment.id)
                        .with_payload_json(
                            serde_json::to_string(&payload).expect("payload serializes"),
                        ),
                )
                .await
                .expect("job should persist");
            let provider = Arc::new(MockTranscriptionProvider {
                requests: Mutex::new(Vec::new()),
            });
            let backend = AudioTranscriptionProcessorBackend::from_arc(provider.clone());

            let result = backend
                .process(&store, &job)
                .await
                .expect("backend should produce result");

            assert_eq!(result.result_text.as_deref(), Some("hello world"));
            assert_eq!(
                result.processor_version.as_deref(),
                Some("local_whisper:mock-1")
            );
            let structured: serde_json::Value = serde_json::from_str(
                result
                    .structured_payload_json
                    .as_deref()
                    .expect("structured payload"),
            )
            .expect("structured payload parses");
            assert_eq!(structured["provider"], LOCAL_WHISPER_PROVIDER_ID);
            assert_eq!(structured["modelId"], "base");
            assert_eq!(structured["language"], "auto");
            assert_eq!(structured["segments"][0]["startMs"], 250);
            assert_eq!(structured["segments"][0]["endMs"], 1_250);
            assert_eq!(structured["words"][0]["startMs"], 250);
            assert_eq!(structured["words"][0]["text"], "hello");

            let requests = provider.recorded_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].audio_path, PathBuf::from("/tmp/mic-1.m4a"));
            assert_eq!(requests[0].model_id.as_deref(), Some("base"));
        });
    }
}
