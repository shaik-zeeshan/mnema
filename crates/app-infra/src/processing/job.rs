use serde::{Deserialize, Serialize};

pub const FRAME_SUBJECT_TYPE: &str = "frame";
pub const AUDIO_SEGMENT_SUBJECT_TYPE: &str = "audio_segment";
pub const OCR_PROCESSOR: &str = "ocr";
pub const AUDIO_TRANSCRIPTION_PROCESSOR: &str = "audio_transcription";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingSubject {
    pub subject_type: String,
    pub subject_id: i64,
}

impl ProcessingSubject {
    pub fn new(subject_type: impl Into<String>, subject_id: i64) -> Self {
        Self {
            subject_type: subject_type.into(),
            subject_id,
        }
    }

    pub fn frame(frame_id: i64) -> Self {
        Self::new(FRAME_SUBJECT_TYPE, frame_id)
    }

    pub fn audio_segment(audio_segment_id: i64) -> Self {
        Self::new(AUDIO_SEGMENT_SUBJECT_TYPE, audio_segment_id)
    }

    pub fn subject_type(&self) -> &str {
        &self.subject_type
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl ProcessingJobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_str(value: &str) -> crate::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(crate::AppInfraError::InvalidProcessingJobStatus(
                other.to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJob {
    pub id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub status: ProcessingJobStatus,
    pub attempt_count: i64,
    pub payload_json: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJobDraft {
    pub subject: ProcessingSubject,
    pub processor: String,
    pub payload_json: Option<String>,
}

impl ProcessingJobDraft {
    pub fn new(subject: ProcessingSubject, processor: impl Into<String>) -> Self {
        Self {
            subject,
            processor: processor.into(),
            payload_json: None,
        }
    }

    pub fn for_frame_ocr(frame_id: i64) -> Self {
        Self::new(ProcessingSubject::frame(frame_id), OCR_PROCESSOR)
    }

    pub fn for_audio_segment_transcription(audio_segment_id: i64) -> Self {
        Self::new(
            ProcessingSubject::audio_segment(audio_segment_id),
            AUDIO_TRANSCRIPTION_PROCESSOR,
        )
    }

    pub fn with_payload_json(mut self, payload_json: impl Into<String>) -> Self {
        self.payload_json = Some(payload_json.into());
        self
    }
}
