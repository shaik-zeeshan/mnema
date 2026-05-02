use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub id: i64,
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameSummary {
    pub id: i64,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewFrame {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
}

impl NewFrame {
    pub fn new(
        session_id: impl Into<String>,
        file_path: impl Into<String>,
        captured_at: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            file_path: file_path.into(),
            captured_at: captured_at.into(),
            width: None,
            height: None,
            content_fingerprint: None,
        }
    }

    pub fn with_dimensions(mut self, width: i64, height: i64) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    pub fn with_content_fingerprint(mut self, content_fingerprint: impl Into<String>) -> Self {
        self.content_fingerprint = Some(content_fingerprint.into());
        self
    }
}
