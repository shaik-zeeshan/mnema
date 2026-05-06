use std::{
    collections::HashMap,
    fs,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(test)]
use std::sync::OnceLock;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use capture_screen::ScreenFrameArtifact;
use capture_types::{OcrRecognitionMode, OcrSettings};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tauri::{Manager, path::BaseDirectory};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::oneshot;

pub type AppInfraState = Arc<::app_infra::AppInfra>;
pub type FramePreviewCacheState = Mutex<FramePreviewState>;

const APP_INFRA_LOCK_FILE_NAME: &str = ".app-infra.lock";
const FRAME_PREVIEW_CACHE_MAX_ENTRIES: usize = 256;
const FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL: Duration = Duration::from_secs(15);
const FRAME_PREVIEW_EXACT_MISS_LOG_THRESHOLD_MS: f64 = 5.0;
const GENERATED_FRAME_PREVIEW_CACHE_DIR: &str = "frame-previews";
const GENERATED_FRAME_PREVIEW_CACHE_MAX_FILES: usize = 512;
const GENERATED_FRAME_PREVIEW_CACHE_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24);
const PROCESSING_WORKER_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const PROCESSING_WORKER_ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const HIDDEN_SEGMENT_WORKSPACE_REPAIR_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameIndexSidecarConversionResult {
    converted_count: u64,
    skipped_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct OcrJobPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recognition_mode: Option<OcrRecognitionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language_correction: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedFramePreview {
    preview: FramePreviewDto,
    cached_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedVideoPreviewFailure {
    message: String,
    cached_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedFramePreviewOffset {
    video_offset_ms: u64,
    exact_match: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyScreenSegmentFrameIndexEntry {
    captured_at_unix_ms: u64,
    frame_index: u64,
    #[allow(dead_code)]
    #[serde(default)]
    artifact_file_name: Option<String>,
    video_offset_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyScreenSegmentFrameIndex {
    version: u32,
    entries: Vec<LegacyScreenSegmentFrameIndexEntry>,
}

#[derive(Debug, Default)]
pub struct FramePreviewState {
    cache: FramePreviewCache,
    in_flight: HashMap<i64, Vec<oneshot::Sender<Result<Option<FramePreviewDto>, String>>>>,
    video_in_flight: HashMap<PathBuf, Vec<oneshot::Sender<Result<(), String>>>>,
}

#[derive(Debug, Default)]
pub struct FramePreviewCache {
    entries: HashMap<i64, CachedFramePreview>,
    video_failures: HashMap<PathBuf, CachedVideoPreviewFailure>,
}

impl FramePreviewState {
    fn get(&mut self, frame_id: i64, ttl: Duration, now: Instant) -> Option<FramePreviewDto> {
        self.cache.get(frame_id, ttl, now)
    }

    fn insert(&mut self, frame_id: i64, preview: FramePreviewDto, ttl: Duration, now: Instant) {
        self.cache.insert(frame_id, preview, ttl, now);
    }

    fn clear(&mut self) {
        self.cache.clear();
        self.in_flight.clear();
        self.video_in_flight.clear();
    }

    fn get_video_failure(&mut self, video_path: &Path, now: Instant) -> Option<String> {
        self.cache.get_video_failure(video_path, now)
    }

    fn insert_video_failure(&mut self, video_path: &Path, message: String, now: Instant) {
        self.cache.insert_video_failure(video_path, message, now);
    }

    fn begin_request(
        &mut self,
        frame_id: i64,
    ) -> Result<(), oneshot::Receiver<Result<Option<FramePreviewDto>, String>>> {
        if let Some(waiters) = self.in_flight.get_mut(&frame_id) {
            let (tx, rx) = oneshot::channel();
            waiters.push(tx);
            return Err(rx);
        }

        self.in_flight.insert(frame_id, Vec::new());
        Ok(())
    }

    fn finish_request(&mut self, frame_id: i64, result: Result<Option<FramePreviewDto>, String>) {
        let waiters = self.in_flight.remove(&frame_id).unwrap_or_default();
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    fn begin_video_request(
        &mut self,
        video_path: &Path,
    ) -> Result<(), oneshot::Receiver<Result<(), String>>> {
        if let Some(waiters) = self.video_in_flight.get_mut(video_path) {
            let (tx, rx) = oneshot::channel();
            waiters.push(tx);
            return Err(rx);
        }

        self.video_in_flight.insert(video_path.to_path_buf(), Vec::new());
        Ok(())
    }

    fn finish_video_request(&mut self, video_path: &Path, result: Result<(), String>) {
        let waiters = self.video_in_flight.remove(video_path).unwrap_or_default();
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    #[cfg(test)]
    fn video_in_flight_len(&self) -> usize {
        self.video_in_flight.len()
    }
}

impl FramePreviewCache {
    fn get(&mut self, frame_id: i64, ttl: Duration, now: Instant) -> Option<FramePreviewDto> {
        self.evict_expired(ttl, now);
        let preview = self.entries.get(&frame_id).map(|entry| entry.preview.clone())?;
        if !Path::new(&preview.file_path).is_file() {
            self.entries.remove(&frame_id);
            return None;
        }
        Some(preview)
    }

    fn insert(&mut self, frame_id: i64, preview: FramePreviewDto, ttl: Duration, now: Instant) {
        self.evict_expired(ttl, now);
        self.entries.insert(
            frame_id,
            CachedFramePreview {
                preview,
                cached_at: now,
            },
        );
        self.evict_oldest_excess_entries();
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.video_failures.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn evict_expired(&mut self, ttl: Duration, now: Instant) {
        self.entries
            .retain(|_, entry| now.duration_since(entry.cached_at) < ttl);
    }

    fn evict_oldest_excess_entries(&mut self) {
        while self.entries.len() > FRAME_PREVIEW_CACHE_MAX_ENTRIES {
            let Some(oldest_frame_id) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.cached_at)
                .map(|(frame_id, _)| *frame_id)
            else {
                break;
            };

            self.entries.remove(&oldest_frame_id);
        }
    }

    fn get_video_failure(&mut self, video_path: &Path, now: Instant) -> Option<String> {
        self.evict_expired_video_failures(now);
        self.video_failures
            .get(video_path)
            .map(|entry| entry.message.clone())
    }

    fn insert_video_failure(&mut self, video_path: &Path, message: String, now: Instant) {
        self.evict_expired_video_failures(now);
        self.video_failures.insert(
            video_path.to_path_buf(),
            CachedVideoPreviewFailure {
                message,
                cached_at: now,
            },
        );
    }

    fn evict_expired_video_failures(&mut self, now: Instant) {
        self.video_failures.retain(|_, entry| {
            now.duration_since(entry.cached_at) < FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL
        });
    }
}

#[derive(Debug, Clone)]
struct ResolvedAppInfraBaseDir {
    save_directory: String,
    base_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitDebugCpuJobRequest {
    pub document_name: String,
    pub source_text: String,
}

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
pub struct GetFramePreviewRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAudioSegmentMediaRequest {
    pub audio_segment_id: i64,
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
    pub equivalence_hint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FramePreviewSourceKindDto {
    OriginalFrame,
    SegmentFrameFallback,
    VideoFallback,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FramePreviewDto {
    pub mime_type: String,
    pub file_path: String,
    pub source_kind: FramePreviewSourceKindDto,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegmentMediaDto {
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSegmentPreviewPaths {
    workspace_dir: PathBuf,
    video_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyHiddenSegmentWorkspaceRequest {
    pub workspace_dir: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HiddenSegmentWorkspacePathsDto {
    pub workspace_dir: String,
    pub frames_dir: String,
    pub visible_segment_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentWorkspaceBatchReferenceDto {
    pub batch_id: i64,
    pub status: ::app_infra::FrameBatchStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentWorkspaceOcrReferenceDto {
    pub frame_id: i64,
    pub job_id: i64,
    pub status: ::app_infra::ProcessingJobStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentWorkspaceCleanupDebugInfoDto {
    pub paths: HiddenSegmentWorkspacePathsDto,
    pub disposition: ::app_infra::SegmentWorkspaceCleanupDisposition,
    pub safe_to_remove: bool,
    pub visible_segment_exists: bool,
    pub frame_count: i64,
    pub batch_references: Vec<SegmentWorkspaceBatchReferenceDto>,
    pub nonterminal_ocr_references: Vec<SegmentWorkspaceOcrReferenceDto>,
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
    pub payload_json: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
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
        Self {
            id: frame.id,
            session_id: frame.session_id,
            file_path: frame.file_path,
            captured_at: frame.captured_at,
            width: frame.width,
            height: frame.height,
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

impl From<::app_infra::ProcessingJob> for ProcessingJobDto {
    fn from(job: ::app_infra::ProcessingJob) -> Self {
        Self {
            id: job.id,
            subject_type: job.subject_type,
            subject_id: job.subject_id,
            processor: job.processor,
            status: job.status,
            attempt_count: job.attempt_count,
            payload_json: job.payload_json,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
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

impl From<::app_infra::FrameProcessingJob> for FrameProcessingJobDto {
    fn from(value: ::app_infra::FrameProcessingJob) -> Self {
        Self {
            frame: value.frame.into(),
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

impl From<::app_infra::HiddenSegmentWorkspacePaths> for HiddenSegmentWorkspacePathsDto {
    fn from(paths: ::app_infra::HiddenSegmentWorkspacePaths) -> Self {
        Self {
            workspace_dir: paths.workspace_dir,
            frames_dir: paths.frames_dir,
            visible_segment_path: paths.visible_segment_path,
        }
    }
}

impl From<::app_infra::SegmentWorkspaceBatchReference> for SegmentWorkspaceBatchReferenceDto {
    fn from(reference: ::app_infra::SegmentWorkspaceBatchReference) -> Self {
        Self {
            batch_id: reference.batch_id,
            status: reference.status,
        }
    }
}

impl From<::app_infra::SegmentWorkspaceOcrReference> for SegmentWorkspaceOcrReferenceDto {
    fn from(reference: ::app_infra::SegmentWorkspaceOcrReference) -> Self {
        Self {
            frame_id: reference.frame_id,
            job_id: reference.job_id,
            status: reference.status,
        }
    }
}

impl From<::app_infra::SegmentWorkspaceCleanupDebugInfo> for SegmentWorkspaceCleanupDebugInfoDto {
    fn from(info: ::app_infra::SegmentWorkspaceCleanupDebugInfo) -> Self {
        Self {
            paths: info.paths.into(),
            disposition: info.disposition,
            safe_to_remove: info.safe_to_remove,
            visible_segment_exists: info.visible_segment_exists,
            frame_count: info.frame_count,
            batch_references: info
                .batch_references
                .into_iter()
                .map(SegmentWorkspaceBatchReferenceDto::from)
                .collect(),
            nonterminal_ocr_references: info
                .nonterminal_ocr_references
                .into_iter()
                .map(SegmentWorkspaceOcrReferenceDto::from)
                .collect(),
        }
    }
}

#[cfg(target_os = "macos")]
fn audio_file_duration_ms(file_path: &str) -> Option<u64> {
    use cidre::{av, ns};

    let url = ns::Url::with_fs_path_str(file_path, false);
    let asset = av::UrlAsset::with_url(&url, None)?;
    let duration_seconds = asset.duration().as_secs();
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }

    Some((duration_seconds * 1_000.0).round() as u64)
}

#[cfg(target_os = "macos")]
fn rfc3339_plus_duration_ms(started_at: &str, duration_ms: u64) -> Option<String> {
    let start = OffsetDateTime::parse(started_at, &Rfc3339).ok()?;
    let end = start.checked_add(time::Duration::milliseconds(duration_ms.try_into().ok()?))?;
    end.format(&Rfc3339).ok()
}

#[cfg(target_os = "macos")]
fn audio_segment_dto_with_media_duration(segment: ::app_infra::AudioSegment) -> AudioSegmentDto {
    let mut dto = AudioSegmentDto::from(segment);
    if let Some(duration_ms) = audio_file_duration_ms(&dto.file_path) {
        if let Some(ended_at) = rfc3339_plus_duration_ms(&dto.started_at, duration_ms) {
            dto.ended_at = ended_at;
        }
    }
    dto
}

#[cfg(not(target_os = "macos"))]
fn audio_segment_dto_with_media_duration(segment: ::app_infra::AudioSegment) -> AudioSegmentDto {
    AudioSegmentDto::from(segment)
}

impl From<SubmitDebugCpuJobRequest> for ::app_infra::DebugCpuJobRequest {
    fn from(request: SubmitDebugCpuJobRequest) -> Self {
        Self {
            document_name: request.document_name,
            source_text: request.source_text,
        }
    }
}

impl DebugInsertFrameAndEnqueueProcessingJobRequest {
    fn into_parts(self) -> (::app_infra::NewFrame, String, Option<String>) {
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

fn captured_at_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn frame_preview_payload(
    file_path: impl Into<String>,
    mime_type: &str,
    source_kind: FramePreviewSourceKindDto,
) -> FramePreviewDto {
    FramePreviewDto {
        mime_type: mime_type.to_string(),
        file_path: file_path.into(),
        source_kind,
    }
}

fn generated_frame_preview_file_name(frame_id: i64, mime_type: &str) -> String {
    let ext = match mime_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    format!("frame-{frame_id}.{ext}")
}

fn cleanup_generated_frame_preview_cache_dir(cache_dir: &Path) -> Result<(), String> {
    if !cache_dir.is_dir() {
        return Ok(());
    }

    let now = std::time::SystemTime::now();
    let mut files = fs::read_dir(cache_dir)
        .map_err(|error| format!("failed to read preview cache directory {}: {error}", cache_dir.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let modified = metadata.modified().ok().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            Some((path, modified))
        })
        .collect::<Vec<_>>();

    for (path, modified) in &files {
        if now.duration_since(*modified).unwrap_or_default() > GENERATED_FRAME_PREVIEW_CACHE_MAX_AGE {
            let _ = fs::remove_file(path);
        }
    }

    files.retain(|(path, _)| path.is_file());
    files.sort_by_key(|(_, modified)| *modified);
    while files.len() > GENERATED_FRAME_PREVIEW_CACHE_MAX_FILES {
        let (path, _) = files.remove(0);
        let _ = fs::remove_file(path);
    }

    Ok(())
}

fn ensure_generated_frame_preview_cache_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let cache_dir = app_handle
        .path()
        .resolve(GENERATED_FRAME_PREVIEW_CACHE_DIR, BaseDirectory::AppCache)
        .map_err(|error| format!("failed to resolve app preview cache directory: {error}"))?;
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("failed to create app preview cache directory {}: {error}", cache_dir.display()))?;
    app_handle
        .asset_protocol_scope()
        .allow_directory(&cache_dir, true)
        .map_err(|error| format!("failed to allow preview cache directory {} in asset scope: {error}", cache_dir.display()))?;
    cleanup_generated_frame_preview_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

fn allow_preview_file(app_handle: &tauri::AppHandle, file_path: &Path) -> Result<(), String> {
    app_handle
        .asset_protocol_scope()
        .allow_file(file_path)
        .map_err(|error| format!("failed to allow preview file {} in asset scope: {error}", file_path.display()))
}

fn persist_generated_frame_preview_in_dir(
    cache_dir: &Path,
    frame_id: i64,
    bytes: &[u8],
    mime_type: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(cache_dir)
        .map_err(|error| format!("failed to create preview cache directory {}: {error}", cache_dir.display()))?;
    let output_path = cache_dir.join(generated_frame_preview_file_name(frame_id, mime_type));
    if !output_path.is_file() {
        let temp_file = tempfile::NamedTempFile::new_in(cache_dir)
            .map_err(|error| format!("failed to create temporary preview file in {}: {error}", cache_dir.display()))?;
        fs::write(temp_file.path(), bytes)
            .map_err(|error| format!("failed to write temporary preview file {}: {error}", temp_file.path().display()))?;
        temp_file
            .persist(&output_path)
            .map_err(|error| format!("failed to persist generated preview file {}: {error}", output_path.display()))?;
    }
    Ok(output_path)
}

fn persist_generated_frame_preview(
    app_handle: &tauri::AppHandle,
    frame_id: i64,
    bytes: &[u8],
    mime_type: &str,
) -> Result<PathBuf, String> {
    let cache_dir = ensure_generated_frame_preview_cache_dir(app_handle)?;
    let output_path = persist_generated_frame_preview_in_dir(&cache_dir, frame_id, bytes, mime_type)?;
    allow_preview_file(app_handle, &output_path)?;
    Ok(output_path)
}

fn frame_image_mime_type(file_path: &Path) -> &'static str {
    match file_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "image/png",
    }
}

fn audio_segment_mime_type(file_path: &Path) -> &'static str {
    match file_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("m4a") => "audio/mp4; codecs=mp4a.40.2",
        Some("mp4") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

async fn get_audio_segment_media_inner(
    infra: &::app_infra::AppInfra,
    audio_segment_id: i64,
) -> ::app_infra::Result<Option<AudioSegmentMediaDto>> {
    let Some(segment) = infra.get_audio_segment(audio_segment_id).await? else {
        return Ok(None);
    };

    let file_path = PathBuf::from(&segment.file_path);
    let bytes = fs::read(&file_path).map_err(|error| {
        ::app_infra::AppInfraError::Io(std::io::Error::new(
            error.kind(),
            format!(
                "failed to read persisted audio segment {} at {}: {error}",
                segment.id,
                file_path.display()
            ),
        ))
    })?;

    Ok(Some(AudioSegmentMediaDto {
        mime_type: audio_segment_mime_type(&file_path).to_string(),
        data_base64: BASE64_STANDARD.encode(bytes),
    }))
}

fn resolve_segment_preview_paths(frame_file_path: &Path) -> Option<ResolvedSegmentPreviewPaths> {
    let paths =
        ::app_infra::HiddenSegmentWorkspacePaths::from_frame_artifact_path(frame_file_path)?;

    Some(ResolvedSegmentPreviewPaths {
        workspace_dir: PathBuf::from(paths.workspace_dir),
        video_path: PathBuf::from(paths.visible_segment_path),
    })
}

fn parse_frame_unix_ms_from_path(frame_file_path: &Path) -> Option<i128> {
    let stem = frame_file_path.file_stem()?.to_str()?;
    let raw = stem.strip_prefix("frame-")?;
    let (unix_ms, _) = raw.rsplit_once('-')?;
    unix_ms.parse().ok()
}

fn parse_frame_identity_from_path(frame_file_path: &Path) -> Option<(u64, u64)> {
    let stem = frame_file_path.file_stem()?.to_str()?;
    let raw = stem.strip_prefix("frame-")?;
    let (captured_at_unix_ms, frame_index) = raw.rsplit_once('-')?;
    Some((captured_at_unix_ms.parse().ok()?, frame_index.parse().ok()?))
}

fn parse_captured_at_unix_ms(captured_at: &str) -> Option<i128> {
    OffsetDateTime::parse(captured_at, &Rfc3339)
        .ok()
        .map(|timestamp| timestamp.unix_timestamp_nanos() / 1_000_000)
}

fn estimate_frame_preview_offset_seconds(
    frame: &::app_infra::Frame,
    related_frames: &[::app_infra::Frame],
) -> f64 {
    let target_unix_ms = frame_preview_unix_ms(frame);

    let first_unix_ms = related_frames.first().and_then(frame_preview_unix_ms);

    match (target_unix_ms, first_unix_ms) {
        (Some(target), Some(first)) if target >= first => (target - first) as f64 / 1000.0,
        _ => 0.0,
    }
}

fn frame_preview_unix_ms(frame: &::app_infra::Frame) -> Option<i128> {
    parse_frame_unix_ms_from_path(Path::new(&frame.file_path))
        .or_else(|| parse_captured_at_unix_ms(&frame.captured_at))
}

fn indexed_frame_preview_offset(
    frame: &::app_infra::Frame,
    video_path: &Path,
) -> std::io::Result<Option<IndexedFramePreviewOffset>> {
    let index_path = capture_screen::screen_segment_frame_index_path(video_path);
    let index = if index_path.is_file() {
        let bytes = fs::read(&index_path)?;
        capture_screen::decode_screen_segment_frame_index(&bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse screen segment frame index {}: {error}",
                    index_path.display()
                ),
            )
        })?
    } else {
        let legacy_path = capture_screen::legacy_screen_segment_frame_index_path(video_path);
        if !legacy_path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&legacy_path)?;
        let legacy: LegacyScreenSegmentFrameIndex = serde_json::from_slice(&bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse legacy screen segment frame index {}: {error}",
                    legacy_path.display()
                ),
            )
        })?;
        capture_screen::ScreenSegmentFrameIndex {
            version: legacy.version,
            entries: legacy
                .entries
                .into_iter()
                .map(|entry| capture_screen::ScreenSegmentFrameIndexEntry {
                    captured_at_unix_ms: entry.captured_at_unix_ms,
                    frame_index: entry.frame_index,
                    video_offset_ms: entry.video_offset_ms,
                })
                .collect(),
        }
    };

    if let Some((captured_at_unix_ms, frame_index)) = parse_frame_identity_from_path(Path::new(&frame.file_path)) {
        if let Some(entry) = index
            .entries
            .iter()
            .find(|entry| entry.captured_at_unix_ms == captured_at_unix_ms && entry.frame_index == frame_index)
        {
            return Ok(Some(IndexedFramePreviewOffset {
                video_offset_ms: entry.video_offset_ms,
                exact_match: true,
            }));
        }
    }

    Ok(None)
}

fn read_nearest_segment_frame_preview(
    frame: &::app_infra::Frame,
    related_frames: &[::app_infra::Frame],
) -> std::io::Result<Option<Vec<u8>>> {
    let target_unix_ms = frame_preview_unix_ms(frame);
    let mut best_match: Option<(bool, i128, usize, &str)> = None;

    for (index, related_frame) in related_frames.iter().enumerate() {
        let candidate_path = Path::new(&related_frame.file_path);
        if !candidate_path.is_file() {
            continue;
        }

        let candidate_unix_ms = frame_preview_unix_ms(related_frame);
        let (has_distance, distance) = match (target_unix_ms, candidate_unix_ms) {
            (Some(target), Some(candidate)) => (true, (target - candidate).abs()),
            _ => (false, 0),
        };

        let should_replace = match best_match {
            Some((best_has_distance, best_distance, best_index, _)) => {
                (!has_distance, distance, index) < (!best_has_distance, best_distance, best_index)
            }
            None => true,
        };

        if should_replace {
            best_match = Some((has_distance, distance, index, &related_frame.file_path));
        }
    }

    best_match
        .map(|(_, _, _, file_path)| fs::read(file_path))
        .transpose()
}

fn read_segment_frame_preview_or_return_video_error(
    frame: &::app_infra::Frame,
    _infra: &::app_infra::AppInfra,
    related_frames: &[::app_infra::Frame],
    video_path: &Path,
    app_handle: Option<&tauri::AppHandle>,
    video_error: impl Into<String>,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let video_error = video_error.into();

    if let Some(bytes) = read_nearest_segment_frame_preview(frame, related_frames)? {
        crate::native_capture::debug_log::log_warn(format!(
            "[DEBUG-frame-preview] frame_id={} falling back to persisted segment frame after video preview failure at {}: {}",
            frame.id,
            video_path.display(),
            video_error,
        ));
        let persisted_path = if let Some(app_handle) = app_handle {
            persist_generated_frame_preview(
                app_handle,
                frame.id,
                &bytes,
                frame_image_mime_type(Path::new(&frame.file_path)),
            )
        } else {
            let cache_dir = std::env::temp_dir().join("mnema-preview-test-cache");
            persist_generated_frame_preview_in_dir(
                &cache_dir,
                frame.id,
                &bytes,
                frame_image_mime_type(Path::new(&frame.file_path)),
            )
        }
        .map_err(::app_infra::AppInfraError::OcrEngine)?;
        return Ok(Some(frame_preview_payload(
            persisted_path.to_string_lossy(),
            frame_image_mime_type(Path::new(&frame.file_path)),
            FramePreviewSourceKindDto::SegmentFrameFallback,
        )));
    }

    Err(::app_infra::AppInfraError::Io(std::io::Error::other(video_error)))
}

fn mov_file_appears_openable_for_preview(video_path: &Path) -> std::io::Result<bool> {
    const SEARCH_WINDOW_BYTES: u64 = 256 * 1024;

    let mut file = fs::File::open(video_path)?;
    let file_len = file.metadata()?.len();
    if file_len < 8 {
        return Ok(false);
    }

    let prefix_len = file_len.min(SEARCH_WINDOW_BYTES) as usize;
    let mut prefix = vec![0_u8; prefix_len];
    file.read_exact(&mut prefix)?;
    if prefix.windows(4).any(|window| window == b"moov") {
        return Ok(true);
    }

    if file_len <= SEARCH_WINDOW_BYTES {
        return Ok(false);
    }

    let suffix_len = file_len.min(SEARCH_WINDOW_BYTES) as usize;
    file.seek(SeekFrom::End(-(suffix_len as i64)))?;
    let mut suffix = vec![0_u8; suffix_len];
    file.read_exact(&mut suffix)?;

    Ok(suffix.windows(4).any(|window| window == b"moov"))
}

#[cfg(test)]
type TestVideoPreviewExtractor =
    dyn Fn(PathBuf, f64) -> Result<(Vec<u8>, &'static str), String> + Send + Sync;

#[cfg(test)]
fn test_video_preview_extractor_state() -> &'static Mutex<Option<Arc<TestVideoPreviewExtractor>>> {
    static STATE: OnceLock<Mutex<Option<Arc<TestVideoPreviewExtractor>>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn run_test_video_preview_extractor(
    video_path: &Path,
    offset_seconds: f64,
) -> Option<Result<(Vec<u8>, &'static str), String>> {
    let extractor = test_video_preview_extractor_state()
        .lock()
        .expect("test video preview extractor poisoned")
        .clone();
    extractor.map(|extractor| extractor(video_path.to_path_buf(), offset_seconds))
}

#[cfg(target_os = "macos")]
fn image_bytes_from_cg_image(
    image: &cidre::cg::Image,
    ut_type: &cidre::ut::Type,
    format_label: &str,
    mime_type: &'static str,
) -> Result<(Vec<u8>, &'static str), String> {
    use cidre::{cf, cg};
    use tempfile::NamedTempFile;

    let type_identifier = ut_type.id();
    let output_file = NamedTempFile::new()
        .map_err(|error| format!("failed to create temporary {format_label} output file: {error}"))?;
    let output_path = output_file.path();
    let output_url = cf::Url::with_file_path(&output_path).ok_or_else(|| {
        format!(
            "failed to create temporary {format_label} output URL at {}",
            output_path.display()
        )
    })?;
    let mut image_destination =
        cg::ImageDst::with_url(output_url.as_ref(), type_identifier.as_cf(), 1).ok_or_else(
            || {
                format!(
                    "failed to create temporary {format_label} image destination at {}",
                    output_path.display()
                )
            },
        )?;
    image_destination.add_image(image, None);

    if !image_destination.finalize() {
        return Err(format!(
            "failed to finalize temporary {format_label} image destination at {}",
            output_path.display()
        ));
    }

    fs::read(output_path)
        .map(|bytes| (bytes, mime_type))
        .map_err(|error| {
            format!(
                "failed to read temporary {format_label} output at {}: {error}",
                output_path.display()
            )
        })
}

#[cfg(target_os = "macos")]
fn preview_image_bytes_from_cg_image(
    image: &cidre::cg::Image,
) -> Result<(Vec<u8>, &'static str), String> {
    use cidre::ut;

    image_bytes_from_cg_image(image, ut::Type::webp(), "WebP", "image/webp").or_else(
        |_webp_error| image_bytes_from_cg_image(image, ut::Type::jpeg(), "JPEG", "image/jpeg"),
    )
}

#[cfg(target_os = "macos")]
fn exact_preview_requested_time(video_offset_ms: u64) -> cidre::cm::Time {
    let value = i64::try_from(video_offset_ms)
        .ok()
        .and_then(|offset_ms| offset_ms.checked_mul(600))
        .map(|scaled_ms| (scaled_ms + 999) / 1000)
        .unwrap_or(i64::MAX);
    cidre::cm::Time::new(value, 600)
}

#[cfg(target_os = "macos")]
fn log_video_preview_exact_miss(
    video_path: &Path,
    frame: &::app_infra::Frame,
    used_indexed_offset: bool,
    require_exact_time: bool,
    offset_seconds: f64,
    requested_time: cidre::cm::Time,
    actual_time: cidre::cm::Time,
) {
    let delta_ms = actual_time.sub(requested_time).abs().as_secs() * 1000.0;
    if delta_ms < FRAME_PREVIEW_EXACT_MISS_LOG_THRESHOLD_MS {
        return;
    }

    let frame_identity = parse_frame_identity_from_path(Path::new(&frame.file_path))
        .map(|(captured_at_unix_ms, frame_index)| format!("{captured_at_unix_ms}:{frame_index}"))
        .unwrap_or_else(|| "unknown".to_string());

    crate::native_capture::debug_log::log_warn(format!(
        "[DEBUG-frame-preview] event=video_exact_miss path={} frame_id={} frame_identity={} used_indexed_offset={} require_exact_time={} offset_seconds={} requested_time={} actual_time={} delta_ms={:.3}",
        video_path.display(),
        frame.id,
        frame_identity,
        used_indexed_offset,
        require_exact_time,
        offset_seconds,
        requested_time.as_secs(),
        actual_time.as_secs(),
        delta_ms,
    ));
}

#[cfg(target_os = "macos")]
fn extract_preview_image_from_video_blocking(
    video_path: PathBuf,
    frame: &::app_infra::Frame,
    exact_offset_ms: Option<u64>,
    offset_seconds: f64,
    require_exact_time: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    #[cfg(test)]
    if let Some(result) = run_test_video_preview_extractor(&video_path, offset_seconds) {
        return result;
    }

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let result = {
        use cidre::{av, cm, ns};

        let video_url = ns::Url::with_fs_path_str(&video_path.to_string_lossy(), false);
        let asset = av::UrlAsset::with_url(&video_url, None)
            .ok_or_else(|| format!("failed to open video asset at {}", video_path.display()))?;

        let duration_seconds = asset.duration().as_secs();
        let clamped_offset_seconds = if duration_seconds.is_finite() && duration_seconds > 0.0 {
            offset_seconds.clamp(0.0, (duration_seconds - 0.001).max(0.0))
        } else {
            0.0
        };
        let requested_time = exact_offset_ms
            .map(exact_preview_requested_time)
            .unwrap_or_else(|| cm::Time::with_secs(clamped_offset_seconds, 600));
        let mut image_generator = av::AssetImageGenerator::with_asset(asset.as_ref());
        image_generator.set_applies_preferred_track_transform(true);
        if require_exact_time {
            image_generator.set_requested_time_tolerance_before(cm::Time::zero());
            image_generator.set_requested_time_tolerance_after(cm::Time::zero());
        }

        let (cg_image, actual_time) = tokio::runtime::Handle::current().block_on(async {
            image_generator.cg_image_for_time(requested_time).await
        })
        .map_err(|error| {
            format!(
                "failed to generate preview image from video {} at {}s: {error}",
                video_path.display(),
                clamped_offset_seconds,
            )
        })?;

        if requested_time.is_valid() && actual_time.is_valid() && requested_time != actual_time {
            log_video_preview_exact_miss(
                &video_path,
                frame,
                exact_offset_ms.is_some(),
                require_exact_time,
                offset_seconds,
                requested_time,
                actual_time,
            );
        }

        let preview = preview_image_bytes_from_cg_image(cg_image.as_ref());
        image_generator.cancel_all_cg_image_gen();
        preview
    };

    result
}

#[cfg(target_os = "macos")]
async fn extract_preview_image_from_video(
    video_path: &Path,
    frame: &::app_infra::Frame,
    exact_offset_ms: Option<u64>,
    offset_seconds: f64,
    require_exact_time: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    tokio::task::spawn_blocking({
        let video_path = video_path.to_path_buf();
        let frame = frame.clone();
        move || {
            extract_preview_image_from_video_blocking(
                video_path,
                &frame,
                exact_offset_ms,
                offset_seconds,
                require_exact_time,
            )
        }
    })
    .await
    .map_err(|error| format!("failed to join video preview extraction task: {error}"))?
}

#[cfg(not(target_os = "macos"))]
async fn extract_preview_image_from_video(
    _video_path: &Path,
    _offset_seconds: f64,
    _require_exact_time: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    Err("video frame preview fallback is only supported on macOS".to_string())
}

async fn get_frame_preview_inner(
    infra: &::app_infra::AppInfra,
    cache: &FramePreviewCacheState,
    app_handle: Option<&tauri::AppHandle>,
    frame_id: i64,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let Some(frame) = infra.get_frame(frame_id).await? else {
        return Ok(None);
    };

    let frame_file_path = PathBuf::from(&frame.file_path);
    if frame_file_path.is_file() {
        if let Some(app_handle) = app_handle {
            allow_preview_file(app_handle, &frame_file_path)
                .map_err(::app_infra::AppInfraError::OcrEngine)?;
        }
        return Ok(Some(frame_preview_payload(
            frame_file_path.to_string_lossy(),
            frame_image_mime_type(&frame_file_path),
            FramePreviewSourceKindDto::OriginalFrame,
        )));
    }

    let segment_paths = resolve_segment_preview_paths(&frame_file_path).ok_or_else(|| {
        ::app_infra::AppInfraError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "unable to infer segment video path from frame artifact path {}",
                frame.file_path
            ),
        ))
    })?;

    let workspace_prefix = format!("{}/", segment_paths.workspace_dir.to_string_lossy());
    let related_frames = infra
        .list_frames_for_segment_workspace(&frame.session_id, &workspace_prefix)
        .await?;

    if !segment_paths.video_path.is_file() {
        if let Some(bytes) = read_nearest_segment_frame_preview(&frame, &related_frames)? {
            let persisted_path = if let Some(app_handle) = app_handle {
                persist_generated_frame_preview(
                    app_handle,
                    frame.id,
                    &bytes,
                    frame_image_mime_type(Path::new(&frame.file_path)),
                )
            } else {
                let cache_dir = std::env::temp_dir().join("mnema-preview-test-cache");
                persist_generated_frame_preview_in_dir(
                    &cache_dir,
                    frame.id,
                    &bytes,
                    frame_image_mime_type(Path::new(&frame.file_path)),
                )
            }
            .map_err(::app_infra::AppInfraError::OcrEngine)?;
            return Ok(Some(frame_preview_payload(
                persisted_path.to_string_lossy(),
                frame_image_mime_type(Path::new(&frame.file_path)),
                FramePreviewSourceKindDto::SegmentFrameFallback,
            )));
        }

        return Err(::app_infra::AppInfraError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "segment video does not exist for frame {} at {}",
                frame.id,
                segment_paths.video_path.display()
            ),
        )));
    }

    let video_metadata = fs::metadata(&segment_paths.video_path)?;
    if video_metadata.len() == 0 {
        return read_segment_frame_preview_or_return_video_error(
            &frame,
            infra,
            &related_frames,
            &segment_paths.video_path,
            app_handle,
            format!(
                "segment video is empty for frame {} at {}",
                frame.id,
                segment_paths.video_path.display()
            ),
        );
    }

    if !mov_file_appears_openable_for_preview(&segment_paths.video_path)? {
        return read_segment_frame_preview_or_return_video_error(
            &frame,
            infra,
            &related_frames,
            &segment_paths.video_path,
            app_handle,
            format!(
                "segment video is missing moov atom for frame {} at {}",
                frame.id,
                segment_paths.video_path.display()
            ),
        );
    }

    let now = Instant::now();
    if let Some(cached_video_error) = cache
        .lock()
        .expect("frame preview cache poisoned")
        .get_video_failure(&segment_paths.video_path, now)
    {
        return read_segment_frame_preview_or_return_video_error(
            &frame,
            infra,
            &related_frames,
            &segment_paths.video_path,
            app_handle,
            cached_video_error,
        );
    }

    let indexed_offset = indexed_frame_preview_offset(&frame, &segment_paths.video_path)?;
    let exact_offset_ms = indexed_offset
        .as_ref()
        .filter(|offset| offset.exact_match)
        .map(|offset| offset.video_offset_ms);
    let require_exact_time = indexed_offset.as_ref().is_some_and(|offset| offset.exact_match);
    let offset_seconds = indexed_offset
        .map(|offset| offset.video_offset_ms as f64 / 1000.0)
        .unwrap_or_else(|| estimate_frame_preview_offset_seconds(&frame, &related_frames));
    let (bytes, mime_type) = loop {
        let video_request_guard = {
            let mut preview_state = cache.lock().expect("frame preview cache poisoned");
            match preview_state.begin_video_request(&segment_paths.video_path) {
                Ok(()) => Ok(()),
                Err(rx) => Err(rx),
            }
        };

        match video_request_guard {
            Ok(()) => {
                let result =
                    extract_preview_image_from_video(
                        &segment_paths.video_path,
                        &frame,
                        exact_offset_ms,
                        offset_seconds,
                        require_exact_time,
                    )
                    .await;
                let notify_result = result.as_ref().map(|_| ()).map_err(Clone::clone);
                cache
                    .lock()
                    .expect("frame preview cache poisoned")
                    .finish_video_request(&segment_paths.video_path, notify_result);

                match result {
                    Ok(result) => break result,
                    Err(video_error) => {
                        cache
                            .lock()
                            .expect("frame preview cache poisoned")
                            .insert_video_failure(&segment_paths.video_path, video_error.clone(), now);
                        return read_segment_frame_preview_or_return_video_error(
                            &frame,
                            infra,
                            &related_frames,
                            &segment_paths.video_path,
                            app_handle,
                            video_error,
                        );
                    }
                }
            }
            Err(waiter) => {
                let leader_result = waiter.await.map_err(|_| {
                    ::app_infra::AppInfraError::Io(std::io::Error::other(format!(
                        "video preview request waiter dropped for {}",
                        segment_paths.video_path.display()
                    )))
                })?;

                if let Err(video_error) = leader_result {
                    return read_segment_frame_preview_or_return_video_error(
                        &frame,
                        infra,
                        &related_frames,
                        &segment_paths.video_path,
                        app_handle,
                        video_error,
                    );
                }
            }
        }
    };

    let persisted_path = if let Some(app_handle) = app_handle {
        persist_generated_frame_preview(app_handle, frame.id, &bytes, mime_type)
    } else {
        let cache_dir = std::env::temp_dir().join("mnema-preview-test-cache");
        persist_generated_frame_preview_in_dir(&cache_dir, frame.id, &bytes, mime_type)
    }
    .map_err(::app_infra::AppInfraError::OcrEngine)?;

    Ok(Some(frame_preview_payload(
        persisted_path.to_string_lossy(),
        mime_type,
        FramePreviewSourceKindDto::VideoFallback,
    )))
}

async fn get_frame_preview_inner_with_logging(
    infra: &::app_infra::AppInfra,
    cache: &FramePreviewCacheState,
    app_handle: Option<&tauri::AppHandle>,
    frame_id: i64,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let started_at = Instant::now();
    let result = get_frame_preview_inner(infra, cache, app_handle, frame_id).await;
    let elapsed_ms = started_at.elapsed().as_millis();

    match &result {
        Ok(Some(_preview)) => {}
        Ok(None) => crate::native_capture::debug_log::log_warn(format!(
            "[DEBUG-frame-preview] frame_id={} missing elapsed_ms={}",
            frame_id, elapsed_ms,
        )),
        Err(error) => crate::native_capture::debug_log::log_error(format!(
            "[DEBUG-frame-preview] frame_id={} failed elapsed_ms={} error={}",
            frame_id, elapsed_ms, error,
        )),
    }

    result
}

fn preview_cache_ttl(settings: &crate::native_capture::RecordingSettingsState) -> Duration {
    let ttl_seconds = settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .preview_cache_ttl_seconds;

    Duration::from_secs(ttl_seconds)
}

fn run_generated_frame_preview_cache_startup_pass(app_handle: &tauri::AppHandle) {
    match app_handle
        .path()
        .resolve(GENERATED_FRAME_PREVIEW_CACHE_DIR, BaseDirectory::AppCache)
    {
        Ok(cache_dir) => {
            if let Err(error) = cleanup_generated_frame_preview_cache_dir(&cache_dir) {
                crate::native_capture::debug_log::log_warn(format!(
                    "failed generated frame preview cache startup cleanup at {}: {error}",
                    cache_dir.display()
                ));
            }
        }
        Err(error) => crate::native_capture::debug_log::log_warn(format!(
            "failed to resolve generated frame preview cache directory for startup cleanup: {error}"
        )),
    }
}

fn merged_ocr_payload_json(
    payload_json: Option<&str>,
    ocr_settings: &OcrSettings,
) -> Result<Option<String>, String> {
    let mut payload = match payload_json {
        Some(payload_json) => serde_json::from_str::<OcrJobPayload>(payload_json)
            .map_err(|error| format!("failed to parse OCR payload JSON: {error}"))?,
        None => OcrJobPayload::default(),
    };

    payload.recognition_mode = Some(ocr_settings.recognition_mode.clone());
    payload.language_correction = Some(ocr_settings.language_correction);

    serde_json::to_string(&payload)
        .map(Some)
        .map_err(|error| format!("failed to serialize OCR payload JSON: {error}"))
}

fn ocr_payload_json_from_settings(
    settings: &crate::native_capture::RecordingSettingsState,
    payload_json: Option<&str>,
) -> Result<Option<String>, String> {
    let ocr_settings = settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .ocr
        .clone();

    merged_ocr_payload_json(payload_json, &ocr_settings)
}

pub async fn persist_screen_frame_artifact(
    infra: &::app_infra::AppInfra,
    settings: &crate::native_capture::RecordingSettingsState,
    session_id: &str,
    artifact: ScreenFrameArtifact,
) -> ::app_infra::Result<::app_infra::CapturedFramePipelineResult> {
    let ScreenFrameArtifact {
        file_path,
        captured_at_unix_ms,
        width,
        height,
        captured_frame_equivalence,
    } = artifact;
    let mut frame = ::app_infra::NewFrame::new(
        session_id,
        file_path.clone(),
        captured_at_from_unix_ms(captured_at_unix_ms),
    );

    if let (Some(width), Some(height)) = (width, height) {
        frame = frame.with_dimensions(i64::from(width), i64::from(height));
    }

    frame = match captured_frame_equivalence {
        capture_screen::CapturedFrameEquivalenceOutcome::Ready(equivalence) => frame
            .with_equivalence(::app_infra::FrameEquivalence::ready(
                equivalence.hint,
                equivalence.proof,
                equivalence.version,
            )),
        capture_screen::CapturedFrameEquivalenceOutcome::Quarantined(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "quarantined captured frame equivalence for session {} artifact {}: {}",
                session_id, file_path, error
            ));
            frame.with_equivalence(::app_infra::FrameEquivalence::quarantined(error))
        }
    };

    let payload_json = ocr_payload_json_from_settings(settings, None)
        .map_err(::app_infra::AppInfraError::OcrEngine)?;

    infra.capture_frame(&frame, payload_json.as_deref()).await
}

#[derive(Debug)]
struct AppInfraDirectoryLock {
    _file: File,
    path: PathBuf,
}

impl AppInfraDirectoryLock {
    fn acquire(base_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(base_dir).map_err(|error| {
            format!(
                "failed to create app infrastructure base directory {}: {error}",
                base_dir.display()
            )
        })?;

        let path = base_dir.join(APP_INFRA_LOCK_FILE_NAME);
        let file = File::create(&path).map_err(|error| {
            format!(
                "failed to open app infrastructure lock file {}: {error}",
                path.display()
            )
        })?;

        file.try_lock_exclusive().map_err(|error| {
            format!(
                "app infrastructure is already active for {}: {error}",
                base_dir.display()
            )
        })?;

        Ok(Self { _file: file, path })
    }
}

impl Drop for AppInfraDirectoryLock {
    fn drop(&mut self) {
        let _ = self._file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

pub fn initialize(app: &mut tauri::App) -> Result<(), String> {
    let app_handle = app.handle().clone();
    let resolved_base_dir = resolve_base_dir(app.handle())?;
    crate::native_capture::debug_log::log_info(format!(
        "initializing app infrastructure (save_directory='{}', base_dir='{}')",
        resolved_base_dir.save_directory,
        resolved_base_dir.base_dir.display()
    ));

    let directory_lock = AppInfraDirectoryLock::acquire(&resolved_base_dir.base_dir).map_err(
        |error| {
            crate::native_capture::debug_log::log_error(format!(
                "failed to acquire app infrastructure directory lock (save_directory='{}', base_dir='{}'): {error}",
                resolved_base_dir.save_directory,
                resolved_base_dir.base_dir.display()
            ));
            error
        },
    )?;

    let infra = tauri::async_runtime::block_on(::app_infra::AppInfra::initialize(
        &resolved_base_dir.base_dir,
    ))
    .map_err(|error| {
        crate::native_capture::debug_log::log_error(format!(
            "failed to initialize app infrastructure (save_directory='{}', base_dir='{}'): {error}",
            resolved_base_dir.save_directory,
            resolved_base_dir.base_dir.display()
        ));

        format!(
            "failed to initialize app infrastructure at {}: {error}",
            resolved_base_dir.base_dir.display()
        )
    })?;
    let infra = Arc::new(infra);
    let frame_preview_cache = FramePreviewCacheState::default();

    if !app.manage(Arc::clone(&infra)) {
        crate::native_capture::debug_log::log_error(
            "app infrastructure state was already initialized; refusing duplicate setup",
        );
        return Err("app infrastructure state was already initialized".to_string());
    }

    if !app.manage(Mutex::new(Some(directory_lock))) {
        crate::native_capture::debug_log::log_error(
            "app infrastructure directory lock state was already initialized; refusing duplicate setup",
        );
        return Err("app infrastructure directory lock state was already initialized".to_string());
    }

    if !app.manage(frame_preview_cache) {
        crate::native_capture::debug_log::log_error(
            "frame preview cache state was already initialized; refusing duplicate setup",
        );
        return Err("frame preview cache state was already initialized".to_string());
    }

    crate::native_capture::debug_log::log_info(format!(
        "initialized app infrastructure successfully (base_dir='{}')",
        resolved_base_dir.base_dir.display()
    ));

    run_generated_frame_preview_cache_startup_pass(&app_handle);
    run_frame_index_sidecar_conversion_startup_pass(&resolved_base_dir.base_dir);
    run_hidden_segment_workspace_repair_startup_pass(&infra, &resolved_base_dir.base_dir);

    spawn_processing_worker(infra, resolved_base_dir.base_dir, app_handle);

    Ok(())
}

fn run_hidden_segment_workspace_repair_startup_pass(
    infra: &::app_infra::AppInfra,
    base_dir: &Path,
) {
    let recordings_root = crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(
        base_dir.to_path_buf(),
    )
    .recordings_root();
    let recordings_root_display = recordings_root.display().to_string();

    match tauri::async_runtime::block_on(repair_hidden_segment_workspaces_once(
        infra,
        &recordings_root,
        None,
    )) {
        Ok(result) => {
            crate::native_capture::debug_log::log_info(format!(
                "startup hidden segment workspace repair completed (recordings_root='{}', scanned={}, removed={}, skipped={})",
                recordings_root_display,
                result.scanned_workspace_count,
                result.removed_workspace_count,
                result.skipped_workspace_count
            ));
        }
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "startup hidden segment workspace repair failed (recordings_root='{}'): {error}",
                recordings_root_display
            ));
        }
    }
}

fn run_frame_index_sidecar_conversion_startup_pass(base_dir: &Path) {
    let recordings_root = crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(
        base_dir.to_path_buf(),
    )
    .recordings_root();
    let recordings_root_display = recordings_root.display().to_string();

    match convert_frame_index_sidecars_once(&recordings_root) {
        Ok(result) => {
            crate::native_capture::debug_log::log_info(format!(
                "startup frame index sidecar conversion completed (recordings_root='{}', converted={}, skipped={})",
                recordings_root_display, result.converted_count, result.skipped_count
            ));
        }
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "startup frame index sidecar conversion failed (recordings_root='{}'): {error}",
                recordings_root_display
            ));
        }
    }
}

fn spawn_processing_worker(infra: AppInfraState, base_dir: PathBuf, app_handle: tauri::AppHandle) {
    let base_dir_display = base_dir.display().to_string();
    let processing_worker_infra = Arc::clone(&infra);
    crate::native_capture::debug_log::log_info(format!(
        "starting app infrastructure processing worker (base_dir='{}', idle_poll_ms={}, error_retry_ms={})",
        base_dir_display,
        PROCESSING_WORKER_IDLE_POLL_INTERVAL.as_millis(),
        PROCESSING_WORKER_ERROR_RETRY_INTERVAL.as_millis()
    ));

    let processing_worker_base_dir_display = base_dir_display.clone();
    tauri::async_runtime::spawn(async move {
        let mut consecutive_failures = 0u64;

        loop {
            match process_pending_jobs_once(&processing_worker_infra).await {
                Ok(Some(_)) => {
                    if consecutive_failures > 0 {
                        crate::native_capture::debug_log::log_info(format!(
                            "app infrastructure processing worker recovered after {} failed iteration(s) (base_dir='{}')",
                            consecutive_failures, processing_worker_base_dir_display
                        ));
                        consecutive_failures = 0;
                    }

                    continue;
                }
                Ok(None) => {
                    if consecutive_failures > 0 {
                        crate::native_capture::debug_log::log_info(format!(
                            "app infrastructure processing worker recovered after {} failed iteration(s) (base_dir='{}')",
                            consecutive_failures, processing_worker_base_dir_display
                        ));
                        consecutive_failures = 0;
                    }

                    tokio::time::sleep(PROCESSING_WORKER_IDLE_POLL_INTERVAL).await;
                }
                Err(error) => {
                    consecutive_failures += 1;
                    crate::native_capture::debug_log::log_error(format!(
                        "app infrastructure processing worker iteration failed (base_dir='{}', consecutive_failures={}, retry_in_ms={}): {error}",
                        processing_worker_base_dir_display,
                        consecutive_failures,
                        PROCESSING_WORKER_ERROR_RETRY_INTERVAL.as_millis()
                    ));
                    tokio::time::sleep(PROCESSING_WORKER_ERROR_RETRY_INTERVAL).await;
                }
            }
        }
    });

    spawn_hidden_segment_workspace_repair_worker(infra, base_dir, app_handle);
}

fn spawn_hidden_segment_workspace_repair_worker(
    infra: AppInfraState,
    base_dir: PathBuf,
    app_handle: tauri::AppHandle,
) {
    let recordings_root = crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(
        base_dir,
    )
    .recordings_root();
    let recordings_root_display = recordings_root.display().to_string();

    crate::native_capture::debug_log::log_info(format!(
        "starting hidden segment workspace repair worker (recordings_root='{}', interval_ms={})",
        recordings_root_display,
        HIDDEN_SEGMENT_WORKSPACE_REPAIR_INTERVAL.as_millis()
    ));

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(HIDDEN_SEGMENT_WORKSPACE_REPAIR_INTERVAL).await;

            let active_screen_session_id =
                active_screen_session_id_for_hidden_workspace_repair(&app_handle);

            match repair_hidden_segment_workspaces_once(
                &infra,
                &recordings_root,
                active_screen_session_id.as_deref(),
            )
            .await
            {
                Ok(result) => {
                    crate::native_capture::debug_log::log_info(format!(
                        "hidden segment workspace repair completed (recordings_root='{}', scanned={}, removed={}, skipped={})",
                        recordings_root_display,
                        result.scanned_workspace_count,
                        result.removed_workspace_count,
                        result.skipped_workspace_count
                    ));
                }
                Err(error) => {
                    crate::native_capture::debug_log::log_error(format!(
                        "hidden segment workspace repair failed (recordings_root='{}'): {error}",
                        recordings_root_display
                    ));
                }
            }
        }
    });
}

fn convert_frame_index_sidecars_once(
    recordings_root: &Path,
) -> Result<FrameIndexSidecarConversionResult, String> {
    if !recordings_root.exists() {
        return Ok(FrameIndexSidecarConversionResult {
            converted_count: 0,
            skipped_count: 0,
        });
    }

    let mut converted_count = 0_u64;
    let mut skipped_count = 0_u64;
    let mut stack = vec![recordings_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|error| format!("failed to read {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("failed to read directory entry under {}: {error}", dir.display())
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!("failed to read file type for {}: {error}", path.display())
            })?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".frame-index.json"))
            {
                continue;
            }

            let binary_path = capture_screen::screen_segment_frame_index_path(
                &path.with_file_name(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .expect("json sidecar file name should be valid utf-8")
                        .replace(".frame-index.json", ".mov"),
                ),
            );
            if binary_path.exists() {
                skipped_count = skipped_count.saturating_add(1);
                continue;
            }

            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let legacy: LegacyScreenSegmentFrameIndex = serde_json::from_slice(&bytes).map_err(
                |error| format!("failed to parse legacy sidecar {}: {error}", path.display()),
            )?;
            let binary = capture_screen::encode_screen_segment_frame_index(
                &capture_screen::ScreenSegmentFrameIndex {
                    version: legacy.version,
                    entries: legacy
                        .entries
                        .into_iter()
                        .map(|entry| capture_screen::ScreenSegmentFrameIndexEntry {
                            captured_at_unix_ms: entry.captured_at_unix_ms,
                            frame_index: entry.frame_index,
                            video_offset_ms: entry.video_offset_ms,
                        })
                        .collect(),
                },
            );
            fs::write(&binary_path, binary)
                .map_err(|error| format!("failed to write {}: {error}", binary_path.display()))?;
            converted_count = converted_count.saturating_add(1);
        }
    }

    Ok(FrameIndexSidecarConversionResult {
        converted_count,
        skipped_count,
    })
}

async fn repair_hidden_segment_workspaces_once(
    infra: &::app_infra::AppInfra,
    recordings_root: &Path,
    active_screen_session_id: Option<&str>,
) -> ::app_infra::Result<::app_infra::HiddenSegmentWorkspaceRepairResult> {
    infra
        .repair_hidden_segment_workspaces_with_context(
            recordings_root,
            &::app_infra::HiddenSegmentWorkspaceRepairContext {
                active_screen_session_id: active_screen_session_id.map(str::to_owned),
            },
        )
        .await
}

fn active_screen_session_id_for_hidden_workspace_repair(
    app_handle: &tauri::AppHandle,
) -> Option<String> {
    let state = app_handle.state::<crate::native_capture::NativeCaptureState>();
    let runtime = state.lock().ok()?;
    let runtime = runtime.runtime();
    if !runtime.is_running {
        return None;
    }

    runtime
        .source_sessions
        .as_ref()?
        .screen
        .as_ref()
        .map(|screen| screen.session_id.clone())
}

async fn process_pending_jobs_once(
    infra: &::app_infra::AppInfra,
) -> ::app_infra::Result<Option<()>> {
    let did_processing = infra.process_next_processing_job().await?.is_some();

    let did_finalize = match infra.process_next_frame_batch_job().await {
        Ok(result) => result.is_some(),
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "app infrastructure frame-batch finalization failed after state update; worker will continue: {error}"
            ));
            true
        }
    };

    if did_processing || did_finalize {
        Ok(Some(()))
    } else {
        Ok(None)
    }
}

fn resolve_base_dir(app_handle: &tauri::AppHandle) -> Result<ResolvedAppInfraBaseDir, String> {
    let settings = crate::native_capture::settings::load_recording_settings_or_default(app_handle);
    let base_dir = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
        &settings.save_directory,
    )
    .base_dir()
    .clone();

    crate::native_capture::debug_log::log_info(format!(
        "resolved app infrastructure base directory (save_directory='{}', base_dir='{}')",
        settings.save_directory,
        base_dir.display()
    ));

    Ok(ResolvedAppInfraBaseDir {
        save_directory: settings.save_directory,
        base_dir,
    })
}
fn processing_subject(subject_type: String, subject_id: i64) -> ::app_infra::ProcessingSubject {
    ::app_infra::ProcessingSubject::new(subject_type, subject_id)
}

async fn debug_insert_frame_and_enqueue_processing_job_inner(
    infra: &::app_infra::AppInfra,
    request: DebugInsertFrameAndEnqueueProcessingJobRequest,
    settings: Option<&crate::native_capture::RecordingSettingsState>,
) -> ::app_infra::Result<FrameProcessingJobDto> {
    let (frame, processor, payload_json) = request.into_parts();
    let payload_json = if processor == ::app_infra::OCR_PROCESSOR {
        settings
            .map(|settings| ocr_payload_json_from_settings(settings, payload_json.as_deref()))
            .transpose()
            .map_err(::app_infra::AppInfraError::OcrEngine)?
            .flatten()
    } else {
        payload_json
    };

    infra
        .debug_insert_frame_and_enqueue_processing_job(&frame, &processor, payload_json.as_deref())
        .await
        .map(FrameProcessingJobDto::from)
}

async fn reprocess_captured_frame_ocr_inner(
    infra: &::app_infra::AppInfra,
    request: ReprocessCapturedFrameOcrRequest,
    settings: &crate::native_capture::RecordingSettingsState,
) -> ::app_infra::Result<CapturedFrameReprocessingResultDto> {
    let payload_json = ocr_payload_json_from_settings(settings, request.payload_json.as_deref())
        .map_err(::app_infra::AppInfraError::OcrEngine)?;

    infra
        .reprocess_captured_frame_ocr(request.frame_id, payload_json.as_deref())
        .await
        .map(CapturedFrameReprocessingResultDto::from)
}

async fn classify_hidden_segment_workspace_inner(
    infra: &::app_infra::AppInfra,
    request: ClassifyHiddenSegmentWorkspaceRequest,
) -> ::app_infra::Result<Option<SegmentWorkspaceCleanupDebugInfoDto>> {
    infra
        .classify_hidden_segment_workspace(Path::new(&request.workspace_dir))
        .await
        .map(|info| info.map(SegmentWorkspaceCleanupDebugInfoDto::from))
}

#[tauri::command]
pub async fn get_app_infra_status(
    state: tauri::State<'_, AppInfraState>,
) -> Result<::app_infra::AppInfraStatus, String> {
    let infra = Arc::clone(&*state);
    infra
        .status()
        .await
        .map_err(|error| format!("failed to read app infrastructure status: {error}"))
}

#[tauri::command]
pub async fn submit_debug_cpu_job(
    request: SubmitDebugCpuJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<AppJobDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .submit_debug_cpu_job(request.into())
        .await
        .map(AppJobDto::from)
        .map_err(|error| format!("failed to submit debug cpu job: {error}"))
}

#[tauri::command]
pub async fn list_app_jobs(
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_jobs()
        .await
        .map(|jobs| jobs.into_iter().map(AppJobDto::from).collect())
        .map_err(|error| format!("failed to list app jobs: {error}"))
}

#[tauri::command]
pub async fn get_app_job(
    request: GetAppJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_job(request.job_id)
        .await
        .map(|job| job.map(AppJobDto::from))
        .map_err(|error| format!("failed to get app job {}: {error}", request.job_id))
}

#[tauri::command]
pub async fn debug_insert_frame_and_enqueue_processing_job(
    request: DebugInsertFrameAndEnqueueProcessingJobRequest,
    state: tauri::State<'_, AppInfraState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    debug_insert_frame_and_enqueue_processing_job_inner(&infra, request, Some(&settings))
        .await
        .map_err(|error| {
            format!("failed to debug-insert frame and enqueue processing job: {error}")
        })
}

#[tauri::command]
pub async fn debug_insert_frame_and_enqueue_ocr(
    request: DebugInsertFrameAndEnqueueOcrRequest,
    state: tauri::State<'_, AppInfraState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    debug_insert_frame_and_enqueue_processing_job_inner(&infra, request.into(), Some(&settings))
        .await
        .map_err(|error| format!("failed to debug-insert frame and enqueue ocr job: {error}"))
}

#[tauri::command]
pub async fn reprocess_captured_frame_ocr(
    request: ReprocessCapturedFrameOcrRequest,
    state: tauri::State<'_, AppInfraState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
) -> Result<CapturedFrameReprocessingResultDto, String> {
    let infra = Arc::clone(&*state);

    reprocess_captured_frame_ocr_inner(&infra, request.clone(), &settings)
        .await
        .map_err(|error| {
            format!(
                "failed to reprocess captured frame OCR for frame {}: {error}",
                request.frame_id
            )
        })
}

#[tauri::command]
pub async fn classify_hidden_segment_workspace(
    request: ClassifyHiddenSegmentWorkspaceRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<SegmentWorkspaceCleanupDebugInfoDto>, String> {
    let infra = Arc::clone(&*state);

    classify_hidden_segment_workspace_inner(&infra, request.clone())
        .await
        .map_err(|error| {
            format!(
                "failed to classify hidden segment workspace {}: {error}",
                request.workspace_dir
            )
        })
}

#[tauri::command]
pub async fn list_frames(
    request: Option<ListFramesRequest>,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    let (session_id, before_id, limit, offset) = match request {
        Some(request) => (
            request.session_id,
            request.before_id,
            request.limit,
            request.offset,
        ),
        None => (None, None, None, None),
    };

    infra
        .list_frames(session_id.as_deref(), before_id, limit, offset)
        .await
        .map(|frames| frames.into_iter().map(FrameDto::from).collect())
        .map_err(|error| format!("failed to list frames: {error}"))
}

#[tauri::command]
pub async fn list_frame_summaries_in_range(
    request: FrameCapturedAtRangeRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<FrameSummaryDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_frame_summaries_in_range(&request.captured_at_start, &request.captured_at_end)
        .await
        .map(|frames| frames.into_iter().map(FrameSummaryDto::from).collect())
        .map_err(|error| format!("failed to list frame summaries in range: {error}"))
}

#[tauri::command]
pub async fn get_latest_frame_in_range(
    request: FrameCapturedAtRangeRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_latest_frame_in_range(&request.captured_at_start, &request.captured_at_end)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get latest frame in range: {error}"))
}

#[tauri::command]
pub async fn list_audio_segments(
    request: ListAudioSegmentsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<AudioSegmentDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_audio_segments_overlapping_range(
            &request.captured_at_start,
            &request.captured_at_end,
            None,
            None,
        )
        .await
        .map(|segments| {
            segments
                .into_iter()
                .map(audio_segment_dto_with_media_duration)
                .collect()
        })
        .map_err(|error| format!("failed to list audio segments: {error}"))
}

#[tauri::command]
pub async fn get_audio_segment_media(
    request: GetAudioSegmentMediaRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<AudioSegmentMediaDto, String> {
    let infra = Arc::clone(&*state);
    get_audio_segment_media_inner(&infra, request.audio_segment_id)
        .await
        .map_err(|error| {
            format!(
                "failed to get audio segment media {}: {error}",
                request.audio_segment_id
            )
        })?
        .ok_or_else(|| format!("audio segment {} not found", request.audio_segment_id))
}

#[tauri::command]
pub async fn get_frame(
    request: GetFrameRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get frame {}: {error}", request.frame_id))
}

#[tauri::command]
pub async fn get_nearest_earlier_equivalent_frame(
    request: GetNearestEarlierEquivalentFrameRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_nearest_earlier_equivalent_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| {
            format!(
                "failed to resolve nearest earlier equivalent frame for frame {}: {error}",
                request.frame_id
            )
        })
}

#[tauri::command]
pub async fn get_earliest_earlier_equivalent_frame(
    request: GetEarliestEarlierEquivalentFrameRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_earliest_earlier_equivalent_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| {
            format!(
                "failed to resolve earliest earlier equivalent frame for frame {}: {error}",
                request.frame_id
            )
        })
}

#[tauri::command]
pub async fn get_timeline_window_around_frame(
    request: GetTimelineWindowAroundFrameRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<FocusedFrameWindowDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_timeline_window_around_frame(
            request.frame_id,
            request.newer_limit,
            request.older_limit,
        )
        .await
        .map(FocusedFrameWindowDto::from)
        .map_err(|error| {
            format!(
                "failed to get timeline window around frame {}: {error}",
                request.frame_id
            )
        })
}

#[tauri::command]
pub async fn get_frame_preview(
    request: GetFramePreviewRequest,
    state: tauri::State<'_, AppInfraState>,
    cache: tauri::State<'_, FramePreviewCacheState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
    app_handle: tauri::AppHandle,
) -> Result<Option<FramePreviewDto>, String> {
    let infra = Arc::clone(&*state);
    let ttl = preview_cache_ttl(&settings);

    if ttl.is_zero() {
        cache.lock().expect("frame preview cache poisoned").clear();
        return get_frame_preview_inner_with_logging(&infra, &cache, Some(&app_handle), request.frame_id)
            .await
            .map_err(|error| format!("failed to get frame preview {}: {error}", request.frame_id));
    }

    let now = Instant::now();
    let request_guard = {
        let mut preview_state = cache.lock().expect("frame preview cache poisoned");
        if let Some(preview) = preview_state.get(request.frame_id, ttl, now) {
            return Ok(Some(preview));
        }

        match preview_state.begin_request(request.frame_id) {
            Ok(()) => Ok(()),
            Err(rx) => Err(rx),
        }
    };

    let preview = match request_guard {
        Ok(()) => {
            let result = get_frame_preview_inner_with_logging(&infra, &cache, Some(&app_handle), request.frame_id)
                .await
                .map_err(|error| format!("failed to get frame preview {}: {error}", request.frame_id));

            let mut preview_state = cache.lock().expect("frame preview cache poisoned");
            if let Ok(Some(preview)) = result.as_ref() {
                preview_state.insert(request.frame_id, preview.clone(), ttl, now);
            }
            preview_state.finish_request(request.frame_id, result.clone());
            result
        }
        Err(waiter) => waiter.await.map_err(|_| {
            format!(
                "failed to get frame preview {}: preview request waiter dropped",
                request.frame_id
            )
        })?,
    }?;

    Ok(preview)
}

#[tauri::command]
pub async fn list_processing_jobs(
    request: ListProcessingJobsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    infra
        .list_processing_jobs_for_subject(&subject)
        .await
        .map(|jobs| jobs.into_iter().map(ProcessingJobDto::from).collect())
        .map_err(|error| format!("failed to list processing jobs: {error}"))
}

#[tauri::command]
pub async fn get_processing_job(
    request: GetProcessingJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_processing_job(request.job_id)
        .await
        .map(|job| job.map(ProcessingJobDto::from))
        .map_err(|error| format!("failed to get processing job {}: {error}", request.job_id))
}

#[tauri::command]
pub async fn get_processing_result(
    request: GetProcessingResultRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<ProcessingResultDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_processing_result_for_job(request.job_id)
        .await
        .map(|result| result.map(ProcessingResultDto::from))
        .map_err(|error| {
            format!(
                "failed to get processing result for job {}: {error}",
                request.job_id
            )
        })
}

#[tauri::command]
pub async fn list_processing_results(
    request: ListProcessingResultsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<ProcessingResultDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    infra
        .list_processing_results_for_subject(&subject)
        .await
        .map(|results| results.into_iter().map(ProcessingResultDto::from).collect())
        .map_err(|error| format!("failed to list processing results: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, atomic::{AtomicUsize, Ordering}},
        thread,
        time::Duration,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("desktop-tauri-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    fn run_multithread_async_test(test: impl std::future::Future<Output = ()> + Send + 'static) {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    struct TestVideoPreviewExtractorGuard;

    impl TestVideoPreviewExtractorGuard {
        fn install(extractor: Arc<TestVideoPreviewExtractor>) -> Self {
            let mut state = test_video_preview_extractor_state()
                .lock()
                .expect("test video preview extractor poisoned");
            assert!(
                state.is_none(),
                "test video preview extractor should not already be installed"
            );
            *state = Some(extractor);
            Self
        }
    }

    impl Drop for TestVideoPreviewExtractorGuard {
        fn drop(&mut self) {
            let mut state = test_video_preview_extractor_state()
                .lock()
                .expect("test video preview extractor poisoned");
            *state = None;
        }
    }

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

    #[test]
    fn resolve_base_dir_from_save_directory_uses_save_directory_as_base_dir() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings",
        );

        assert_eq!(layout.base_dir(), &PathBuf::from("/tmp/mnema-recordings"));
    }

    #[test]
    fn resolve_base_dir_from_save_directory_keeps_database_out_of_segment_root() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings/session-output",
        );
        let base_dir = layout.base_dir();

        assert_eq!(base_dir.parent(), Some(Path::new("/tmp/mnema-recordings/session-output")));
        assert_eq!(
            base_dir.file_name().and_then(|value| value.to_str()),
            Some("session-output")
        );
    }

    #[test]
    fn app_infra_directory_lock_rejects_second_owner_for_same_base_dir() {
        let dir = TestDir::new("app-infra-lock");

        let first = AppInfraDirectoryLock::acquire(dir.path())
            .expect("first app infra directory lock should succeed");
        let error = AppInfraDirectoryLock::acquire(dir.path())
            .expect_err("second app infra directory lock should fail");

        assert!(error.contains("already active"));

        drop(first);

        AppInfraDirectoryLock::acquire(dir.path())
            .expect("directory lock should be reacquirable after release");
    }

    #[test]
    fn recordings_root_dir_nests_under_dot_z() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings",
        );

        assert_eq!(
            layout.recordings_root(),
            PathBuf::from("/tmp/mnema-recordings").join("recordings")
        );
    }

    #[test]
    fn recordings_root_dir_is_child_of_base_dir() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings",
        );
        let base_dir = layout.base_dir().clone();
        let recordings_root = layout.recordings_root();

        assert_eq!(recordings_root.parent(), Some(base_dir.as_path()));
    }

    #[test]
    fn resolve_segment_preview_paths_maps_hidden_workspace_to_visible_video() {
        let frame_path =
            Path::new("/tmp/2026/04/12/.session-abc-segment-0004/frames/frame-1744459200123-7.png");

        let resolved =
            resolve_segment_preview_paths(frame_path).expect("segment paths should resolve");

        assert_eq!(
            resolved.workspace_dir,
            PathBuf::from("/tmp/2026/04/12/.session-abc-segment-0004")
        );
        assert_eq!(
            resolved.video_path,
            PathBuf::from("/tmp/2026/04/12/session-abc-segment-0004.mov")
        );
    }

    #[test]
    fn classify_hidden_segment_workspace_info_dto_maps_nested_debug_payload() {
        let dto = SegmentWorkspaceCleanupDebugInfoDto::from(
            ::app_infra::SegmentWorkspaceCleanupDebugInfo {
                paths: ::app_infra::HiddenSegmentWorkspacePaths {
                    workspace_dir: "/tmp/.session-segment-0001".to_string(),
                    frames_dir: "/tmp/.session-segment-0001/frames".to_string(),
                    visible_segment_path: "/tmp/session-segment-0001.mov".to_string(),
                },
                disposition: ::app_infra::SegmentWorkspaceCleanupDisposition::CompletedOnly,
                safe_to_remove: true,
                visible_segment_exists: true,
                frame_count: 2,
                batch_references: vec![::app_infra::SegmentWorkspaceBatchReference {
                    batch_id: 7,
                    status: ::app_infra::FrameBatchStatus::Completed,
                }],
                nonterminal_ocr_references: vec![::app_infra::SegmentWorkspaceOcrReference {
                    frame_id: 11,
                    job_id: 12,
                    status: ::app_infra::ProcessingJobStatus::Queued,
                }],
            },
        );

        assert_eq!(dto.paths.workspace_dir, "/tmp/.session-segment-0001");
        assert_eq!(dto.paths.frames_dir, "/tmp/.session-segment-0001/frames");
        assert_eq!(
            dto.paths.visible_segment_path,
            "/tmp/session-segment-0001.mov"
        );
        assert_eq!(
            dto.disposition,
            ::app_infra::SegmentWorkspaceCleanupDisposition::CompletedOnly
        );
        assert!(dto.safe_to_remove);
        assert_eq!(dto.frame_count, 2);
        assert_eq!(dto.batch_references.len(), 1);
        assert_eq!(dto.batch_references[0].batch_id, 7);
        assert_eq!(dto.nonterminal_ocr_references.len(), 1);
        assert_eq!(dto.nonterminal_ocr_references[0].frame_id, 11);
        assert_eq!(dto.nonterminal_ocr_references[0].job_id, 12);
    }

    #[test]
    fn estimate_frame_preview_offset_seconds_uses_segment_frame_times() {
        let frame = ::app_infra::Frame {
            id: 2,
            session_id: "session-estimate".to_string(),
            file_path: "/tmp/.session-estimate-segment-0004/frames/frame-1744459201500-1.png"
                .to_string(),
            captured_at: "2025-04-12T10:00:01.500Z".to_string(),
            width: None,
            height: None,
            equivalence: ::app_infra::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        };
        let related_frames = vec![::app_infra::Frame {
            id: 1,
            session_id: "session-estimate".to_string(),
            file_path: "/tmp/.session-estimate-segment-0004/frames/frame-1744459200000-0.png"
                .to_string(),
            captured_at: "2025-04-12T10:00:00Z".to_string(),
            width: None,
            height: None,
            equivalence: ::app_infra::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        }];

        let offset_seconds = estimate_frame_preview_offset_seconds(&frame, &related_frames);

        assert!((offset_seconds - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn indexed_frame_preview_offset_prefers_exact_frame_identity_match() {
        let dir = TestDir::new("frame-preview-indexed-exact");
        let video_path = dir.path().join("session-preview-segment-0001.mov");
        fs::write(&video_path, b"fake mov").expect("video fixture should exist");
        let index_path = capture_screen::screen_segment_frame_index_path(&video_path);
        let index = capture_screen::ScreenSegmentFrameIndex {
            version: capture_screen::SCREEN_SEGMENT_FRAME_INDEX_VERSION,
            entries: vec![capture_screen::ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 1_744_459_201_500,
                frame_index: 42,
                video_offset_ms: 875,
            }],
        };
        fs::write(
            &index_path,
            capture_screen::encode_screen_segment_frame_index(&index),
        )
            .expect("index file should be written");

        let frame = ::app_infra::Frame {
            id: 2,
            session_id: "session-preview".to_string(),
            file_path: dir
                .path()
                .join(".session-preview-segment-0001/frames/frame-1744459201500-000042.jpg")
                .to_string_lossy()
                .to_string(),
            captured_at: "2025-04-12T10:00:01.500Z".to_string(),
            width: None,
            height: None,
            equivalence: ::app_infra::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        };

        let offset = indexed_frame_preview_offset(&frame, &video_path)
            .expect("index lookup should succeed")
            .expect("index entry should exist");

        assert_eq!(offset.video_offset_ms, 875);
        assert!(offset.exact_match);
    }

    #[test]
    fn indexed_frame_preview_offset_reads_legacy_json_sidecar() {
        let dir = TestDir::new("frame-preview-indexed-legacy-json");
        let video_path = dir.path().join("session-preview-segment-0001.mov");
        fs::write(&video_path, b"fake mov").expect("video fixture should exist");
        let legacy_path = capture_screen::legacy_screen_segment_frame_index_path(&video_path);
        fs::write(
            &legacy_path,
            br#"{"version":1,"entries":[{"captured_at_unix_ms":1744459201500,"frame_index":42,"artifact_file_name":"frame-1744459201500-000042.jpg","video_offset_ms":875}]}"#,
        )
        .expect("legacy json sidecar should be written");

        let frame = ::app_infra::Frame {
            id: 2,
            session_id: "session-preview".to_string(),
            file_path: dir
                .path()
                .join(".session-preview-segment-0001/frames/frame-1744459201500-000042.jpg")
                .to_string_lossy()
                .to_string(),
            captured_at: "2025-04-12T10:00:01.500Z".to_string(),
            width: None,
            height: None,
            equivalence: ::app_infra::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        };

        let offset = indexed_frame_preview_offset(&frame, &video_path)
            .expect("legacy lookup should succeed")
            .expect("legacy index entry should exist");

        assert_eq!(offset.video_offset_ms, 875);
        assert!(offset.exact_match);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn exact_preview_requested_time_rounds_up_to_video_tick() {
        let requested = exact_preview_requested_time(56_133);

        assert_eq!(requested.as_secs(), 56.13333333333333);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn video_preview_exact_miss_log_includes_requested_actual_and_delta() {
        let dir = TestDir::new("frame-preview-exact-miss-log");
        let log_path = dir.path().join("native-capture-debug.log");
        let requested = cidre::cm::Time::with_secs(1.5, 600);
        let actual = cidre::cm::Time::with_secs(1.0 / 600.0 + 1.5, 600);
        let frame = ::app_infra::Frame {
            id: 2,
            session_id: "session-preview".to_string(),
            file_path: dir
                .path()
                .join(".session-preview-segment-0001/frames/frame-1744459201500-000042.jpg")
                .to_string_lossy()
                .to_string(),
            captured_at: "2025-04-12T10:00:01.500Z".to_string(),
            width: None,
            height: None,
            equivalence: ::app_infra::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        };

        capture_runtime::configure_debug_log(true, Some(log_path.clone()));
        log_video_preview_exact_miss(
            Path::new("/tmp/session-preview-segment-0001.mov"),
            &frame,
            true,
            true,
            1.5,
            requested,
            actual,
        );
        capture_runtime::configure_debug_log(false, None);

        let contents = fs::read_to_string(&log_path).expect("exact miss log file should exist");
        assert!(contents.contains("[DEBUG-frame-preview] event=video_exact_miss"));
        assert!(contents.contains("path=/tmp/session-preview-segment-0001.mov"));
        assert!(contents.contains("frame_id=2"));
        assert!(contents.contains("frame_identity=1744459201500:42"));
        assert!(contents.contains("used_indexed_offset=true"));
        assert!(contents.contains("require_exact_time=true"));
        assert!(contents.contains("offset_seconds=1.5"));
        assert!(contents.contains("requested_time=1.5"));
        assert!(contents.contains("actual_time=1.5016666666666667"));
        assert!(contents.contains("delta_ms=1.667"));
    }

    #[test]
    fn get_frame_preview_inner_returns_original_frame_bytes_when_png_exists() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-original");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let cache = FramePreviewCacheState::default();
            let frame_path = dir.path().join("frame-preview.png");
            fs::write(&frame_path, b"not-a-real-png-but-preview-bytes")
                .expect("frame preview file should be written");

            let stored_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should be inserted");

            let preview = get_frame_preview_inner(&infra, &cache, None, stored_frame.id)
                .await
                .expect("preview should load")
                .expect("preview should exist");

            assert_eq!(preview.mime_type, "image/png");
            assert_eq!(
                preview.source_kind,
                FramePreviewSourceKindDto::OriginalFrame
            );
            assert_eq!(preview.file_path, frame_path.to_string_lossy());
        });
    }

    #[test]
    fn get_frame_preview_inner_returns_segment_frame_bytes_when_exact_png_and_video_are_missing() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-segment-fallback");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let cache = FramePreviewCacheState::default();
            let workspace_dir = dir.path().join("2026/04/12/.session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");

            let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
            let sibling_frame_path = frames_dir.join("frame-1744459201000-0.png");
            let sibling_bytes = b"segment-frame-preview-bytes";
            fs::write(&sibling_frame_path, sibling_bytes)
                .expect("sibling frame preview file should be written");

            let target_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    target_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.500Z",
                ))
                .await
                .expect("target frame should be inserted");

            infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    sibling_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.000Z",
                ))
                .await
                .expect("sibling frame should be inserted");

            let preview = get_frame_preview_inner(&infra, &cache, None, target_frame.id)
                .await
                .expect("preview should load")
                .expect("preview should exist");

            assert_eq!(preview.mime_type, "image/png");
            assert_eq!(
                preview.source_kind,
                FramePreviewSourceKindDto::SegmentFrameFallback
            );
            assert_ne!(preview.file_path, sibling_frame_path.to_string_lossy());
            assert!(Path::new(&preview.file_path).is_file());
        });
    }

    #[test]
    fn get_frame_preview_inner_falls_back_to_segment_frame_when_visible_video_cannot_be_opened() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-video-preferred");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let cache = FramePreviewCacheState::default();
            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");

            let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
            let sibling_frame_path = frames_dir.join("frame-1744459201000-0.png");
            let video_path = segment_dir.join("session-preview-segment-0001.mov");
            let sibling_bytes = b"segment-frame-preview-bytes";
            fs::write(&sibling_frame_path, sibling_bytes)
                .expect("sibling frame preview file should be written");
            fs::write(&video_path, b"not-a-real-video")
                .expect("visible segment video should be written");

            let target_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    target_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.500Z",
                ))
                .await
                .expect("target frame should be inserted");

            infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    sibling_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.000Z",
                ))
                .await
                .expect("sibling frame should be inserted");

            let preview = get_frame_preview_inner(&infra, &cache, None, target_frame.id)
                .await
                .expect("preview should load")
                .expect("preview should exist");

            assert_eq!(preview.mime_type, "image/png");
            assert_eq!(
                preview.source_kind,
                FramePreviewSourceKindDto::SegmentFrameFallback
            );
            assert_ne!(preview.file_path, sibling_frame_path.to_string_lossy());
            assert!(Path::new(&preview.file_path).is_file());
        });
    }

    #[test]
    fn get_frame_preview_inner_returns_error_immediately_when_visible_video_is_empty_and_no_segment_frame_exists() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-empty-video");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let cache = FramePreviewCacheState::default();
            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");

            let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
            let video_path = segment_dir.join("session-preview-segment-0001.mov");
            fs::write(&video_path, b"").expect("visible segment video should be written");

            let target_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    target_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.500Z",
                ))
                .await
                .expect("target frame should be inserted");

            let error = get_frame_preview_inner(&infra, &cache, None, target_frame.id)
                .await
                .expect_err("empty visible video without persisted fallback should error");

            let error_message = error.to_string();
            assert!(error_message.contains("segment video is empty"));
            assert!(error_message.contains(&video_path.display().to_string()));
        });
    }

    #[test]
    fn get_frame_preview_inner_serializes_video_extraction_per_segment() {
        run_multithread_async_test(async {
            let dir = TestDir::new("frame-preview-video-serialization");
            let infra = Arc::new(
                ::app_infra::AppInfra::initialize(dir.path())
                    .await
                    .expect("app infra should initialize"),
            );
            let cache = Arc::new(FramePreviewCacheState::default());
            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");

            let video_path = segment_dir.join("session-preview-segment-0001.mov");
            fs::write(&video_path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  moov mdat")
                .expect("visible segment video should be written");

            let mut frame_ids = Vec::new();
            for index in 0..4 {
                let frame_path =
                    frames_dir.join(format!("frame-1744459201{index}00-{index}.png"));
                let captured_at = format!("2025-04-12T10:00:01.{index}00Z");
                let frame = infra
                    .insert_frame(&::app_infra::NewFrame::new(
                        "session-preview",
                        frame_path.to_string_lossy().to_string(),
                        captured_at,
                    ))
                    .await
                    .expect("frame should be inserted");
                frame_ids.push(frame.id);
            }

            let concurrent_calls = Arc::new(AtomicUsize::new(0));
            let max_concurrent_calls = Arc::new(AtomicUsize::new(0));
            let _extractor_guard = TestVideoPreviewExtractorGuard::install(Arc::new({
                let concurrent_calls = Arc::clone(&concurrent_calls);
                let max_concurrent_calls = Arc::clone(&max_concurrent_calls);
                move |path, _offset_seconds| {
                    struct ActiveCallGuard {
                        concurrent_calls: Arc<AtomicUsize>,
                    }

                    impl Drop for ActiveCallGuard {
                        fn drop(&mut self) {
                            self.concurrent_calls.fetch_sub(1, Ordering::SeqCst);
                        }
                    }

                    let active = concurrent_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent_calls.fetch_max(active, Ordering::SeqCst);
                    let _active_call_guard = ActiveCallGuard {
                        concurrent_calls: Arc::clone(&concurrent_calls),
                    };

                    thread::sleep(Duration::from_millis(40));
                    if active > 1 {
                        return Err(format!(
                            "test extractor saw concurrent access for {}",
                            path.display()
                        ));
                    }

                    Ok((b"preview-bytes".to_vec(), "image/png"))
                }
            }));

            let mut tasks = Vec::new();
            for frame_id in frame_ids {
                let infra = Arc::clone(&infra);
                let cache = Arc::clone(&cache);
                tasks.push(tokio::spawn(async move {
                    get_frame_preview_inner(&infra, &cache, None, frame_id).await
                }));
            }

            for task in tasks {
                let preview = task
                    .await
                    .expect("preview task should complete")
                    .expect("preview should load")
                    .expect("preview should exist");
                assert_eq!(preview.source_kind, FramePreviewSourceKindDto::VideoFallback);
                assert!(Path::new(&preview.file_path).is_file());
            }

            assert_eq!(max_concurrent_calls.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn frame_preview_cache_returns_entries_within_ttl() {
        let dir = TestDir::new("frame-preview-cache-hit");
        let preview_path = dir.path().join("frame-preview.png");
        fs::write(&preview_path, b"preview").expect("preview fixture should exist");
        let mut cache = FramePreviewState::default();
        let now = Instant::now();
        let preview = FramePreviewDto {
            mime_type: "image/png".to_string(),
            file_path: preview_path.to_string_lossy().to_string(),
            source_kind: FramePreviewSourceKindDto::OriginalFrame,
        };

        cache.insert(42, preview.clone(), Duration::from_secs(60), now);

        assert_eq!(cache.get(42, Duration::from_secs(60), now), Some(preview));
    }

    #[test]
    fn frame_preview_cache_evicts_expired_entries() {
        let mut cache = FramePreviewState::default();
        let now = Instant::now();

        cache.insert(
            42,
            FramePreviewDto {
                mime_type: "image/png".to_string(),
                file_path: "/tmp/frame-preview.png".to_string(),
                source_kind: FramePreviewSourceKindDto::OriginalFrame,
            },
            Duration::from_secs(1),
            now,
        );

        assert_eq!(
            cache.get(42, Duration::from_secs(1), now + Duration::from_secs(1)),
            None
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn frame_preview_cache_clear_removes_existing_entries() {
        let mut cache = FramePreviewState::default();
        cache.insert(
            42,
            FramePreviewDto {
                mime_type: "image/png".to_string(),
                file_path: "/tmp/frame-preview.png".to_string(),
                source_kind: FramePreviewSourceKindDto::OriginalFrame,
            },
            Duration::from_secs(60),
            Instant::now(),
        );

        cache.clear();

        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn frame_preview_state_collapses_duplicate_in_flight_video_requests() {
        run_async_test(async {
            let mut state = FramePreviewState::default();
            let video_path = Path::new("/tmp/segment-0001.mov");

            assert!(state.begin_video_request(video_path).is_ok());
            let waiter = state
                .begin_video_request(video_path)
                .expect_err("second request should subscribe to the in-flight video leader");
            assert_eq!(state.video_in_flight_len(), 1);

            state.finish_video_request(video_path, Ok(()));

            assert_eq!(state.video_in_flight_len(), 0);
            assert_eq!(waiter.await.expect("waiter should receive result"), Ok(()));
        });
    }

    #[test]
    fn frame_preview_cache_returns_video_failure_within_ttl() {
        let mut cache = FramePreviewCache::default();
        let now = Instant::now();
        let video_path = Path::new("/tmp/segment-0001.mov");

        cache.insert_video_failure(video_path, "cannot open".to_string(), now);

        assert_eq!(
            cache.get_video_failure(video_path, now + Duration::from_secs(1)),
            Some("cannot open".to_string())
        );
    }

    #[test]
    fn mov_file_appears_openable_for_preview_requires_moov_atom() {
        let dir = TestDir::new("frame-preview-moov-check");
        let missing_moov_path = dir.path().join("missing-moov.mov");
        let with_moov_path = dir.path().join("with-moov.mov");

        fs::write(&missing_moov_path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  ")
            .expect("mov fixture without moov should be written");
        fs::write(&with_moov_path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  moov")
            .expect("mov fixture with moov should be written");

        assert!(
            !mov_file_appears_openable_for_preview(&missing_moov_path)
                .expect("missing-moov fixture should read")
        );
        assert!(
            mov_file_appears_openable_for_preview(&with_moov_path)
                .expect("with-moov fixture should read")
        );
    }

    #[test]
    fn frame_preview_cache_evicts_expired_video_failures() {
        let mut cache = FramePreviewCache::default();
        let now = Instant::now();
        let video_path = Path::new("/tmp/segment-0001.mov");

        cache.insert_video_failure(video_path, "cannot open".to_string(), now);

        assert_eq!(
            cache.get_video_failure(
                video_path,
                now + FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL + Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn frame_preview_cache_evicts_oldest_entries_when_max_size_is_reached() {
        let dir = TestDir::new("frame-preview-cache-max-size");
        let mut cache = FramePreviewState::default();
        let now = Instant::now();
        let ttl = Duration::from_secs(60);

        for frame_id in 0..=FRAME_PREVIEW_CACHE_MAX_ENTRIES as i64 {
            let preview_path = dir.path().join(format!("frame-preview-{frame_id}.png"));
            fs::write(&preview_path, frame_id.to_string()).expect("preview fixture should exist");
            cache.insert(
                frame_id,
                FramePreviewDto {
                    mime_type: "image/png".to_string(),
                    file_path: preview_path.to_string_lossy().to_string(),
                    source_kind: FramePreviewSourceKindDto::OriginalFrame,
                },
                ttl,
                now + Duration::from_millis(frame_id as u64),
            );
        }

        assert_eq!(cache.len(), FRAME_PREVIEW_CACHE_MAX_ENTRIES);
        assert_eq!(cache.get(0, ttl, now + Duration::from_secs(1)), None);
        assert!(cache
            .get(
                FRAME_PREVIEW_CACHE_MAX_ENTRIES as i64,
                ttl,
                now + Duration::from_secs(1)
            )
            .is_some());
    }

    #[test]
    fn frame_preview_state_collapses_duplicate_in_flight_requests() {
        run_async_test(async {
            let mut state = FramePreviewState::default();

            assert!(state.begin_request(42).is_ok());
            let waiter = state
                .begin_request(42)
                .expect_err("second request should subscribe to the in-flight leader");
            assert_eq!(state.in_flight_len(), 1);

            let preview = Some(FramePreviewDto {
                mime_type: "image/png".to_string(),
                file_path: "/tmp/frame-preview.png".to_string(),
                source_kind: FramePreviewSourceKindDto::OriginalFrame,
            });
            state.finish_request(42, Ok(preview.clone()));

            assert_eq!(state.in_flight_len(), 0);
            assert_eq!(waiter.await.expect("waiter should receive result"), Ok(preview));
        });
    }

    #[test]
    fn frame_preview_state_clear_removes_in_flight_requests() {
        let mut state = FramePreviewState::default();

        assert!(state.begin_request(7).is_ok());
        assert_eq!(state.in_flight_len(), 1);

        state.clear();

        assert_eq!(state.in_flight_len(), 0);
        assert_eq!(state.video_in_flight_len(), 0);
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn persist_screen_frame_artifact_maps_metadata_for_dedupable_ocr() {
        run_async_test(async {
            let dir = TestDir::new("screen-frame-artifact");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let settings = crate::native_capture::RecordingSettingsState::default();

            let persisted = persist_screen_frame_artifact(
                &infra,
                &settings,
                "session-artifact",
                ScreenFrameArtifact {
                    file_path: "/tmp/frame-artifact.png".to_string(),
                    captured_at_unix_ms: 1_744_539_600_123,
                    width: Some(1440),
                    height: Some(900),
                    captured_frame_equivalence: capture_screen::CapturedFrameEquivalenceOutcome::Ready(
                        capture_screen::CapturedFrameEquivalence {
                            hint: "feedbeefhint0001".to_string(),
                            proof: b"feedbeef-proof".to_vec(),
                            version: capture_screen::CAPTURED_FRAME_EQUIVALENCE_VERSION,
                        },
                    ),
                },
            )
            .await
            .expect("artifact should persist");

            assert_eq!(persisted.frame.session_id, "session-artifact");
            assert_eq!(persisted.frame.file_path, "/tmp/frame-artifact.png");
            assert_eq!(persisted.frame.width, Some(1440));
            assert_eq!(persisted.frame.height, Some(900));
            assert_eq!(
                persisted.frame.equivalence.hint.as_deref(),
                Some("feedbeefhint0001")
            );
            assert_eq!(
                persisted.job.as_ref().map(|job| job.processor.as_str()),
                Some("ocr")
            );

            let batches = infra
                .list_frame_batches(Some("session-artifact"))
                .await
                .expect("frame batches should list");
            assert_eq!(batches.len(), 1);
            assert_eq!(batches[0].status, ::app_infra::FrameBatchStatus::Open);
            assert_eq!(batches[0].frame_count, 1);
        });
    }

    #[test]
    fn persist_screen_frame_artifact_preserves_quarantined_equivalence() {
        run_async_test(async {
            let dir = TestDir::new("screen-frame-artifact-quarantined");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let settings = crate::native_capture::RecordingSettingsState::default();

            let persisted = persist_screen_frame_artifact(
                &infra,
                &settings,
                "session-artifact-quarantined",
                ScreenFrameArtifact {
                    file_path: "/tmp/frame-artifact-quarantined.png".to_string(),
                    captured_at_unix_ms: 1_744_539_600_123,
                    width: Some(1440),
                    height: Some(900),
                    captured_frame_equivalence:
                        capture_screen::CapturedFrameEquivalenceOutcome::quarantined(
                            "failed to derive captured frame equivalence from screen sample",
                        ),
                },
            )
            .await
            .expect("artifact should persist even when equivalence is quarantined");

            assert_eq!(
                persisted.frame.equivalence.status,
                Some(::app_infra::FrameEquivalenceStatus::Quarantined)
            );
            assert_eq!(
                persisted.frame.equivalence.error.as_deref(),
                Some("failed to derive captured frame equivalence from screen sample")
            );
            assert!(persisted.job.is_some(), "quarantined frames must still enqueue OCR");
        });
    }

    #[test]
    fn process_pending_jobs_once_claims_and_processes_queued_work() {
        run_async_test(async {
            let dir = TestDir::new("processing-worker-once");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_processing_job(
                    &::app_infra::NewFrame::new(
                        "session-worker",
                        "/tmp/frame-worker.png",
                        "2026-04-12T10:00:00Z",
                    ),
                    "missing-processor",
                    None,
                )
                .await
                .expect("frame and job should persist");

            let processed = process_pending_jobs_once(&infra)
                .await
                .expect("worker iteration should succeed");
            assert_eq!(processed, Some(()));

            let job = infra
                .get_processing_job(persisted.job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(job.status, ::app_infra::ProcessingJobStatus::Failed);
            assert_eq!(job.attempt_count, 1);
            assert_eq!(
                job.last_error.as_deref(),
                Some("processor backend is not registered for 'missing-processor'")
            );
        });
    }

    #[test]
    fn process_pending_jobs_once_can_process_frame_batch_finalize_jobs() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-worker-once");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            infra
                .capture_frame(
                    &::app_infra::NewFrame::new(
                        "session-batch-worker",
                        "/tmp/session-batch-worker-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            infra
                .capture_frame(
                    &::app_infra::NewFrame::new(
                        "session-batch-worker",
                        "/tmp/session-batch-worker-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            let processed = process_pending_jobs_once(&infra)
                .await
                .expect("worker iteration should succeed");
            assert_eq!(processed, Some(()));

            let batches = infra
                .list_frame_batches(Some("session-batch-worker"))
                .await
                .expect("frame batches should list");

            // The first batch's OCR job is processed (fails: no backend) in the
            // same iteration, making the finalize job claimable.  Finalization
            // completes successfully (frame cleanup skips missing files), so the
            // batch ends up Completed — proving the worker now services finalize
            // jobs alongside processing jobs instead of starving them.
            assert!(batches
                .iter()
                .any(|batch| batch.status == ::app_infra::FrameBatchStatus::Completed));
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_once_removes_safe_unreferenced_workspace() {
        run_async_test(async {
            let dir = TestDir::new("repair-hidden-workspace-safe");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let recordings_root = crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(
                dir.path(),
            )
            .recordings_root();
            let day_dir = recordings_root.join("2026/04/12");
            let workspace_dir = day_dir.join(".session-repair-safe-segment-0001");
            fs::create_dir_all(workspace_dir.join("frames"))
                .expect("workspace frames dir should be created");
            fs::write(
                day_dir.join("session-repair-safe-segment-0001.mov"),
                b"fake mov",
            )
            .expect("visible segment should be written");

            let result = repair_hidden_segment_workspaces_once(&infra, &recordings_root, None)
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 1);
            assert_eq!(result.removed_workspace_count, 1);
            assert_eq!(result.skipped_workspace_count, 0);
            assert!(!workspace_dir.exists(), "safe workspace should be removed");
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_once_preserves_missing_visible_segment_workspace() {
        run_async_test(async {
            let dir = TestDir::new("repair-hidden-workspace-missing-visible");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let recordings_root = crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(
                dir.path(),
            )
            .recordings_root();
            let workspace_dir = recordings_root
                .join("2026/04/12/.session-repair-preserve-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("workspace frames dir should be created");
            fs::write(frames_dir.join("frame-1.jpg"), b"fake frame")
                .expect("workspace frame artifact should be created");

            let result = repair_hidden_segment_workspaces_once(&infra, &recordings_root, None)
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 1);
            assert_eq!(result.removed_workspace_count, 0);
            assert_eq!(result.skipped_workspace_count, 1);
            assert!(
                workspace_dir.exists(),
                "workspace should be preserved when visible segment is missing"
            );
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_once_skips_active_screen_session_workspace() {
        run_async_test(async {
            let dir = TestDir::new("repair-hidden-workspace-active-session");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let recordings_root = crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(
                dir.path(),
            )
            .recordings_root();
            let day_dir = recordings_root.join("2026/04/12");
            let workspace_dir = day_dir.join(".active-screen-session-segment-0001");
            fs::create_dir_all(workspace_dir.join("frames"))
                .expect("workspace frames dir should be created");
            fs::write(
                day_dir.join("active-screen-session-segment-0001.mov"),
                b"fake mov",
            )
            .expect("visible segment should be written");

            let result = repair_hidden_segment_workspaces_once(
                &infra,
                &recordings_root,
                Some("active-screen-session"),
            )
            .await
            .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 1);
            assert_eq!(result.removed_workspace_count, 0);
            assert_eq!(result.skipped_workspace_count, 1);
            assert!(
                workspace_dir.exists(),
                "active screen session workspace should be preserved"
            );
        });
    }
}
