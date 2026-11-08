//! The commands' request payloads, as the frontend sends them.

use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppJobRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugInsertFrameAndEnqueueProcessingJobRequest {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub processor: String,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugInsertFrameAndEnqueueOcrRequest {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessCapturedFrameOcrRequest {
    pub frame_id: i64,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessAudioSegmentTranscriptionRequest {
    pub audio_segment_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNearestEarlierEquivalentFrameRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetEarliestEarlierEquivalentFrameRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTimelineWindowAroundFrameRequest {
    pub frame_id: i64,
    pub newer_limit: u32,
    pub older_limit: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAudioSegmentMediaRequest {
    pub audio_segment_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAudioSegmentWaveformPeaksRequest {
    pub audio_segment_id: i64,
    pub bucket_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAudioSegmentRequest {
    pub audio_segment_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSpeakerTurnsRequest {
    pub audio_segment_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSpeakerClustersRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePersonProfileRequest {
    pub display_name: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePersonProfileRequest {
    pub person_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameSpeakerClusterRequest {
    pub cluster_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkSpeakerClusterRequest {
    pub cluster_id: i64,
    pub person_id: i64,
    pub add_embedding: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerClusterRequest {
    pub cluster_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmSpeakerSuggestionRequest {
    pub cluster_id: i64,
    pub add_embedding: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeSpeakerClustersRequest {
    pub source_cluster_id: i64,
    pub target_cluster_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveSpeakerTurnRequest {
    pub turn_id: i64,
    pub target_cluster_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFramesRequest {
    pub session_id: Option<String>,
    pub before_id: Option<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameCapturedAtRangeRequest {
    pub captured_at_start: String,
    pub captured_at_end: String,
    /// Journal only: drop lock-screen frames so their span reads as Away.
    #[serde(default)]
    pub exclude_lock_screen: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAudioSegmentsRequest {
    pub captured_at_start: String,
    pub captured_at_end: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProcessingJobRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingJobsRequest {
    pub subject_type: String,
    pub subject_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProcessingResultRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingResultsRequest {
    pub subject_type: String,
    pub subject_id: i64,
}

impl DebugInsertFrameAndEnqueueProcessingJobRequest {
    pub(super) fn into_parts(self) -> (::app_infra::NewFrame, String, Option<String>) {
        let Self {
            session_id,
            file_path,
            captured_at,
            width,
            height,
            processor,
            payload_json,
        } = self;

        let mut frame = ::app_infra::NewFrame::new(session_id, file_path, captured_at);

        if let (Some(width), Some(height)) = (width, height) {
            frame = frame.with_dimensions(width, height);
        }

        (frame, processor, payload_json)
    }
}

impl From<DebugInsertFrameAndEnqueueOcrRequest> for DebugInsertFrameAndEnqueueProcessingJobRequest {
    fn from(request: DebugInsertFrameAndEnqueueOcrRequest) -> Self {
        Self {
            session_id: request.session_id,
            file_path: request.file_path,
            captured_at: request.captured_at,
            width: request.width,
            height: request.height,
            processor: ::app_infra::OCR_PROCESSOR.to_string(),
            payload_json: request.payload_json,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_insert_frame_processing_request_maps_optional_dimensions() {
        let request = DebugInsertFrameAndEnqueueProcessingJobRequest {
            session_id: "session-a".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-04-12T10:00:00Z".to_string(),
            width: Some(1280),
            height: Some(720),
            processor: "custom-processor".to_string(),
            payload_json: Some("{\"language\":\"eng\"}".to_string()),
        }
        .into_parts();

        assert_eq!(request.0.session_id, "session-a");
        assert_eq!(request.0.file_path, "/tmp/frame.png");
        assert_eq!(request.0.width, Some(1280));
        assert_eq!(request.0.height, Some(720));
        assert_eq!(request.1, "custom-processor");
        assert_eq!(request.2.as_deref(), Some("{\"language\":\"eng\"}"));
    }

    #[test]
    fn debug_insert_frame_processing_request_ignores_partial_dimensions() {
        let request = DebugInsertFrameAndEnqueueProcessingJobRequest {
            session_id: "session-b".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-04-12T10:00:00Z".to_string(),
            width: Some(1280),
            height: None,
            processor: "custom-processor".to_string(),
            payload_json: None,
        }
        .into_parts();

        assert_eq!(request.0.width, None);
        assert_eq!(request.0.height, None);
        assert_eq!(request.1, "custom-processor");
        assert_eq!(request.2, None);
    }
    #[test]
    fn debug_insert_frame_ocr_request_wraps_generic_processing_request() {
        let request = DebugInsertFrameAndEnqueueProcessingJobRequest::from(
            DebugInsertFrameAndEnqueueOcrRequest {
                session_id: "session-ocr".to_string(),
                file_path: "/tmp/frame-ocr.png".to_string(),
                captured_at: "2026-04-12T10:00:00Z".to_string(),
                width: Some(1920),
                height: Some(1080),
                payload_json: Some("{\"language\":\"eng\"}".to_string()),
            },
        )
        .into_parts();

        assert_eq!(request.0.session_id, "session-ocr");
        assert_eq!(request.0.file_path, "/tmp/frame-ocr.png");
        assert_eq!(request.0.width, Some(1920));
        assert_eq!(request.0.height, Some(1080));
        assert_eq!(request.1, ::app_infra::OCR_PROCESSOR);
        assert_eq!(request.2.as_deref(), Some("{\"language\":\"eng\"}"));
    }
}
