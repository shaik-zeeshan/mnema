use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscriptionError {
    #[error("audio transcription provider is unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("audio transcription failed: {0}")]
    Transcription(String),
    #[error("invalid transcription request: {0}")]
    InvalidRequest(String),
    /// A transient environmental failure (offline, timeout, rate limit, server error, or a
    /// rejected API key for a cloud provider). ADR 0048: the processing queue requeues the job
    /// with backoff WITHOUT consuming a retry attempt, so an offline or mis-keyed stretch never
    /// burns a segment's failure cap. Distinct from `Transcription`/`InvalidRequest`, which are
    /// genuine per-segment failures on the bounded-retry path.
    #[error("audio transcription provider is temporarily unavailable: {0}")]
    TransientLiveness(String),
}

pub type TranscriptionResult<T> = std::result::Result<T, TranscriptionError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionRequest {
    pub audio_path: PathBuf,
    pub provider: String,
    pub model_id: Option<String>,
    pub language: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,
}

impl TranscriptionRequest {
    pub fn new(
        audio_path: impl Into<PathBuf>,
        provider: impl Into<String>,
        model_id: Option<String>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            audio_path: audio_path.into(),
            provider: provider.into(),
            model_id,
            language: language.into(),
            options: BTreeMap::new(),
        }
    }

    pub fn with_option(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.options.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionMetadata {
    pub provider: String,
    pub model_id: Option<String>,
    pub language: String,
    #[serde(default)]
    pub segments: Vec<TranscriptionSegment>,
    #[serde(default)]
    pub words: Vec<TranscriptionWord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provenance: BTreeMap<String, serde_json::Value>,
}

impl TranscriptionMetadata {
    pub fn from_request(request: &TranscriptionRequest) -> Self {
        Self {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
            language: request.language.clone(),
            segments: Vec::new(),
            words: Vec::new(),
            provenance: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionOutput {
    pub text: String,
    pub metadata: TranscriptionMetadata,
    pub provider_version: Option<String>,
}

impl TranscriptionOutput {
    pub fn new(text: impl Into<String>, metadata: TranscriptionMetadata) -> Self {
        Self {
            text: text.into(),
            metadata,
            provider_version: None,
        }
    }

    pub fn no_speech(metadata: TranscriptionMetadata) -> Self {
        Self::new("", metadata)
    }

    pub fn with_provider_version(mut self, version: impl Into<String>) -> Self {
        self.provider_version = Some(version.into());
        self
    }

    pub fn structured_payload_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.metadata)
    }
}

#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    fn provider(&self) -> &'static str;

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LOCAL_WHISPER_PROVIDER_ID;

    #[derive(Debug, Default)]
    struct MockProvider;

    #[async_trait]
    impl TranscriptionProvider for MockProvider {
        fn provider(&self) -> &'static str {
            LOCAL_WHISPER_PROVIDER_ID
        }

        async fn transcribe(
            &self,
            request: TranscriptionRequest,
        ) -> TranscriptionResult<TranscriptionOutput> {
            let mut metadata = TranscriptionMetadata::from_request(&request);
            metadata.segments.push(TranscriptionSegment {
                start_ms: 10,
                end_ms: 110,
                text: "hi".to_string(),
                confidence: Some(0.9),
            });
            Ok(TranscriptionOutput::new("hi", metadata).with_provider_version("mock-1"))
        }
    }

    #[test]
    fn mock_provider_round_trips_request_and_output_contract() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(async {
                let request = TranscriptionRequest::new(
                    "/tmp/audio.m4a",
                    LOCAL_WHISPER_PROVIDER_ID,
                    Some("base".to_string()),
                    "auto",
                );

                let output = MockProvider
                    .transcribe(request)
                    .await
                    .expect("mock provider should transcribe");

                assert_eq!(output.text, "hi");
                assert_eq!(output.provider_version.as_deref(), Some("mock-1"));
                assert_eq!(output.metadata.provider, LOCAL_WHISPER_PROVIDER_ID);
                assert_eq!(output.metadata.model_id.as_deref(), Some("base"));
                assert_eq!(output.metadata.language, "auto");
                assert_eq!(output.metadata.segments[0].start_ms, 10);
            });
    }
}
