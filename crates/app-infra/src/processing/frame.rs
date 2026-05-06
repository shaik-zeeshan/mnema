use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrameEquivalenceStatus {
    Ready,
    Quarantined,
}

impl FrameEquivalenceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Quarantined => "quarantined",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "quarantined" => Some(Self::Quarantined),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameEquivalence {
    pub hint: Option<String>,
    #[serde(with = "serde_bytes")]
    pub proof: Option<Vec<u8>>,
    pub version: Option<i64>,
    pub status: Option<FrameEquivalenceStatus>,
    pub error: Option<String>,
}

impl FrameEquivalence {
    pub fn ready(hint: impl Into<String>, proof: Vec<u8>, version: i64) -> Self {
        Self {
            hint: Some(hint.into()),
            proof: Some(proof),
            version: Some(version),
            status: Some(FrameEquivalenceStatus::Ready),
            error: None,
        }
    }

    pub fn quarantined(error: impl Into<String>) -> Self {
        Self {
            hint: None,
            proof: None,
            version: None,
            status: Some(FrameEquivalenceStatus::Quarantined),
            error: Some(error.into()),
        }
    }

    pub fn ready_parts(&self) -> Option<(&str, &[u8], i64)> {
        let status = self.status.as_ref()?;
        if *status != FrameEquivalenceStatus::Ready {
            return None;
        }

        Some((self.hint.as_deref()?, self.proof.as_deref()?, self.version?))
    }

    pub fn is_quarantined(&self) -> bool {
        self.status == Some(FrameEquivalenceStatus::Quarantined)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub id: i64,
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub equivalence: FrameEquivalence,
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
    pub equivalence: FrameEquivalence,
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
            equivalence: FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
        }
    }

    pub fn with_dimensions(mut self, width: i64, height: i64) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    pub fn with_equivalence(mut self, equivalence: FrameEquivalence) -> Self {
        self.equivalence = equivalence;
        self
    }
}
