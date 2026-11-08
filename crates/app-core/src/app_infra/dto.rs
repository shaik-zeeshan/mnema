//! The commands' response payloads and their mapping from app-infra rows.

use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppJobDto {
    pub id: i64,
    pub kind: String,
    pub status: ::app_infra::BackgroundJobStatus,
    pub payload_json: Option<String>,
    pub result_text: Option<String>,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameDto {
    pub id: i64,
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    /// Guarded host+path of the page captured in this frame (read-time
    /// sanitized + secret-redacted via the broker URL guard). The raw captured
    /// `browser_url` is local-only and is NEVER placed here — only the guarded
    /// form crosses to the frontend. `None` when the frame had no URL or it was
    /// not http(s).
    pub url: Option<String>,
    pub equivalence_hint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureRequest {
    pub query: String,
    pub frame_limit: Option<u32>,
    pub frame_offset: Option<u32>,
    pub audio_limit: Option<u32>,
    pub audio_offset: Option<u32>,
    pub snapshot_document_id: Option<i64>,
    pub refinements: Option<::app_infra::SearchCaptureRefinements>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureResponseDto {
    pub normalized_query: String,
    pub snapshot_document_id: i64,
    pub frames: Vec<FrameSearchResultDto>,
    pub audio: Vec<AudioSearchResultDto>,
    pub has_more_frames: bool,
    pub has_more_audio: bool,
    pub applied_refinements: ::app_infra::SearchCaptureRefinements,
    pub residual_query: String,
    pub parse_errors: Vec<::app_infra::SearchParseError>,
    /// Why results may be partial (see `::app_infra::SearchCaptureResponse`).
    pub index_empty: bool,
    pub semantic_degraded: bool,
    pub untranscribed_segments_in_range: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSearchResultDto {
    /// Lower is better; the frontend merges frames and audio by it (ADR 0036:
    /// no calibration — see `::app_infra::FrameSearchResult::rank`).
    pub rank: f64,
    pub group_key: String,
    pub representative_frame: FrameDto,
    pub group_start_at: String,
    pub group_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    /// Guarded host+path of the page behind this result (read-time sanitized +
    /// secret-redacted via the broker URL guard). The raw captured `browser_url`
    /// is local-only and is NEVER placed here — only the guarded form crosses to
    /// the frontend. `None` when the frame had no URL or it was not http(s).
    pub url: Option<String>,
    pub thumbnail_frame_id: i64,
    pub text_source_kind: String,
    pub secret_redaction_count: u32,
    pub has_secret_redactions: bool,
    /// A meaning-only **Semantic Search** hit: the snippet is a leading
    /// `body_text` excerpt the frontend tags "found by meaning".
    pub found_by_meaning: bool,
    /// Where the query's words sit on `representative_frame` (normalised,
    /// top-left origin). Filled by `search_capture`, never by the `From`.
    pub match_boxes: Vec<::app_infra::MatchBox>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSearchResultDto {
    /// Lower is better (see [`FrameSearchResultDto::rank`]).
    pub rank: f64,
    pub group_key: String,
    pub audio_segment: AudioSegmentDto,
    pub source_kind: ::app_infra::AudioSegmentSourceKind,
    pub span_start_ms: u64,
    pub span_end_ms: u64,
    pub absolute_start_at: String,
    pub absolute_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub aligned_frame: Option<FrameDto>,
    pub secret_redaction_count: u32,
    pub has_secret_redactions: bool,
    /// A meaning-only **Semantic Search** hit (see [`FrameSearchResultDto`]).
    pub found_by_meaning: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSummaryDto {
    pub id: i64,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusedFrameWindowDto {
    pub frames: Vec<FrameDto>,
    pub target_index: usize,
    pub has_newer: bool,
    pub has_older: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegmentMediaDto {
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJobDto {
    pub id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub status: ::app_infra::ProcessingJobStatus,
    pub attempt_count: i64,
    pub failure_count: i64,
    pub payload_json: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub queued_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// The job's model is locked (downloading, absent, or being deleted), so the claim predicate
    /// skips it. `status = "queued"` **and** `modelLocked` is the "Preparing" state: the job is
    /// waiting for its model, not queued behind other work and not burning failure attempts.
    /// `false` unless the caller resolved it — only the job-listing commands do.
    pub model_locked: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingResultDto {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameProcessingJobDto {
    pub frame: FrameDto,
    pub job: ProcessingJobDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedFrameReprocessingResultDto {
    pub outcome: ::app_infra::CapturedFrameReprocessingOutcome,
    pub job: ProcessingJobDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegmentTranscriptionReprocessingResultDto {
    pub outcome: ::app_infra::AudioSegmentTranscriptionReprocessingOutcome,
    pub job: ProcessingJobDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegmentSpeakerAnalysisReprocessingResultDto {
    pub outcome: ::app_infra::AudioSegmentSpeakerAnalysisReprocessingOutcome,
    pub job: ProcessingJobDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioSpeechActivityReprocessingResultDto {
    pub outcome: ::app_infra::SystemAudioSpeechActivityReprocessingOutcome,
    pub job: ProcessingJobDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegmentDto {
    pub id: i64,
    pub source_kind: ::app_infra::AudioSegmentSourceKind,
    pub source_session_id: String,
    pub segment_index: i64,
    pub file_path: String,
    pub started_at: String,
    pub ended_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerTurnDto {
    pub id: i64,
    pub audio_segment_id: i64,
    pub session_id: String,
    pub cluster_id: i64,
    pub segment_cluster_id: Option<i64>,
    pub provider_cluster_id: String,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub person_link_auto: bool,
    pub suggested_person_id: Option<i64>,
    pub recognition_confidence: Option<String>,
    pub recognition_score: Option<f32>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub transcript_text: Option<String>,
    pub overlaps: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonProfileDto {
    pub id: i64,
    pub display_name: String,
    pub notes: Option<String>,
    pub embedding_count: i64,
    pub is_account_owner: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerClusterDto {
    pub id: i64,
    pub session_id: String,
    pub provider: String,
    pub model_id: Option<String>,
    pub provider_cluster_id: String,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub person_link_auto: bool,
    pub suggested_person_id: Option<i64>,
    pub recognition_confidence: Option<String>,
    pub recognition_score: Option<f32>,
    pub suggested_merge_target_cluster_id: Option<i64>,
    pub suggested_merge_score: Option<f32>,
}

impl From<::app_infra::BackgroundJob> for AppJobDto {
    fn from(job: ::app_infra::BackgroundJob) -> Self {
        Self {
            id: job.id,
            kind: job.kind,
            status: job.status,
            payload_json: job.payload_json,
            result_text: job.result_text,
            attempt_count: job.attempt_count,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }
}

impl From<::app_infra::Frame> for FrameDto {
    fn from(frame: ::app_infra::Frame) -> Self {
        // Read the GUARDED host+path from the raw captured `browser_url` BEFORE
        // the snapshot is moved below. The raw URL never lands on the DTO — only
        // the guard's sanitized, secret-redacted host+path (or `None`).
        let url = frame
            .metadata_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.browser_url.as_deref())
            .and_then(::app_infra::guard_browser_url);

        let (app_bundle_id, app_name, window_title) = frame
            .metadata_snapshot
            .map(|metadata| {
                (
                    metadata.app_bundle_id,
                    metadata.app_name,
                    metadata.window_title,
                )
            })
            .unwrap_or((None, None, None));

        Self {
            id: frame.id,
            session_id: frame.session_id,
            file_path: frame.file_path,
            captured_at: frame.captured_at,
            width: frame.width,
            height: frame.height,
            app_bundle_id,
            app_name,
            window_title,
            url,
            equivalence_hint: frame.equivalence.hint,
            created_at: frame.created_at,
            updated_at: frame.updated_at,
        }
    }
}

impl From<::app_infra::FrameSummary> for FrameSummaryDto {
    fn from(frame: ::app_infra::FrameSummary) -> Self {
        Self {
            id: frame.id,
            captured_at: frame.captured_at,
        }
    }
}

impl From<::app_infra::FocusedFrameWindow> for FocusedFrameWindowDto {
    fn from(window: ::app_infra::FocusedFrameWindow) -> Self {
        Self {
            frames: window.frames.into_iter().map(FrameDto::from).collect(),
            target_index: window.target_index,
            has_newer: window.has_newer,
            has_older: window.has_older,
        }
    }
}

impl From<::app_infra::FrameSearchResult> for FrameSearchResultDto {
    fn from(result: ::app_infra::FrameSearchResult) -> Self {
        Self {
            rank: result.rank,
            group_key: result.group_key,
            representative_frame: FrameDto::from(result.representative_frame),
            group_start_at: result.group_start_at,
            group_end_at: result.group_end_at,
            match_count: result.match_count,
            snippet: result.snippet,
            app_bundle_id: result.app_bundle_id,
            app_name: result.app_name,
            window_title: result.window_title,
            // Guard the raw captured `browser_url` at the boundary: only the
            // sanitized host+path crosses to the frontend; the raw URL never does.
            url: result
                .browser_url
                .as_deref()
                .and_then(::app_infra::guard_browser_url),
            thumbnail_frame_id: result.thumbnail_frame_id,
            text_source_kind: result.text_source_kind,
            secret_redaction_count: result.secret_redaction_count,
            has_secret_redactions: result.has_secret_redactions,
            found_by_meaning: result.found_by_meaning,
            match_boxes: Vec::new(),
        }
    }
}

impl From<::app_infra::AudioSearchResult> for AudioSearchResultDto {
    fn from(result: ::app_infra::AudioSearchResult) -> Self {
        Self {
            rank: result.rank,
            group_key: result.group_key,
            audio_segment: AudioSegmentDto::from(result.audio_segment),
            source_kind: result.source_kind,
            span_start_ms: result.span_start_ms,
            span_end_ms: result.span_end_ms,
            absolute_start_at: result.absolute_start_at,
            absolute_end_at: result.absolute_end_at,
            match_count: result.match_count,
            snippet: result.snippet,
            aligned_frame: result.aligned_frame.map(FrameDto::from),
            secret_redaction_count: result.secret_redaction_count,
            has_secret_redactions: result.has_secret_redactions,
            found_by_meaning: result.found_by_meaning,
        }
    }
}

impl From<::app_infra::SearchCaptureResponse> for SearchCaptureResponseDto {
    fn from(response: ::app_infra::SearchCaptureResponse) -> Self {
        Self {
            normalized_query: response.normalized_query,
            snapshot_document_id: response.snapshot_document_id,
            frames: response
                .frames
                .into_iter()
                .map(FrameSearchResultDto::from)
                .collect(),
            audio: response
                .audio
                .into_iter()
                .map(AudioSearchResultDto::from)
                .collect(),
            has_more_frames: response.has_more_frames,
            has_more_audio: response.has_more_audio,
            applied_refinements: response.applied_refinements,
            residual_query: response.residual_query,
            parse_errors: response.parse_errors,
            index_empty: response.index_empty,
            semantic_degraded: response.semantic_degraded,
            untranscribed_segments_in_range: response.untranscribed_segments_in_range,
        }
    }
}

impl From<::app_infra::ProcessingJob> for ProcessingJobDto {
    fn from(job: ::app_infra::ProcessingJob) -> Self {
        Self {
            id: job.id,
            subject_type: job.subject_type,
            subject_id: job.subject_id,
            processor: job.processor,
            status: job.status,
            attempt_count: job.attempt_count,
            failure_count: job.failure_count,
            payload_json: job.payload_json,
            last_error: job.last_error,
            created_at: job.created_at,
            queued_at: job.queued_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            model_locked: false,
        }
    }
}

impl ProcessingJobDto {
    pub(super) fn from_job(
        job: ::app_infra::ProcessingJob,
        locked_model_keys: &BTreeSet<(String, String)>,
    ) -> Self {
        let model_locked = ::app_infra::processing_job_model_is_locked(&job, locked_model_keys);
        Self {
            model_locked,
            ..Self::from(job)
        }
    }
}

impl From<::app_infra::ProcessingResult> for ProcessingResultDto {
    fn from(result: ::app_infra::ProcessingResult) -> Self {
        Self {
            id: result.id,
            job_id: result.job_id,
            subject_type: result.subject_type,
            subject_id: result.subject_id,
            processor: result.processor,
            result_text: result.result_text,
            structured_payload_json: result.structured_payload_json,
            processor_version: result.processor_version,
            created_at: result.created_at,
        }
    }
}

impl From<::app_infra::SpeakerTurnView> for SpeakerTurnDto {
    fn from(value: ::app_infra::SpeakerTurnView) -> Self {
        Self {
            id: value.id,
            audio_segment_id: value.audio_segment_id,
            session_id: value.session_id,
            cluster_id: value.cluster_id,
            segment_cluster_id: value.segment_cluster_id,
            provider_cluster_id: value.provider_cluster_id,
            speaker_label: value.speaker_label,
            person_id: value.person_id,
            person_link_auto: value.person_link_auto,
            suggested_person_id: value.suggested_person_id,
            recognition_confidence: value.recognition_confidence,
            recognition_score: value.recognition_score,
            start_ms: value.start_ms,
            end_ms: value.end_ms,
            transcript_text: value.transcript_text,
            overlaps: value.overlaps,
        }
    }
}

impl From<::app_infra::PersonProfile> for PersonProfileDto {
    fn from(value: ::app_infra::PersonProfile) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            notes: value.notes,
            embedding_count: value.embedding_count,
            is_account_owner: value.is_account_owner,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<::app_infra::SpeakerClusterView> for SpeakerClusterDto {
    fn from(value: ::app_infra::SpeakerClusterView) -> Self {
        Self {
            id: value.id,
            session_id: value.session_id,
            provider: value.provider,
            model_id: value.model_id,
            provider_cluster_id: value.provider_cluster_id,
            speaker_label: value.speaker_label,
            person_id: value.person_id,
            person_link_auto: value.person_link_auto,
            suggested_person_id: value.suggested_person_id,
            recognition_confidence: value.recognition_confidence,
            recognition_score: value.recognition_score,
            suggested_merge_target_cluster_id: value.suggested_merge_target_cluster_id,
            suggested_merge_score: value.suggested_merge_score,
        }
    }
}

impl From<::app_infra::FrameProcessingJob> for FrameProcessingJobDto {
    fn from(value: ::app_infra::FrameProcessingJob) -> Self {
        Self {
            frame: value.frame.into(),
            job: value.job.into(),
        }
    }
}

impl From<::app_infra::AudioSegmentSpeakerAnalysisReprocessingResult>
    for AudioSegmentSpeakerAnalysisReprocessingResultDto
{
    fn from(value: ::app_infra::AudioSegmentSpeakerAnalysisReprocessingResult) -> Self {
        Self {
            outcome: value.outcome,
            job: value.job.into(),
        }
    }
}

impl From<::app_infra::SystemAudioSpeechActivityReprocessingResult>
    for SystemAudioSpeechActivityReprocessingResultDto
{
    fn from(value: ::app_infra::SystemAudioSpeechActivityReprocessingResult) -> Self {
        Self {
            outcome: value.outcome,
            job: value.job.into(),
        }
    }
}

impl From<::app_infra::CapturedFrameReprocessingResult> for CapturedFrameReprocessingResultDto {
    fn from(value: ::app_infra::CapturedFrameReprocessingResult) -> Self {
        Self {
            outcome: value.outcome,
            job: value.job.into(),
        }
    }
}

impl From<::app_infra::AudioSegmentTranscriptionReprocessingResult>
    for AudioSegmentTranscriptionReprocessingResultDto
{
    fn from(value: ::app_infra::AudioSegmentTranscriptionReprocessingResult) -> Self {
        Self {
            outcome: value.outcome,
            job: value.job.into(),
        }
    }
}

impl From<::app_infra::AudioSegment> for AudioSegmentDto {
    fn from(segment: ::app_infra::AudioSegment) -> Self {
        Self {
            id: segment.id,
            source_kind: segment.source_kind,
            source_session_id: segment.source_session_id,
            segment_index: segment.segment_index,
            file_path: segment.file_path,
            started_at: segment.started_at,
            ended_at: segment.ended_at,
            created_at: segment.created_at,
            updated_at: segment.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_dto_exposes_only_app_metadata_for_bulk_timeline_payloads() {
        let frame = ::app_infra::Frame {
            id: 7,
            session_id: "session-metadata".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-05-12T10:00:00Z".to_string(),
            width: Some(1440),
            height: Some(900),
            equivalence: ::app_infra::FrameEquivalence {
                hint: Some("hint".to_string()),
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: "2026-05-12T10:00:00Z".to_string(),
            updated_at: "2026-05-12T10:00:00Z".to_string(),
            metadata_snapshot: Some(capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.example.Browser".to_string()),
                app_name: Some("Browser".to_string()),
                window_title: Some("Sensitive Project".to_string()),
                window_id: None,
                browser_url: Some("https://example.com/private".to_string()),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
        };

        let value = serde_json::to_value(FrameDto::from(frame)).expect("dto should serialize");

        assert!(value.get("metadata").is_none());
        assert_eq!(
            value.get("appBundleId").and_then(|value| value.as_str()),
            Some("com.example.Browser")
        );
        assert_eq!(
            value.get("appName").and_then(|value| value.as_str()),
            Some("Browser")
        );
        // The window title is a curated, flattened field on the DTO (not the raw
        // metadata snapshot), so it is exposed for bulk timeline payloads.
        assert_eq!(
            value.get("windowTitle").and_then(|value| value.as_str()),
            Some("Sensitive Project")
        );
        // The browser URL is never flattened onto the DTO, so it must not leak.
        assert!(!value.to_string().contains("https://example.com/private"));
    }

    #[test]
    fn frame_search_result_dto_exposes_app_bundle_id() {
        let frame = ::app_infra::Frame {
            id: 9,
            session_id: "session-search-metadata".to_string(),
            file_path: "/tmp/search-frame.png".to_string(),
            captured_at: "2026-05-12T10:00:00Z".to_string(),
            width: Some(1440),
            height: Some(900),
            equivalence: ::app_infra::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: "2026-05-12T10:00:00Z".to_string(),
            updated_at: "2026-05-12T10:00:00Z".to_string(),
            metadata_snapshot: None,
        };
        let result = ::app_infra::FrameSearchResult {
            rank: -1.0,
            group_key: "frame:9".to_string(),
            representative_frame: frame,
            group_start_at: "2026-05-12T10:00:00Z".to_string(),
            group_end_at: "2026-05-12T10:00:00Z".to_string(),
            match_count: 1,
            snippet: "<mark>match</mark>".to_string(),
            app_bundle_id: Some("com.example.Search".to_string()),
            app_name: Some("Search App".to_string()),
            window_title: Some("Search Window".to_string()),
            browser_url: None,
            thumbnail_frame_id: 9,
            text_source_kind: "ocr".to_string(),
            secret_redaction_count: 0,
            has_secret_redactions: false,
            found_by_meaning: false,
        };

        let value =
            serde_json::to_value(FrameSearchResultDto::from(result)).expect("dto should serialize");

        assert_eq!(
            value.get("appBundleId").and_then(|value| value.as_str()),
            Some("com.example.Search")
        );
        assert_eq!(value["rank"], serde_json::json!(-1.0));
        assert_eq!(value["matchBoxes"], serde_json::json!([]));
        let match_box = ::app_infra::MatchBox {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.05,
            kind: ::app_infra::MatchBoxKind::Meaning,
        };
        assert_eq!(
            serde_json::to_value(match_box).unwrap(),
            serde_json::json!({"x": 0.1, "y": 0.2, "w": 0.3, "h": 0.05, "kind": "meaning"})
        );
    }
}
