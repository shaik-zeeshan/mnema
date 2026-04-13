use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingResult {
    pub id: i64,
    pub job_id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub result_text: Option<String>,
    pub structured_payload_json: Option<String>,
    pub processor_version: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingResultDraft {
    pub result_text: Option<String>,
    pub structured_payload_json: Option<String>,
    pub processor_version: Option<String>,
}

impl ProcessingResultDraft {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_result_text(mut self, result_text: impl Into<String>) -> Self {
        self.result_text = Some(result_text.into());
        self
    }

    pub fn with_structured_payload_json(mut self, payload_json: impl Into<String>) -> Self {
        self.structured_payload_json = Some(payload_json.into());
        self
    }

    pub fn with_optional_structured_payload_json(mut self, payload_json: Option<String>) -> Self {
        self.structured_payload_json = payload_json;
        self
    }

    pub fn with_processor_version(mut self, processor_version: impl Into<String>) -> Self {
        self.processor_version = Some(processor_version.into());
        self
    }

    pub fn with_optional_processor_version(mut self, processor_version: Option<String>) -> Self {
        self.processor_version = processor_version;
        self
    }
}
