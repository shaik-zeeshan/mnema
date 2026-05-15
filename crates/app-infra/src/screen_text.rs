use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapturedScreenTextSource {
    Accessibility,
}

impl CapturedScreenTextSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accessibility => "accessibility",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "accessibility" => Some(Self::Accessibility),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewCapturedScreenText {
    pub source: CapturedScreenTextSource,
    pub result_text: String,
    pub structured_payload_json: Option<String>,
    pub captured_at_unix_ms: i64,
    pub source_app_bundle_id: Option<String>,
    pub source_app_name: Option<String>,
    pub source_window_title: Option<String>,
    pub source_window_id: Option<i64>,
    pub snapshot_age_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapturedScreenText {
    pub id: i64,
    pub frame_id: i64,
    pub source: CapturedScreenTextSource,
    pub result_text: String,
    pub structured_payload_json: Option<String>,
    pub captured_at_unix_ms: i64,
    pub source_app_bundle_id: Option<String>,
    pub source_app_name: Option<String>,
    pub source_window_title: Option<String>,
    pub source_window_id: Option<i64>,
    pub snapshot_age_ms: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedScreenTextSource {
    Accessibility,
    Ocr,
    EquivalentAccessibility,
    EquivalentOcr,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedScreenText {
    pub text: Option<String>,
    pub source: ResolvedScreenTextSource,
    pub frame_id: Option<i64>,
}

impl ResolvedScreenText {
    pub fn none() -> Self {
        Self {
            text: None,
            source: ResolvedScreenTextSource::None,
            frame_id: None,
        }
    }
}
