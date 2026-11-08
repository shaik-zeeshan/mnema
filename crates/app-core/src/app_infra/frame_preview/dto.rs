//! The preview commands' request and response shapes (the webview's wire types).

use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFramePreviewRequest {
    pub frame_id: i64,
    pub video_scope: Option<FramePreviewVideoScopeDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameScrubPreviewsRequest {
    pub frame_ids: Vec<i64>,
    pub max_pixel_size: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FramePreviewSourceKindDto {
    OriginalFrame,
    SegmentFrameFallback,
    VideoFallback,
    ScrubPreview,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FramePreviewDto {
    pub mime_type: String,
    pub file_path: String,
    pub source_kind: FramePreviewSourceKindDto,
    pub has_secret_redactions: bool,
    pub secret_redaction_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScrubPreviewMissingReasonDto {
    FrameNotFound,
    DirectFileMissing,
    SegmentUnresolved,
    FrameIndexMissing,
    FrameIndexEntryMissing,
    SegmentVideoMissing,
    DecodeFailed,
    CacheWriteFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameScrubPreviewResultDto {
    pub frame_id: i64,
    pub preview: Option<FramePreviewDto>,
    pub missing_reason: Option<ScrubPreviewMissingReasonDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameScrubPreviewsDto {
    pub previews: Vec<FrameScrubPreviewResultDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetScrubPreviewAvailabilityRequest {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub enqueue_missing: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ScrubPreviewAvailabilityStatusDto {
    Ready,
    Queued,
    NotIndexed,
    SourceMissing,
    FrameIndexMissing,
    DecodeFailed,
    CacheWriteFailed,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScrubPreviewAvailabilityPreviewDto {
    pub file_path: String,
    pub mime_type: String,
    pub source_kind: FramePreviewSourceKindDto,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScrubPreviewAvailabilityIntervalDto {
    pub segment_cache_key: String,
    pub interval_start_video_offset_ms: u64,
    pub interval_end_video_offset_ms: u64,
    pub interval_start_unix_ms: i64,
    pub interval_end_unix_ms: i64,
    pub preview: Option<ScrubPreviewAvailabilityPreviewDto>,
    pub status: ScrubPreviewAvailabilityStatusDto,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScrubPreviewAvailabilityDto {
    pub rendition: String,
    pub intervals: Vec<ScrubPreviewAvailabilityIntervalDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScrubPreviewCacheChangedPayload {
    pub rendition: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScrubPreviewCacheStatusDto {
    pub rendition: String,
    pub cache_directory: String,
    pub segment_directories: usize,
    pub preview_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedSegmentPreviewPaths {
    pub(super) workspace_dir: PathBuf,
    pub(super) video_path: PathBuf,
}
