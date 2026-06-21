use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use speaker_analysis::{
    builtin_model_manifest, find_model_descriptor, SpeakerAnalysisProvider, SpeakerAnalysisRequest,
    SpeakerAnalysisResult as ProviderResult, SPEAKRS_DEFAULT_MODEL_ID, SPEAKRS_PROVIDER_ID,
};

use crate::{AppInfraError, AudioSegment, Result};

use super::{
    ProcessingJob, ProcessingResultDraft, ProcessingStore, AUDIO_SEGMENT_SUBJECT_TYPE,
    SPEAKER_ANALYSIS_PROCESSOR,
};

pub const HELPER_TIMEOUT_SECONDS_OPTION: &str = "helperTimeoutSeconds";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisJobPayload {
    // A valid-JSON payload that omits `provider` must still deserialize (not error
    // out) so the Rust cleanup-lock claim path stays symmetric with the SQL-atomic
    // claim path: the SQL CASE keys any valid-JSON speaker_analysis payload onto the
    // normalized speakrs default key, so a missing provider here defaults to the empty
    // string and `normalize_model_selection` remaps it onto speakrs the same way. A
    // missing-provider serde Err would otherwise be swallowed as "unlocked" and reopen
    // a delete-out-from-under window.
    #[serde(default)]
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

    /// Normalize a (possibly legacy) speaker-analysis selection onto the sole
    /// surviving provider. speakrs is the only on-device provider, so any
    /// non-speakrs provider — including the legacy `sherpa_onnx` literal frozen on
    /// an in-flight/queued job payload — is remapped to speakrs. After the
    /// provider is normalized, an unknown/legacy `model_id` for speakrs is reset
    /// to the speakrs default.
    pub fn normalize_model_selection(&mut self) {
        if self.provider != SPEAKRS_PROVIDER_ID {
            // Leave a one-time trail when we actually collapse a legacy/non-speakrs
            // selection onto speakrs and discard its model_id. The model_id belongs to
            // a removed provider's voiceprint space, so dropping it is intentional, but
            // an empty already-speakrs payload must NOT log — only an actual non-speakrs
            // provider or a non-empty stale model_id reaches here. (The crate has no
            // logging facade dependency, so this writes a single line to stderr.)
            if !self.provider.is_empty()
                || self
                    .model_id
                    .as_deref()
                    .is_some_and(|model_id| !model_id.is_empty())
            {
                eprintln!(
                    "speaker_analysis: normalized legacy selection onto speakrs (dropped provider={:?} model_id={:?})",
                    self.provider, self.model_id
                );
            }
            self.provider = SPEAKRS_PROVIDER_ID.to_string();
            // The old model_id belongs to a removed provider's voiceprint space;
            // drop it so the speakrs default is selected below.
            self.model_id = None;
        }
        if find_model_descriptor(
            &builtin_model_manifest(),
            SPEAKRS_PROVIDER_ID,
            self.model_id.as_deref(),
        )
        .is_none()
        {
            self.model_id = Some(SPEAKRS_DEFAULT_MODEL_ID.to_string());
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

    pub fn from_provider_arcs<I>(providers: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn SpeakerAnalysisProvider>>,
    {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.provider().to_string(), provider))
                .collect(),
        }
    }

    /// Resolve the analysis provider for a (possibly legacy) provider string.
    ///
    /// speakrs is the sole on-device provider; sherpa-onnx is removed. A request
    /// for the exact provider id is used directly. Any other provider string —
    /// including the legacy `sherpa_onnx` literal on a job payload frozen before
    /// the removal — falls back to the registered speakrs provider rather than
    /// erroring, so legacy work re-runs through speakrs. (Payloads are already
    /// normalized to speakrs in `normalize_model_selection`; this is a defensive
    /// backstop for any path that bypasses that.)
    fn provider_for(&self, provider: &str) -> Result<Arc<dyn SpeakerAnalysisProvider>> {
        if let Some(provider) = self.providers.get(provider).cloned() {
            return Ok(provider);
        }
        self.providers
            .get(SPEAKRS_PROVIDER_ID)
            .cloned()
            .ok_or_else(|| {
                AppInfraError::SpeakerAnalysisEngine(format!(
                    "speaker analysis provider is not registered for '{provider}' and the speakrs fallback is unavailable"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_model_selection_keeps_default_speakrs_model() {
        let mut payload = SpeakerAnalysisJobPayload {
            provider: SPEAKRS_PROVIDER_ID.to_string(),
            model_id: Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            recognize_people: false,
            options: serde_json::Map::new(),
        };

        payload.normalize_model_selection();

        assert_eq!(payload.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(payload.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
    }

    #[test]
    fn normalize_model_selection_falls_back_to_default_for_unknown_speakrs_model() {
        let mut payload = SpeakerAnalysisJobPayload {
            provider: SPEAKRS_PROVIDER_ID.to_string(),
            model_id: Some("bogus-model-xyz".to_string()),
            recognize_people: false,
            options: serde_json::Map::new(),
        };

        payload.normalize_model_selection();

        assert_eq!(payload.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(payload.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
    }

    /// MIGRATION: a job payload frozen with the removed `sherpa_onnx` provider
    /// (and a sherpa model id) is remapped onto speakrs + its default model, so
    /// the queued job re-runs through speakrs rather than against a gone provider.
    #[test]
    fn normalize_model_selection_remaps_legacy_sherpa_provider_to_speakrs() {
        let mut payload = SpeakerAnalysisJobPayload {
            provider: "sherpa_onnx".to_string(),
            model_id: Some("pyannote-3.0-nemo-titanet-small".to_string()),
            recognize_people: false,
            options: serde_json::Map::new(),
        };

        payload.normalize_model_selection();

        assert_eq!(payload.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(payload.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
    }

    /// A valid-JSON payload that omits `provider` must still deserialize (via
    /// `#[serde(default)]`) and normalize onto the speakrs default, so the Rust
    /// cleanup-lock claim path keys it the same way the SQL-atomic claim path does
    /// (which treats any valid-JSON speaker_analysis payload as the speakrs key). A
    /// deserialize error here would otherwise be swallowed as "unlocked" and reopen a
    /// delete-out-from-under window for a corrupt payload.
    #[test]
    fn missing_provider_deserializes_and_normalizes_to_speakrs() {
        let payload: SpeakerAnalysisJobPayload =
            serde_json::from_str(r#"{"modelId":null,"recognizePeople":false}"#)
                .expect("missing provider must default rather than error");

        assert_eq!(payload.provider, "");

        let mut payload = payload;
        payload.normalize_model_selection();

        assert_eq!(payload.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(payload.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
    }

    #[test]
    fn new_remaps_legacy_sherpa_provider_to_speakrs() {
        // `new` runs `normalize_model_selection`, so even constructing a payload
        // with the legacy provider lands on speakrs.
        let payload = SpeakerAnalysisJobPayload::new(
            "sherpa_onnx",
            Some("pyannote-3.0-nemo-titanet-small".to_string()),
        );

        assert_eq!(payload.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(payload.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
    }
}
