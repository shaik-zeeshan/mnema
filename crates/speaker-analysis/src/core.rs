use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpeakerAnalysisError {
    #[error("speaker analysis provider is unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("speaker analysis failed: {0}")]
    Analysis(String),
    #[error("invalid speaker analysis request: {0}")]
    InvalidRequest(String),
}

pub type SpeakerAnalysisResult<T> = std::result::Result<T, SpeakerAnalysisError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisRequest {
    pub audio_path: PathBuf,
    pub provider: String,
    pub model_id: Option<String>,
    pub session_id: String,
    pub audio_segment_id: i64,
    pub recognize_people: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enrolled_people: Vec<PersonEnrollment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_people: Vec<PersonRecognitionRejection>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,
}

impl SpeakerAnalysisRequest {
    pub fn new(
        audio_path: impl Into<PathBuf>,
        provider: impl Into<String>,
        model_id: Option<String>,
        session_id: impl Into<String>,
        audio_segment_id: i64,
    ) -> Self {
        Self {
            audio_path: audio_path.into(),
            provider: provider.into(),
            model_id,
            session_id: session_id.into(),
            audio_segment_id,
            recognize_people: false,
            enrolled_people: Vec::new(),
            rejected_people: Vec::new(),
            options: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonEnrollment {
    pub person_id: i64,
    pub display_name: String,
    #[serde(with = "serde_bytes")]
    pub embedding: Vec<u8>,
    pub embedding_model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonRecognitionRejection {
    pub person_id: i64,
    #[serde(with = "serde_bytes")]
    pub embedding: Vec<u8>,
    pub embedding_model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerRecognitionSuggestion {
    pub person_id: i64,
    pub display_name: String,
    pub confidence: RecognitionConfidence,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerCluster {
    pub provider_cluster_id: String,
    pub stable_label: String,
    #[serde(with = "serde_bytes")]
    pub embedding: Vec<u8>,
    pub embedding_model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<SpeakerRecognitionSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerTurn {
    pub provider_cluster_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_text: Option<String>,
    #[serde(default)]
    pub overlaps: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisMetadata {
    pub provider: String,
    pub model_id: Option<String>,
    pub session_id: String,
    pub audio_segment_id: i64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provenance: BTreeMap<String, serde_json::Value>,
}

impl SpeakerAnalysisMetadata {
    pub fn from_request(request: &SpeakerAnalysisRequest) -> Self {
        Self {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
            session_id: request.session_id.clone(),
            audio_segment_id: request.audio_segment_id,
            provenance: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisOutput {
    pub clusters: Vec<SpeakerCluster>,
    pub turns: Vec<SpeakerTurn>,
    pub metadata: SpeakerAnalysisMetadata,
    pub provider_version: Option<String>,
}

impl SpeakerAnalysisOutput {
    pub fn new(metadata: SpeakerAnalysisMetadata) -> Self {
        Self {
            clusters: Vec::new(),
            turns: Vec::new(),
            metadata,
            provider_version: None,
        }
    }

    pub fn with_provider_version(mut self, version: impl Into<String>) -> Self {
        self.provider_version = Some(version.into());
        self
    }

    pub fn structured_payload_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[async_trait]
pub trait SpeakerAnalysisProvider: Send + Sync {
    fn provider(&self) -> &'static str;

    async fn analyze(
        &self,
        request: SpeakerAnalysisRequest,
    ) -> SpeakerAnalysisResult<SpeakerAnalysisOutput>;
}
