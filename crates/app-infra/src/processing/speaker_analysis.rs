use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use speaker_analysis::{
    SpeakerAnalysisProvider, SpeakerAnalysisRequest, SpeakerAnalysisResult as ProviderResult,
    DEFAULT_SHERPA_ONNX_MODEL_ID, SHERPA_ONNX_PROVIDER_ID,
};

use crate::{AppInfraError, AudioSegment, Result};

use super::{
    ProcessingJob, ProcessingResultDraft, ProcessingStore, AUDIO_SEGMENT_SUBJECT_TYPE,
    SPEAKER_ANALYSIS_PROCESSOR,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisJobPayload {
    pub provider: String,
    pub model_id: Option<String>,
    pub recognize_people: bool,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub options: serde_json::Map<String, serde_json::Value>,
}

impl SpeakerAnalysisJobPayload {
    pub fn new(provider: impl Into<String>, model_id: Option<String>) -> Self {
        let mut payload = Self {
            provider: provider.into(),
            model_id,
            recognize_people: false,
            options: serde_json::Map::new(),
        };
        payload.normalize_model_selection();
        payload
    }

    pub fn normalize_model_selection(&mut self) {
        if self.provider == SHERPA_ONNX_PROVIDER_ID
            && self.model_id.as_deref() != Some(DEFAULT_SHERPA_ONNX_MODEL_ID)
        {
            self.model_id = Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string());
        }
    }

    fn from_job(job: &ProcessingJob) -> Result<Self> {
        let Some(payload_json) = job.payload_json.as_deref() else {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "speaker analysis job is missing frozen provider/model payload".to_string(),
            ));
        };
        let mut payload: Self = serde_json::from_str(payload_json)?;
        payload.normalize_model_selection();
        Ok(payload)
    }
}

#[derive(Clone)]
pub struct SpeakerAnalysisProcessorBackend {
    providers: HashMap<String, Arc<dyn SpeakerAnalysisProvider>>,
}

impl SpeakerAnalysisProcessorBackend {
    pub fn new<P>(provider: P) -> Self
    where
        P: SpeakerAnalysisProvider + 'static,
    {
        Self::from_arc(Arc::new(provider))
    }

    pub fn from_arc(provider: Arc<dyn SpeakerAnalysisProvider>) -> Self {
        let provider_id = provider.provider().to_string();
        Self {
            providers: HashMap::from([(provider_id, provider)]),
        }
    }

    fn provider_for(&self, provider: &str) -> Result<Arc<dyn SpeakerAnalysisProvider>> {
        self.providers.get(provider).cloned().ok_or_else(|| {
            AppInfraError::SpeakerAnalysisEngine(format!(
                "speaker analysis provider is not registered for '{provider}'"
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
impl super::ProcessorBackend for SpeakerAnalysisProcessorBackend {
    fn processor(&self) -> &'static str {
        SPEAKER_ANALYSIS_PROCESSOR
    }

    async fn process(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<ProcessingResultDraft> {
        let segment = self.load_audio_segment(store, job).await?;
        let payload = SpeakerAnalysisJobPayload::from_job(job)?;
        let provider = self.provider_for(&payload.provider)?;
        let mut request = SpeakerAnalysisRequest::new(
            segment.file_path,
            payload.provider,
            payload.model_id,
            segment.source_session_id,
            segment.id,
        );
        request.recognize_people = payload.recognize_people;
        request.options = payload.options.into_iter().collect();
        if request.recognize_people {
            request.enrolled_people = store
                .list_person_enrollments_for_speaker_model(
                    &request.provider,
                    request.model_id.as_deref(),
                )
                .await?;
            request.rejected_people = store
                .list_person_recognition_rejections_for_speaker_model(
                    &request.provider,
                    request.model_id.as_deref(),
                )
                .await?;
        }

        let output = map_provider_result(provider.analyze(request).await)?;
        let provider = provider.provider();
        let processor_version = output
            .provider_version
            .clone()
            .map(|version| format!("{provider}:{version}"))
            .unwrap_or_else(|| provider.to_string());
        let structured_payload_json = output.structured_payload_json()?;

        Ok(ProcessingResultDraft::new()
            .with_processor_version(processor_version)
            .with_structured_payload_json(structured_payload_json))
    }
}

fn map_provider_result<T>(result: ProviderResult<T>) -> Result<T> {
    result.map_err(|error| AppInfraError::SpeakerAnalysisEngine(error.to_string()))
}
