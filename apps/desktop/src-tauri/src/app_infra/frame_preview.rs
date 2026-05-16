use std::{
    collections::{HashMap, HashSet},
    fs,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use std::sync::Condvar;

#[cfg(test)]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{path::BaseDirectory, Emitter, Manager};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::oneshot;

use super::AppInfraState;

pub type FramePreviewCacheState = Mutex<FramePreviewState>;

pub(super) const FRAME_PREVIEW_CACHE_MAX_ENTRIES: usize = 256;
pub(super) const FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL: Duration = Duration::from_secs(15);
const FRAME_PREVIEW_EXACT_MISS_LOG_THRESHOLD_MS: f64 = 5.0;
const FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT: Duration = Duration::from_secs(5);
const SCRUB_PREVIEW_DEFAULT_MAX_PIXEL_SIZE: u32 = 200;
const SCRUB_PREVIEW_MIN_MAX_PIXEL_SIZE: u32 = 200;
const SCRUB_PREVIEW_MAX_MAX_PIXEL_SIZE: u32 = 1280;
const SCRUB_PREVIEW_PERF_LOG_THRESHOLD_MS: u128 = 25;
const SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT: Duration = Duration::from_secs(2);
const SCRUB_PREVIEW_JPEG_QUALITY: u8 = 72;
const SCRUB_PREVIEW_RENDITION: &str = "v1-jpeg-q72-max360-1fps";
const SCRUB_PREVIEW_MAX_PIXEL_SIZE: u32 = 360;
const SCRUB_PREVIEW_INTERVAL_MS: u64 = 1000;
const SCRUB_PREVIEW_CHUNK_SIZE: usize = 30;
pub const SCRUB_PREVIEW_CACHE_CHANGED_EVENT: &str = "scrub_preview_cache_changed";
const GENERATED_FRAME_PREVIEW_CACHE_DIR: &str = "frame-previews";
const GENERATED_SCRUB_PREVIEW_CACHE_DIR: &str = "scrub-previews";
const GENERATED_FRAME_SCRUB_PREVIEW_CACHE_DIR: &str = "frame-v1-jpeg-q72";
const GENERATED_FRAME_PREVIEW_CACHE_MAX_FILES: usize = 512;
const GENERATED_FRAME_PREVIEW_CACHE_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ScrubPreviewCacheKey {
    frame_id: i64,
    max_pixel_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScrubIntervalWorkKey {
    segment_cache_key: String,
    interval_start_video_offset_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IndexedFramePreviewOffset {
    pub(super) video_offset_ms: u64,
    pub(super) exact_match: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LegacyScreenSegmentFrameIndexEntry {
    pub(super) captured_at_unix_ms: u64,
    pub(super) frame_index: u64,
    #[allow(dead_code)]
    #[serde(default)]
    pub(super) artifact_file_name: Option<String>,
    pub(super) video_offset_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LegacyScreenSegmentFrameIndex {
    pub(super) version: u32,
    pub(super) entries: Vec<LegacyScreenSegmentFrameIndexEntry>,
}

#[derive(Debug, Default)]
pub struct FramePreviewState {
    cache: FramePreviewCache,
    scrub_cache: FrameScrubPreviewCache,
    in_flight: HashMap<i64, Vec<oneshot::Sender<Result<Option<FramePreviewDto>, String>>>>,
    video_in_flight: HashMap<PathBuf, Vec<oneshot::Sender<Result<(), String>>>>,
    scrub_interval_in_flight: HashSet<ScrubIntervalWorkKey>,
}

#[derive(Debug, Default)]
pub(super) struct FramePreviewCache {
    entries: HashMap<i64, CachedFramePreview>,
    video_failures: HashMap<PathBuf, CachedVideoPreviewFailure>,
}

#[derive(Debug, Default)]
pub struct FrameScrubPreviewCache {
    entries: HashMap<ScrubPreviewCacheKey, CachedFramePreview>,
}

impl FramePreviewState {
    pub(super) fn get(
        &mut self,
        frame_id: i64,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.cache.get(frame_id, ttl, now)
    }

    pub(super) fn insert(
        &mut self,
        frame_id: i64,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.cache.insert(frame_id, preview, ttl, now);
    }

    pub(super) fn clear(&mut self) {
        self.cache.clear();
        self.scrub_cache.clear();
        self.in_flight.clear();
        self.video_in_flight.clear();
    }

    pub(super) fn insert_scrub(
        &mut self,
        frame_id: i64,
        max_pixel_size: u32,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.scrub_cache.insert(
            ScrubPreviewCacheKey {
                frame_id,
                max_pixel_size,
            },
            preview,
            ttl,
            now,
        );
    }

    pub(super) fn get_scrub(
        &mut self,
        frame_id: i64,
        max_pixel_size: u32,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.scrub_cache.get(
            ScrubPreviewCacheKey {
                frame_id,
                max_pixel_size,
            },
            ttl,
            now,
        )
    }

    pub(super) fn get_video_failure(&mut self, video_path: &Path, now: Instant) -> Option<String> {
        self.cache.get_video_failure(video_path, now)
    }

    pub(super) fn insert_video_failure(
        &mut self,
        video_path: &Path,
        message: String,
        now: Instant,
    ) {
        self.cache.insert_video_failure(video_path, message, now);
    }

    pub(super) fn begin_request(
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

    pub(super) fn finish_request(
        &mut self,
        frame_id: i64,
        result: Result<Option<FramePreviewDto>, String>,
    ) {
        let waiters = self.in_flight.remove(&frame_id).unwrap_or_default();
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    pub(super) fn begin_video_request(
        &mut self,
        video_path: &Path,
    ) -> Result<(), oneshot::Receiver<Result<(), String>>> {
        if let Some(waiters) = self.video_in_flight.get_mut(video_path) {
            let (tx, rx) = oneshot::channel();
            waiters.push(tx);
            return Err(rx);
        }

        self.video_in_flight
            .insert(video_path.to_path_buf(), Vec::new());
        Ok(())
    }

    pub(super) fn finish_video_request(&mut self, video_path: &Path, result: Result<(), String>) {
        let waiters = self.video_in_flight.remove(video_path).unwrap_or_default();
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    fn begin_scrub_interval_work(
        &mut self,
        keys: &[ScrubIntervalWorkKey],
    ) -> Vec<ScrubIntervalWorkKey> {
        let mut accepted = Vec::new();
        for key in keys {
            if self.scrub_interval_in_flight.insert(key.clone()) {
                accepted.push(key.clone());
            }
        }
        accepted
    }

    fn finish_scrub_interval_work(&mut self, keys: &[ScrubIntervalWorkKey]) {
        for key in keys {
            self.scrub_interval_in_flight.remove(key);
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub(super) fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    #[cfg(test)]
    pub(super) fn video_in_flight_len(&self) -> usize {
        self.video_in_flight.len()
    }
}

impl FramePreviewCache {
    pub(super) fn get(
        &mut self,
        frame_id: i64,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.evict_expired(ttl, now);
        let preview = self
            .entries
            .get(&frame_id)
            .map(|entry| entry.preview.clone())?;
        if !Path::new(&preview.file_path).is_file() {
            self.entries.remove(&frame_id);
            return None;
        }
        Some(preview)
    }

    pub(super) fn insert(
        &mut self,
        frame_id: i64,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
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

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.video_failures.clear();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
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

    pub(super) fn get_video_failure(&mut self, video_path: &Path, now: Instant) -> Option<String> {
        self.evict_expired_video_failures(now);
        self.video_failures
            .get(video_path)
            .map(|entry| entry.message.clone())
    }

    pub(super) fn insert_video_failure(
        &mut self,
        video_path: &Path,
        message: String,
        now: Instant,
    ) {
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

impl FrameScrubPreviewCache {
    pub(super) fn get(
        &mut self,
        key: ScrubPreviewCacheKey,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.evict_expired(ttl, now);
        let preview = self.entries.get(&key).map(|entry| entry.preview.clone())?;
        if !Path::new(&preview.file_path).is_file() {
            self.entries.remove(&key);
            return None;
        }
        Some(preview)
    }

    pub(super) fn insert(
        &mut self,
        key: ScrubPreviewCacheKey,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.evict_expired(ttl, now);
        self.entries.insert(
            key,
            CachedFramePreview {
                preview,
                cached_at: now,
            },
        );
        self.evict_oldest_excess_entries();
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    fn evict_expired(&mut self, ttl: Duration, now: Instant) {
        self.entries
            .retain(|_, entry| now.duration_since(entry.cached_at) < ttl);
    }

    fn evict_oldest_excess_entries(&mut self) {
        while self.entries.len() > FRAME_PREVIEW_CACHE_MAX_ENTRIES {
            let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.cached_at)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.entries.remove(&oldest_key);
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFramePreviewRequest {
    pub frame_id: i64,
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

pub(super) fn captured_at_from_unix_ms(unix_ms: u64) -> String {
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

fn scrub_preview_source_hash(source_path: &Path, extra: impl AsRef<str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_path.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update(extra.as_ref().as_bytes());
    if let Ok(metadata) = fs::metadata(source_path) {
        hasher.update(b"\0len=");
        hasher.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.duration_since(std::time::SystemTime::UNIX_EPOCH) {
                hasher.update(b"\0mtime=");
                hasher.update(duration.as_secs().to_le_bytes());
                hasher.update(duration.subsec_nanos().to_le_bytes());
            }
        }
    }
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn generated_scrub_preview_file_name(
    frame_id: i64,
    max_pixel_size: u32,
    source_hash: &str,
) -> String {
    format!("scrub-v3-frame-{frame_id}-{max_pixel_size}-{source_hash}.jpg")
}

fn generated_scrub_preview_path(
    cache_dir: &Path,
    frame_id: i64,
    max_pixel_size: u32,
    source_hash: &str,
) -> PathBuf {
    cache_dir.join(generated_scrub_preview_file_name(
        frame_id,
        max_pixel_size,
        source_hash,
    ))
}

fn clamp_scrub_preview_max_pixel_size(max_pixel_size: Option<u32>) -> u32 {
    max_pixel_size
        .unwrap_or(SCRUB_PREVIEW_DEFAULT_MAX_PIXEL_SIZE)
        .clamp(
            SCRUB_PREVIEW_MIN_MAX_PIXEL_SIZE,
            SCRUB_PREVIEW_MAX_MAX_PIXEL_SIZE,
        )
}

fn cleanup_generated_frame_preview_cache_dir(cache_dir: &Path) -> Result<(), String> {
    if !cache_dir.is_dir() {
        return Ok(());
    }

    let now = std::time::SystemTime::now();
    let mut files = fs::read_dir(cache_dir)
        .map_err(|error| {
            format!(
                "failed to read preview cache directory {}: {error}",
                cache_dir.display()
            )
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let modified = metadata
                .modified()
                .ok()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            Some((path, modified))
        })
        .collect::<Vec<_>>();

    for (path, modified) in &files {
        if now.duration_since(*modified).unwrap_or_default() > GENERATED_FRAME_PREVIEW_CACHE_MAX_AGE
        {
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

fn ensure_generated_frame_preview_cache_dir(
    app_handle: &tauri::AppHandle,
) -> Result<PathBuf, String> {
    let cache_dir = app_handle
        .path()
        .resolve(GENERATED_FRAME_PREVIEW_CACHE_DIR, BaseDirectory::AppCache)
        .map_err(|error| format!("failed to resolve app preview cache directory: {error}"))?;
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create app preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    app_handle
        .asset_protocol_scope()
        .allow_directory(&cache_dir, true)
        .map_err(|error| {
            format!(
                "failed to allow preview cache directory {} in asset scope: {error}",
                cache_dir.display()
            )
        })?;
    cleanup_generated_frame_preview_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

fn ensure_generated_frame_scrub_preview_cache_dir(
    app_handle: &tauri::AppHandle,
) -> Result<PathBuf, String> {
    let cache_dir = app_handle
        .path()
        .resolve(
            format!(
                "{GENERATED_SCRUB_PREVIEW_CACHE_DIR}/{GENERATED_FRAME_SCRUB_PREVIEW_CACHE_DIR}"
            ),
            BaseDirectory::AppCache,
        )
        .map_err(|error| {
            format!("failed to resolve app frame scrub preview cache directory: {error}")
        })?;
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create app frame scrub preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    app_handle
        .asset_protocol_scope()
        .allow_directory(&cache_dir, true)
        .map_err(|error| {
            format!(
                "failed to allow frame scrub preview cache directory {} in asset scope: {error}",
                cache_dir.display()
            )
        })?;
    cleanup_generated_frame_preview_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

fn allow_preview_file(app_handle: &tauri::AppHandle, file_path: &Path) -> Result<(), String> {
    app_handle
        .asset_protocol_scope()
        .allow_file(file_path)
        .map_err(|error| {
            format!(
                "failed to allow preview file {} in asset scope: {error}",
                file_path.display()
            )
        })
}

fn persist_generated_frame_preview_in_dir(
    cache_dir: &Path,
    frame_id: i64,
    bytes: &[u8],
    mime_type: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(cache_dir).map_err(|error| {
        format!(
            "failed to create preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    let output_path = cache_dir.join(generated_frame_preview_file_name(frame_id, mime_type));
    if !output_path.is_file() {
        let temp_file = tempfile::NamedTempFile::new_in(cache_dir).map_err(|error| {
            format!(
                "failed to create temporary preview file in {}: {error}",
                cache_dir.display()
            )
        })?;
        fs::write(temp_file.path(), bytes).map_err(|error| {
            format!(
                "failed to write temporary preview file {}: {error}",
                temp_file.path().display()
            )
        })?;
        temp_file.persist(&output_path).map_err(|error| {
            format!(
                "failed to persist generated preview file {}: {error}",
                output_path.display()
            )
        })?;
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
    let output_path =
        persist_generated_frame_preview_in_dir(&cache_dir, frame_id, bytes, mime_type)?;
    allow_preview_file(app_handle, &output_path)?;
    Ok(output_path)
}

fn persist_generated_scrub_preview_in_dir(
    cache_dir: &Path,
    frame_id: i64,
    max_pixel_size: u32,
    source_hash: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    fs::create_dir_all(cache_dir).map_err(|error| {
        format!(
            "failed to create preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    let output_path =
        generated_scrub_preview_path(cache_dir, frame_id, max_pixel_size, source_hash);
    if !output_path.is_file() {
        let temp_file = tempfile::NamedTempFile::new_in(cache_dir).map_err(|error| {
            format!(
                "failed to create temporary scrub preview file in {}: {error}",
                cache_dir.display()
            )
        })?;
        fs::write(temp_file.path(), bytes).map_err(|error| {
            format!(
                "failed to write temporary scrub preview file {}: {error}",
                temp_file.path().display()
            )
        })?;
        temp_file.persist(&output_path).map_err(|error| {
            format!(
                "failed to persist generated scrub preview file {}: {error}",
                output_path.display()
            )
        })?;
    }
    Ok(output_path)
}

pub(super) fn generate_scrub_preview_derivative_in_dir(
    cache_dir: &Path,
    frame_id: i64,
    max_pixel_size: u32,
    source_path: &Path,
) -> Result<PathBuf, String> {
    let source_hash = scrub_preview_source_hash(source_path, "direct-frame");
    let cached_path =
        generated_scrub_preview_path(cache_dir, frame_id, max_pixel_size, &source_hash);
    if cached_path.is_file() {
        return Ok(cached_path);
    }

    let image = image::ImageReader::open(source_path)
        .map_err(|error| {
            format!(
                "failed to open source frame image {}: {error}",
                source_path.display()
            )
        })?
        .with_guessed_format()
        .map_err(|error| {
            format!(
                "failed to detect source frame image format {}: {error}",
                source_path.display()
            )
        })?
        .decode()
        .map_err(|error| {
            format!(
                "failed to decode source frame image {}: {error}",
                source_path.display()
            )
        })?;
    let derivative = image.resize(
        max_pixel_size,
        max_pixel_size,
        image::imageops::FilterType::Triangle,
    );

    fs::create_dir_all(cache_dir).map_err(|error| {
        format!(
            "failed to create preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    let temp_file = tempfile::NamedTempFile::new_in(cache_dir).map_err(|error| {
        format!(
            "failed to create temporary scrub preview file in {}: {error}",
            cache_dir.display()
        )
    })?;
    let mut output = std::io::BufWriter::new(File::create(temp_file.path()).map_err(|error| {
        format!(
            "failed to create temporary scrub preview output {}: {error}",
            temp_file.path().display()
        )
    })?);
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, SCRUB_PREVIEW_JPEG_QUALITY);
    encoder.encode_image(&derivative).map_err(|error| {
        format!(
            "failed to encode scrub preview derivative for {}: {error}",
            source_path.display()
        )
    })?;
    drop(output);

    temp_file.persist(&cached_path).map_err(|error| {
        format!(
            "failed to persist generated scrub preview file {}: {error}",
            cached_path.display()
        )
    })?;
    Ok(cached_path)
}

pub(super) fn frame_image_mime_type(file_path: &Path) -> &'static str {
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

pub(super) fn resolve_segment_preview_paths(
    frame_file_path: &Path,
) -> Option<ResolvedSegmentPreviewPaths> {
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

pub(super) fn estimate_frame_preview_offset_seconds(
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

pub(super) fn indexed_frame_preview_offset(
    frame: &::app_infra::Frame,
    video_path: &Path,
) -> std::io::Result<Option<IndexedFramePreviewOffset>> {
    let Some(index) = load_screen_segment_frame_index(video_path)? else {
        return Ok(None);
    };

    Ok(find_indexed_frame_preview_offset(frame, &index))
}

fn load_screen_segment_frame_index(
    video_path: &Path,
) -> std::io::Result<Option<capture_screen::ScreenSegmentFrameIndex>> {
    let index_path = capture_screen::screen_segment_frame_index_path(video_path);
    if index_path.is_file() {
        let bytes = fs::read(&index_path)?;
        return capture_screen::decode_screen_segment_frame_index(&bytes)
            .map(Some)
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "failed to parse screen segment frame index {}: {error}",
                        index_path.display()
                    ),
                )
            });
    }

    let legacy_path = capture_screen::legacy_screen_segment_frame_index_path(video_path);
    if !legacy_path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&legacy_path)?;
    let legacy: LegacyScreenSegmentFrameIndex =
        serde_json::from_slice(&bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse legacy screen segment frame index {}: {error}",
                    legacy_path.display()
                ),
            )
        })?;
    Ok(Some(capture_screen::ScreenSegmentFrameIndex {
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
    }))
}

fn find_indexed_frame_preview_offset(
    frame: &::app_infra::Frame,
    index: &capture_screen::ScreenSegmentFrameIndex,
) -> Option<IndexedFramePreviewOffset> {
    if let Some((captured_at_unix_ms, frame_index)) =
        parse_frame_identity_from_path(Path::new(&frame.file_path))
    {
        if let Some(entry) = index.entries.iter().find(|entry| {
            entry.captured_at_unix_ms == captured_at_unix_ms && entry.frame_index == frame_index
        }) {
            return Some(IndexedFramePreviewOffset {
                video_offset_ms: entry.video_offset_ms,
                exact_match: true,
            });
        }
    }

    None
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

    Err(::app_infra::AppInfraError::Io(std::io::Error::other(
        video_error,
    )))
}

pub(super) fn mov_file_appears_openable_for_preview(video_path: &Path) -> std::io::Result<bool> {
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
pub(super) type TestVideoPreviewExtractor =
    dyn Fn(PathBuf, f64) -> Result<(Vec<u8>, &'static str), String> + Send + Sync;

#[cfg(test)]
pub(super) fn test_video_preview_extractor_state(
) -> &'static Mutex<Option<Arc<TestVideoPreviewExtractor>>> {
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
fn block_on_waker_driven_future<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    struct ThreadWaker(std::thread::Thread);

    impl std::task::Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = std::task::Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);

    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => return output,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
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
    let output_file = NamedTempFile::new().map_err(|error| {
        format!("failed to create temporary {format_label} output file: {error}")
    })?;
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
fn scrub_preview_image_bytes_from_cg_image(image: &cidre::cg::Image) -> Result<Vec<u8>, String> {
    use cidre::ut;

    image_bytes_from_cg_image(image, ut::Type::jpeg(), "JPEG", "image/jpeg").map(|(bytes, _)| bytes)
}

#[cfg(target_os = "macos")]
pub(super) fn exact_preview_requested_time(video_offset_ms: u64) -> cidre::cm::Time {
    let value = i64::try_from(video_offset_ms)
        .ok()
        .and_then(|offset_ms| offset_ms.checked_mul(600))
        .map(|scaled_ms| (scaled_ms + 999) / 1000)
        .unwrap_or(i64::MAX);
    cidre::cm::Time::new(value, 600)
}

#[cfg(target_os = "macos")]
pub(super) fn log_video_preview_exact_miss(
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

        let (cg_image, actual_time) =
            block_on_waker_driven_future(image_generator.cg_image_for_time(requested_time))
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
    tokio::time::timeout(
        FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT,
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
        }),
    )
    .await
    .map_err(|_| {
        format!(
            "timed out generating exact frame preview after {}s",
            FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT.as_secs()
        )
    })?
    .map_err(|error| format!("failed to join video preview extraction task: {error}"))?
}

#[cfg(target_os = "macos")]
fn extract_scrub_preview_images_from_video_batch_blocking(
    video_path: PathBuf,
    video_offset_ms: Vec<u64>,
    max_pixel_size: u32,
) -> Result<HashMap<u64, Result<Vec<u8>, String>>, String> {
    #[cfg(test)]
    if test_video_preview_extractor_state()
        .lock()
        .expect("test video preview extractor poisoned")
        .is_some()
    {
        let mut results = HashMap::new();
        for offset_ms in video_offset_ms {
            if let Some(result) =
                run_test_video_preview_extractor(&video_path, offset_ms as f64 / 1000.0)
            {
                results.insert(offset_ms, result.map(|(bytes, _)| bytes));
            }
        }
        return Ok(results);
    }

    if video_offset_ms.is_empty() {
        return Ok(HashMap::new());
    }

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    use cidre::{av, cg, cm, ns};

    let video_url = ns::Url::with_fs_path_str(&video_path.to_string_lossy(), false);
    let asset = av::UrlAsset::with_url(&video_url, None)
        .ok_or_else(|| format!("failed to open video asset at {}", video_path.display()))?;
    let requested_times = video_offset_ms
        .iter()
        .map(|offset_ms| (*offset_ms, exact_preview_requested_time(*offset_ms)))
        .collect::<Vec<_>>();
    let requested_time_values = requested_times
        .iter()
        .map(|(_, requested_time)| ns::Value::with_cm_time(requested_time))
        .collect::<Vec<_>>();
    let requested_time_array = ns::Array::from_slice_retained(&requested_time_values);
    let mut image_generator = av::AssetImageGenerator::with_asset(asset.as_ref());
    image_generator.set_applies_preferred_track_transform(true);
    image_generator.set_max_size(cg::Size {
        width: f64::from(max_pixel_size),
        height: f64::from(max_pixel_size),
    });
    let tolerant_window = cm::Time::with_secs(0.25, 600);
    image_generator.set_requested_time_tolerance_before(tolerant_window);
    image_generator.set_requested_time_tolerance_after(tolerant_window);

    struct BatchScrubPreviewState {
        remaining: usize,
        results: HashMap<cm::Time, Result<Vec<u8>, String>>,
    }

    let state = Arc::new((
        Mutex::new(BatchScrubPreviewState {
            remaining: requested_times.len(),
            results: HashMap::new(),
        }),
        Condvar::new(),
    ));
    let callback_state = Arc::clone(&state);
    let mut block = av::AssetImageGeneratorCh::new5(
        move |requested_time: cm::Time,
              image: Option<&cg::Image>,
              _actual_time: cm::Time,
              result: av::AssetImageGeneratorResult,
              error: Option<&ns::Error>| {
            let image_result = match result {
                av::AssetImageGeneratorResult::Succeeded => image
                    .ok_or_else(|| "scrub preview generation succeeded without image".to_string())
                    .and_then(scrub_preview_image_bytes_from_cg_image),
                av::AssetImageGeneratorResult::Failed => Err(error
                    .map(|error| format!("scrub preview image generation failed: {error}"))
                    .unwrap_or_else(|| "scrub preview image generation failed".to_string())),
                av::AssetImageGeneratorResult::Cancelled => {
                    Err("scrub preview image generation was cancelled".to_string())
                }
            };

            let (lock, cvar) = &*callback_state;
            let mut state = lock.lock().expect("scrub preview batch state poisoned");
            state.results.insert(requested_time, image_result);
            state.remaining = state.remaining.saturating_sub(1);
            if state.remaining == 0 {
                cvar.notify_one();
            }
        },
    );
    image_generator.cg_images_for_times_ch(requested_time_array.as_ref(), &mut block);

    let (lock, cvar) = &*state;
    let mut state = lock.lock().expect("scrub preview batch state poisoned");
    let wait_started_at = Instant::now();
    while state.remaining > 0 {
        let remaining_timeout = SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT
            .checked_sub(wait_started_at.elapsed())
            .unwrap_or_default();
        if remaining_timeout.is_zero() {
            break;
        }
        let (next_state, wait_result) = cvar
            .wait_timeout(state, remaining_timeout)
            .expect("scrub preview batch state poisoned while waiting");
        state = next_state;
        if wait_result.timed_out() {
            break;
        }
    }
    let timed_out = state.remaining > 0;
    let results_by_time = std::mem::take(&mut state.results);
    drop(state);
    image_generator.cancel_all_cg_image_gen();

    if timed_out {
        return Err(format!(
            "timed out generating scrub preview batch after {}s",
            SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT.as_secs()
        ));
    }

    Ok(requested_times
        .into_iter()
        .map(|(offset_ms, requested_time)| {
            let result = results_by_time
                .get(&requested_time)
                .cloned()
                .unwrap_or_else(|| {
                    Err(format!(
                        "scrub preview generation did not return image for {}ms",
                        offset_ms
                    ))
                });
            (offset_ms, result)
        })
        .collect())
}

#[cfg(target_os = "macos")]
async fn extract_scrub_preview_images_from_video_batch(
    video_path: &Path,
    video_offset_ms: &[u64],
    max_pixel_size: u32,
) -> Result<HashMap<u64, Result<Vec<u8>, String>>, String> {
    tokio::time::timeout(
        SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT,
        tokio::task::spawn_blocking({
            let video_path = video_path.to_path_buf();
            let video_offset_ms = video_offset_ms.to_vec();
            move || {
                extract_scrub_preview_images_from_video_batch_blocking(
                    video_path,
                    video_offset_ms,
                    max_pixel_size,
                )
            }
        }),
    )
    .await
    .map_err(|_| {
        format!(
            "timed out joining scrub preview extraction task after {}s",
            SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT.as_secs()
        )
    })?
    .map_err(|error| format!("failed to join scrub preview extraction task: {error}"))?
}

#[cfg(not(target_os = "macos"))]
async fn extract_scrub_preview_images_from_video_batch(
    _video_path: &Path,
    _video_offset_ms: &[u64],
    _max_pixel_size: u32,
) -> Result<HashMap<u64, Result<Vec<u8>, String>>, String> {
    Err("scrub preview video generation is only supported on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
async fn extract_preview_image_from_video(
    _video_path: &Path,
    _frame: &::app_infra::Frame,
    _exact_offset_ms: Option<u64>,
    _offset_seconds: f64,
    _require_exact_time: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    Err("video frame preview fallback is only supported on macOS".to_string())
}

pub(super) async fn get_frame_preview_inner(
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
    let require_exact_time = indexed_offset
        .as_ref()
        .is_some_and(|offset| offset.exact_match);
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
                let result = extract_preview_image_from_video(
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
                            .insert_video_failure(
                                &segment_paths.video_path,
                                video_error.clone(),
                                now,
                            );
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

pub(super) async fn get_frame_preview_inner_with_logging(
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

#[derive(Debug, Clone)]
struct PreparedVideoScrubPreview {
    frame_id: i64,
    video_path: PathBuf,
    video_offset_ms: u64,
    source_hash: String,
}

#[derive(Debug, Clone)]
enum PreparedFrameScrubPreview {
    Ready(FrameScrubPreviewResultDto),
    Video(PreparedVideoScrubPreview),
}

async fn prepare_frame_scrub_preview(
    infra: &::app_infra::AppInfra,
    app_handle: &tauri::AppHandle,
    frame_id: i64,
    max_pixel_size: u32,
    cache_dir: Option<&Path>,
) -> Result<PreparedFrameScrubPreview, String> {
    let Some(frame) = infra
        .get_frame(frame_id)
        .await
        .map_err(|error| format!("failed to get frame {frame_id}: {error}"))?
    else {
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: None,
                missing_reason: Some(ScrubPreviewMissingReasonDto::FrameNotFound),
            },
        ));
    };

    let frame_file_path = PathBuf::from(&frame.file_path);
    if let Some(cache_dir) = cache_dir {
        let source_hash = scrub_preview_source_hash(&frame_file_path, "direct-frame");
        let cached_path =
            generated_scrub_preview_path(cache_dir, frame_id, max_pixel_size, &source_hash);
        if cached_path.is_file() {
            allow_preview_file(app_handle, &cached_path)?;
            return Ok(PreparedFrameScrubPreview::Ready(
                FrameScrubPreviewResultDto {
                    frame_id,
                    preview: Some(frame_preview_payload(
                        cached_path.to_string_lossy(),
                        "image/jpeg",
                        FramePreviewSourceKindDto::ScrubPreview,
                    )),
                    missing_reason: None,
                },
            ));
        }
    }

    if frame_file_path.is_file() {
        let Some(cache_dir) = cache_dir else {
            return Ok(PreparedFrameScrubPreview::Ready(
                FrameScrubPreviewResultDto {
                    frame_id,
                    preview: None,
                    missing_reason: Some(ScrubPreviewMissingReasonDto::CacheWriteFailed),
                },
            ));
        };
        let derivative_path = generate_scrub_preview_derivative_in_dir(
            cache_dir,
            frame_id,
            max_pixel_size,
            &frame_file_path,
        )?;
        allow_preview_file(app_handle, &derivative_path)?;
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: Some(frame_preview_payload(
                    derivative_path.to_string_lossy(),
                    "image/jpeg",
                    FramePreviewSourceKindDto::ScrubPreview,
                )),
                missing_reason: None,
            },
        ));
    }

    let Some(segment_paths) = resolve_segment_preview_paths(&frame_file_path) else {
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: None,
                missing_reason: Some(if frame_file_path.extension().is_some() {
                    ScrubPreviewMissingReasonDto::DirectFileMissing
                } else {
                    ScrubPreviewMissingReasonDto::SegmentUnresolved
                }),
            },
        ));
    };

    if !segment_paths.video_path.is_file() {
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: None,
                missing_reason: Some(ScrubPreviewMissingReasonDto::SegmentVideoMissing),
            },
        ));
    }
    if fs::metadata(&segment_paths.video_path)
        .map(|metadata| metadata.len() == 0)
        .unwrap_or(true)
        || !mov_file_appears_openable_for_preview(&segment_paths.video_path).unwrap_or(false)
    {
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: None,
                missing_reason: Some(ScrubPreviewMissingReasonDto::DecodeFailed),
            },
        ));
    }

    let index = match load_screen_segment_frame_index(&segment_paths.video_path) {
        Ok(Some(index)) => index,
        Ok(None) => {
            return Ok(PreparedFrameScrubPreview::Ready(
                FrameScrubPreviewResultDto {
                    frame_id,
                    preview: None,
                    missing_reason: Some(ScrubPreviewMissingReasonDto::FrameIndexMissing),
                },
            ));
        }
        Err(_) => {
            return Ok(PreparedFrameScrubPreview::Ready(
                FrameScrubPreviewResultDto {
                    frame_id,
                    preview: None,
                    missing_reason: Some(ScrubPreviewMissingReasonDto::DecodeFailed),
                },
            ));
        }
    };
    let Some(indexed_offset) = find_indexed_frame_preview_offset(&frame, &index) else {
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: None,
                missing_reason: Some(ScrubPreviewMissingReasonDto::FrameIndexEntryMissing),
            },
        ));
    };

    Ok(PreparedFrameScrubPreview::Video(
        PreparedVideoScrubPreview {
            frame_id,
            source_hash: scrub_preview_source_hash(
                &segment_paths.video_path,
                indexed_offset.video_offset_ms.to_string(),
            ),
            video_path: segment_paths.video_path,
            video_offset_ms: indexed_offset.video_offset_ms,
        },
    ))
}

fn preview_cache_ttl(settings: &crate::native_capture::RecordingSettingsState) -> Duration {
    let ttl_seconds = settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .preview_cache_ttl_seconds;

    Duration::from_secs(ttl_seconds)
}

fn scrub_preview_perf_debug_enabled() -> bool {
    matches!(
        std::env::var("MNEMA_SCRUB_PERF_DEBUG").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

fn log_scrub_preview_perf(event: &str, fields: impl AsRef<str>) {
    if !scrub_preview_perf_debug_enabled() {
        return;
    }
    crate::native_capture::debug_log::log_info(format!(
        "[DEBUG-scrub-perf] event={event} {}",
        fields.as_ref()
    ));
}

pub(crate) fn run_generated_frame_preview_cache_startup_pass(app_handle: &tauri::AppHandle) {
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

fn scrub_preview_cache_root(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let cache_dir = app_handle
        .path()
        .resolve(GENERATED_SCRUB_PREVIEW_CACHE_DIR, BaseDirectory::AppCache)
        .map_err(|error| format!("failed to resolve scrub preview cache directory: {error}"))?
        .join(SCRUB_PREVIEW_RENDITION);
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create scrub preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    app_handle
        .asset_protocol_scope()
        .allow_directory(&cache_dir, true)
        .map_err(|error| {
            format!(
                "failed to allow scrub preview cache directory {} in asset scope: {error}",
                cache_dir.display()
            )
        })?;
    Ok(cache_dir)
}

fn unix_ms_to_rfc3339(unix_ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn rfc3339_to_unix_ms(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(|dt| i64::try_from(dt.unix_timestamp_nanos() / 1_000_000).ok())
}

fn file_freshness(path: &Path) -> (u64, u64) {
    let Ok(metadata) = fs::metadata(path) else {
        return (0, 0);
    };
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| {
            modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .ok()
        })
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    (metadata.len(), modified_ms)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ScrubPreviewSegmentMetadata {
    rendition: String,
    segment_cache_key: String,
    video_path: String,
    video_size: u64,
    video_modified_unix_ms: u64,
    frame_index_size: u64,
    frame_index_modified_unix_ms: u64,
}

fn screen_frame_index_existing_path(video_path: &Path) -> Option<PathBuf> {
    let binary = capture_screen::screen_segment_frame_index_path(video_path);
    if binary.is_file() {
        return Some(binary);
    }
    let legacy = capture_screen::legacy_screen_segment_frame_index_path(video_path);
    if legacy.is_file() {
        return Some(legacy);
    }
    None
}

fn segment_cache_key(video_path: &Path, frame_index_path: Option<&Path>) -> String {
    let canonical_video = video_path
        .canonicalize()
        .unwrap_or_else(|_| video_path.to_path_buf());
    let (video_size, video_modified) = file_freshness(&canonical_video);
    let (index_size, index_modified) = frame_index_path.map(file_freshness).unwrap_or((0, 0));
    let mut hasher = Sha256::new();
    hasher.update(canonical_video.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update(video_size.to_le_bytes());
    hasher.update(video_modified.to_le_bytes());
    hasher.update(index_size.to_le_bytes());
    hasher.update(index_modified.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

fn scrub_preview_segment_dir(root: &Path, segment_cache_key: &str) -> PathBuf {
    root.join(segment_cache_key)
}

fn scrub_preview_interval_path(segment_dir: &Path, interval_start_video_offset_ms: u64) -> PathBuf {
    segment_dir.join(format!("{interval_start_video_offset_ms:06}.jpg"))
}

fn scrub_preview_metadata_path(segment_dir: &Path) -> PathBuf {
    segment_dir.join("metadata.json")
}

fn write_scrub_preview_metadata(
    segment_dir: &Path,
    segment_cache_key: &str,
    video_path: &Path,
    frame_index_path: Option<&Path>,
) -> Result<(), String> {
    fs::create_dir_all(segment_dir).map_err(|error| {
        format!(
            "failed to create scrub preview segment directory {}: {error}",
            segment_dir.display()
        )
    })?;
    let canonical_video = video_path
        .canonicalize()
        .unwrap_or_else(|_| video_path.to_path_buf());
    let (video_size, video_modified_unix_ms) = file_freshness(&canonical_video);
    let (frame_index_size, frame_index_modified_unix_ms) =
        frame_index_path.map(file_freshness).unwrap_or((0, 0));
    let metadata = ScrubPreviewSegmentMetadata {
        rendition: SCRUB_PREVIEW_RENDITION.to_string(),
        segment_cache_key: segment_cache_key.to_string(),
        video_path: canonical_video.to_string_lossy().to_string(),
        video_size,
        video_modified_unix_ms,
        frame_index_size,
        frame_index_modified_unix_ms,
    };
    let bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("failed to encode scrub preview metadata: {error}"))?;
    let temp_file = tempfile::NamedTempFile::new_in(segment_dir).map_err(|error| {
        format!(
            "failed to create temporary scrub preview metadata in {}: {error}",
            segment_dir.display()
        )
    })?;
    fs::write(temp_file.path(), bytes).map_err(|error| {
        format!(
            "failed to write temporary scrub preview metadata {}: {error}",
            temp_file.path().display()
        )
    })?;
    temp_file
        .persist(scrub_preview_metadata_path(segment_dir))
        .map_err(|error| format!("failed to persist scrub preview metadata: {error}"))?;
    Ok(())
}

fn scrub_preview_metadata_valid(
    segment_dir: &Path,
    segment_cache_key: &str,
    video_path: &Path,
    frame_index_path: Option<&Path>,
) -> bool {
    let Ok(bytes) = fs::read(scrub_preview_metadata_path(segment_dir)) else {
        return false;
    };
    let Ok(metadata) = serde_json::from_slice::<ScrubPreviewSegmentMetadata>(&bytes) else {
        return false;
    };
    let canonical_video = video_path
        .canonicalize()
        .unwrap_or_else(|_| video_path.to_path_buf());
    let (video_size, video_modified_unix_ms) = file_freshness(&canonical_video);
    let (frame_index_size, frame_index_modified_unix_ms) =
        frame_index_path.map(file_freshness).unwrap_or((0, 0));
    metadata.rendition == SCRUB_PREVIEW_RENDITION
        && metadata.segment_cache_key == segment_cache_key
        && metadata.video_path == canonical_video.to_string_lossy().as_ref()
        && metadata.video_size == video_size
        && metadata.video_modified_unix_ms == video_modified_unix_ms
        && metadata.frame_index_size == frame_index_size
        && metadata.frame_index_modified_unix_ms == frame_index_modified_unix_ms
}

fn indexed_scrub_preview_offsets(
    index: &capture_screen::ScreenSegmentFrameIndex,
) -> HashMap<u64, u64> {
    let mut offsets = HashMap::new();
    for entry in &index.entries {
        let bucket =
            (entry.video_offset_ms / SCRUB_PREVIEW_INTERVAL_MS) * SCRUB_PREVIEW_INTERVAL_MS;
        offsets.entry(bucket).or_insert(entry.video_offset_ms);
    }
    offsets
}

fn scrub_preview_last_bucket(duration_ms: u64, has_indexed_offsets: bool) -> u64 {
    if duration_ms == 0 && has_indexed_offsets {
        return SCRUB_PREVIEW_INTERVAL_MS;
    }
    ((duration_ms + SCRUB_PREVIEW_INTERVAL_MS - 1) / SCRUB_PREVIEW_INTERVAL_MS)
        * SCRUB_PREVIEW_INTERVAL_MS
}

fn scrub_preview_interval_end_unix_ms(
    interval_start_unix_ms: i64,
    segment_ended_unix_ms: i64,
) -> i64 {
    interval_start_unix_ms
        .saturating_add(SCRUB_PREVIEW_INTERVAL_MS as i64)
        .min(segment_ended_unix_ms.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_preview_last_bucket_emits_one_interval_for_indexed_zero_duration_segment() {
        assert_eq!(
            scrub_preview_last_bucket(0, true),
            SCRUB_PREVIEW_INTERVAL_MS
        );
    }

    #[test]
    fn scrub_preview_last_bucket_keeps_empty_zero_duration_segment_empty() {
        assert_eq!(scrub_preview_last_bucket(0, false), 0);
    }

    #[test]
    fn scrub_preview_interval_end_is_exclusive_for_segment_end_frame() {
        assert_eq!(scrub_preview_interval_end_unix_ms(1_000, 1_000), 1_001);
        assert_eq!(scrub_preview_interval_end_unix_ms(1_000, 1_500), 1_501);
        assert_eq!(scrub_preview_interval_end_unix_ms(1_000, 3_000), 2_000);
    }
}

fn scrub_preview_segment_bounds_unix_ms(
    segment_started_unix_ms: i64,
    segment_ended_unix_ms: i64,
    index: &capture_screen::ScreenSegmentFrameIndex,
) -> (i64, i64) {
    let indexed_start = index
        .entries
        .iter()
        .map(|entry| entry.captured_at_unix_ms as i64)
        .min();
    let indexed_end = index
        .entries
        .iter()
        .map(|entry| entry.captured_at_unix_ms as i64)
        .max();

    match (indexed_start, indexed_end) {
        (Some(start), Some(end)) => (
            segment_started_unix_ms.min(start),
            segment_ended_unix_ms.max(end),
        ),
        _ => (segment_started_unix_ms, segment_ended_unix_ms),
    }
}

fn persist_scrub_preview_interval(
    segment_dir: &Path,
    interval_start_video_offset_ms: u64,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    fs::create_dir_all(segment_dir).map_err(|error| {
        format!(
            "failed to create scrub preview segment directory {}: {error}",
            segment_dir.display()
        )
    })?;
    let output_path = scrub_preview_interval_path(segment_dir, interval_start_video_offset_ms);
    let temp_file = tempfile::NamedTempFile::new_in(segment_dir).map_err(|error| {
        format!(
            "failed to create temporary scrub preview file in {}: {error}",
            segment_dir.display()
        )
    })?;
    fs::write(temp_file.path(), bytes).map_err(|error| {
        format!(
            "failed to write temporary scrub preview file {}: {error}",
            temp_file.path().display()
        )
    })?;
    temp_file.persist(&output_path).map_err(|error| {
        format!(
            "failed to persist scrub preview file {}: {error}",
            output_path.display()
        )
    })?;
    Ok(output_path)
}

#[derive(Debug, Clone)]
struct ScrubPreviewGenerationJob {
    segment_cache_key: String,
    segment_dir: PathBuf,
    video_path: PathBuf,
    frame_index_path: Option<PathBuf>,
    started_unix_ms: i64,
    intervals: Vec<(u64, u64)>,
}

async fn generate_scrub_preview_job(job: ScrubPreviewGenerationJob, app_handle: tauri::AppHandle) {
    let keys = job
        .intervals
        .iter()
        .map(|(interval_start, _)| ScrubIntervalWorkKey {
            segment_cache_key: job.segment_cache_key.clone(),
            interval_start_video_offset_ms: *interval_start,
        })
        .collect::<Vec<_>>();

    let accepted = {
        app_handle
            .state::<FramePreviewCacheState>()
            .lock()
            .expect("frame preview cache poisoned")
            .begin_scrub_interval_work(&keys)
    };
    if accepted.is_empty() {
        return;
    }
    let accepted_intervals = job
        .intervals
        .into_iter()
        .filter(|(interval_start, _)| {
            accepted
                .iter()
                .any(|key| key.interval_start_video_offset_ms == *interval_start)
        })
        .collect::<Vec<_>>();

    for chunk in accepted_intervals.chunks(SCRUB_PREVIEW_CHUNK_SIZE) {
        let offsets = chunk
            .iter()
            .map(|(_, selected_offset)| *selected_offset)
            .collect::<Vec<_>>();
        let result = extract_scrub_preview_images_from_video_batch(
            &job.video_path,
            &offsets,
            SCRUB_PREVIEW_MAX_PIXEL_SIZE,
        )
        .await;
        if let Ok(decoded_by_offset) = result {
            let _ = write_scrub_preview_metadata(
                &job.segment_dir,
                &job.segment_cache_key,
                &job.video_path,
                job.frame_index_path.as_deref(),
            );
            for (interval_start, selected_offset) in chunk {
                if let Some(Ok(bytes)) = decoded_by_offset.get(selected_offset) {
                    if let Ok(path) =
                        persist_scrub_preview_interval(&job.segment_dir, *interval_start, bytes)
                    {
                        let _ = allow_preview_file(&app_handle, &path);
                    }
                }
            }
            let start_unix_ms = job.started_unix_ms + chunk[0].0 as i64;
            let end_unix_ms = job.started_unix_ms
                + chunk
                    .last()
                    .map(|(interval_start, _)| *interval_start + SCRUB_PREVIEW_INTERVAL_MS)
                    .unwrap_or(0) as i64;
            let _ = app_handle.emit(
                SCRUB_PREVIEW_CACHE_CHANGED_EVENT,
                ScrubPreviewCacheChangedPayload {
                    rendition: SCRUB_PREVIEW_RENDITION.to_string(),
                    start_unix_ms,
                    end_unix_ms,
                },
            );
        }
    }

    app_handle
        .state::<FramePreviewCacheState>()
        .lock()
        .expect("frame preview cache poisoned")
        .finish_scrub_interval_work(&accepted);
}

#[tauri::command]
pub async fn get_scrub_preview_availability(
    request: GetScrubPreviewAvailabilityRequest,
    state: tauri::State<'_, AppInfraState>,
    app_handle: tauri::AppHandle,
) -> Result<ScrubPreviewAvailabilityDto, String> {
    let start_unix_ms = request.start_unix_ms.min(request.end_unix_ms);
    let end_unix_ms = request.start_unix_ms.max(request.end_unix_ms);
    let enqueue_missing = request.enqueue_missing.unwrap_or(true);
    let infra = Arc::clone(&*state);
    let cache_root = scrub_preview_cache_root(&app_handle)?;
    let segments = infra
        .list_finalized_screen_segments_overlapping_window(
            &unix_ms_to_rfc3339(start_unix_ms),
            &unix_ms_to_rfc3339(end_unix_ms),
        )
        .await
        .map_err(|error| format!("failed to list screen segments for scrub previews: {error}"))?;

    let mut intervals = Vec::new();
    let mut jobs = Vec::new();
    for segment in segments {
        let segment_started_unix_ms =
            rfc3339_to_unix_ms(&segment.started_at).unwrap_or(start_unix_ms);
        let segment_ended_unix_ms =
            rfc3339_to_unix_ms(&segment.ended_at).unwrap_or(segment_started_unix_ms);
        let video_path = PathBuf::from(&segment.media_file_path);
        let frame_index_path = screen_frame_index_existing_path(&video_path);
        let segment_cache_key = segment_cache_key(&video_path, frame_index_path.as_deref());
        let segment_dir = scrub_preview_segment_dir(&cache_root, &segment_cache_key);
        let metadata_valid = scrub_preview_metadata_valid(
            &segment_dir,
            &segment_cache_key,
            &video_path,
            frame_index_path.as_deref(),
        );

        if !video_path.is_file()
            || fs::metadata(&video_path)
                .map(|metadata| metadata.len() == 0)
                .unwrap_or(true)
            || !mov_file_appears_openable_for_preview(&video_path).unwrap_or(false)
        {
            intervals.push(ScrubPreviewAvailabilityIntervalDto {
                segment_cache_key,
                interval_start_video_offset_ms: 0,
                interval_end_video_offset_ms: SCRUB_PREVIEW_INTERVAL_MS,
                interval_start_unix_ms: segment_started_unix_ms,
                interval_end_unix_ms: segment_ended_unix_ms,
                preview: None,
                status: ScrubPreviewAvailabilityStatusDto::SourceMissing,
            });
            continue;
        }

        let index = match load_screen_segment_frame_index(&video_path) {
            Ok(Some(index)) => index,
            Ok(None) => {
                intervals.push(ScrubPreviewAvailabilityIntervalDto {
                    segment_cache_key,
                    interval_start_video_offset_ms: 0,
                    interval_end_video_offset_ms: SCRUB_PREVIEW_INTERVAL_MS,
                    interval_start_unix_ms: segment_started_unix_ms,
                    interval_end_unix_ms: segment_ended_unix_ms,
                    preview: None,
                    status: ScrubPreviewAvailabilityStatusDto::FrameIndexMissing,
                });
                continue;
            }
            Err(_) => {
                intervals.push(ScrubPreviewAvailabilityIntervalDto {
                    segment_cache_key,
                    interval_start_video_offset_ms: 0,
                    interval_end_video_offset_ms: SCRUB_PREVIEW_INTERVAL_MS,
                    interval_start_unix_ms: segment_started_unix_ms,
                    interval_end_unix_ms: segment_ended_unix_ms,
                    preview: None,
                    status: ScrubPreviewAvailabilityStatusDto::DecodeFailed,
                });
                continue;
            }
        };

        let indexed_offsets = indexed_scrub_preview_offsets(&index);
        let (segment_started_unix_ms, segment_ended_unix_ms) = scrub_preview_segment_bounds_unix_ms(
            segment_started_unix_ms,
            segment_ended_unix_ms,
            &index,
        );
        let duration_ms = (segment_ended_unix_ms - segment_started_unix_ms).max(0) as u64;
        let last_bucket = scrub_preview_last_bucket(duration_ms, !indexed_offsets.is_empty());
        let mut missing_for_generation = Vec::new();
        let mut bucket = 0;
        while bucket < last_bucket {
            let interval_start_unix_ms = segment_started_unix_ms + bucket as i64;
            let interval_end_unix_ms =
                scrub_preview_interval_end_unix_ms(interval_start_unix_ms, segment_ended_unix_ms);
            if interval_end_unix_ms < start_unix_ms || interval_start_unix_ms > end_unix_ms {
                bucket += SCRUB_PREVIEW_INTERVAL_MS;
                continue;
            }
            let interval_path = scrub_preview_interval_path(&segment_dir, bucket);
            if metadata_valid && interval_path.is_file() {
                allow_preview_file(&app_handle, &interval_path)?;
                intervals.push(ScrubPreviewAvailabilityIntervalDto {
                    segment_cache_key: segment_cache_key.clone(),
                    interval_start_video_offset_ms: bucket,
                    interval_end_video_offset_ms: bucket + SCRUB_PREVIEW_INTERVAL_MS,
                    interval_start_unix_ms,
                    interval_end_unix_ms,
                    preview: Some(ScrubPreviewAvailabilityPreviewDto {
                        file_path: interval_path.to_string_lossy().to_string(),
                        mime_type: "image/jpeg".to_string(),
                        source_kind: FramePreviewSourceKindDto::ScrubPreview,
                    }),
                    status: ScrubPreviewAvailabilityStatusDto::Ready,
                });
            } else if let Some(selected_offset) = indexed_offsets.get(&bucket) {
                if cfg!(target_os = "macos") && enqueue_missing {
                    missing_for_generation.push((bucket, *selected_offset));
                }
                intervals.push(ScrubPreviewAvailabilityIntervalDto {
                    segment_cache_key: segment_cache_key.clone(),
                    interval_start_video_offset_ms: bucket,
                    interval_end_video_offset_ms: bucket + SCRUB_PREVIEW_INTERVAL_MS,
                    interval_start_unix_ms,
                    interval_end_unix_ms,
                    preview: None,
                    status: if !cfg!(target_os = "macos") {
                        ScrubPreviewAvailabilityStatusDto::UnsupportedPlatform
                    } else if enqueue_missing {
                        ScrubPreviewAvailabilityStatusDto::Queued
                    } else {
                        ScrubPreviewAvailabilityStatusDto::NotIndexed
                    },
                });
            } else {
                intervals.push(ScrubPreviewAvailabilityIntervalDto {
                    segment_cache_key: segment_cache_key.clone(),
                    interval_start_video_offset_ms: bucket,
                    interval_end_video_offset_ms: bucket + SCRUB_PREVIEW_INTERVAL_MS,
                    interval_start_unix_ms,
                    interval_end_unix_ms,
                    preview: None,
                    status: ScrubPreviewAvailabilityStatusDto::NotIndexed,
                });
            }
            bucket += SCRUB_PREVIEW_INTERVAL_MS;
        }

        if !missing_for_generation.is_empty() {
            jobs.push(ScrubPreviewGenerationJob {
                segment_cache_key,
                segment_dir,
                video_path,
                frame_index_path,
                started_unix_ms: segment_started_unix_ms,
                intervals: missing_for_generation,
            });
        }
    }

    for job in jobs {
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            generate_scrub_preview_job(job, app_handle).await;
        });
    }

    Ok(ScrubPreviewAvailabilityDto {
        rendition: SCRUB_PREVIEW_RENDITION.to_string(),
        intervals,
    })
}

#[tauri::command]
pub fn get_scrub_preview_cache_status(
    app_handle: tauri::AppHandle,
) -> Result<ScrubPreviewCacheStatusDto, String> {
    let cache_root = scrub_preview_cache_root(&app_handle)?;
    let mut segment_directories = 0usize;
    let mut preview_files = 0usize;
    let mut total_bytes = 0u64;
    if cache_root.is_dir() {
        for entry in fs::read_dir(&cache_root).map_err(|error| {
            format!(
                "failed to read scrub preview cache directory {}: {error}",
                cache_root.display()
            )
        })? {
            let Ok(entry) = entry else { continue };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                segment_directories += 1;
                let Ok(files) = fs::read_dir(entry.path()) else {
                    continue;
                };
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().and_then(|ext| ext.to_str()) != Some("jpg") {
                        continue;
                    }
                    if let Ok(metadata) = file.metadata() {
                        preview_files += 1;
                        total_bytes = total_bytes.saturating_add(metadata.len());
                    }
                }
            }
        }
    }
    Ok(ScrubPreviewCacheStatusDto {
        rendition: SCRUB_PREVIEW_RENDITION.to_string(),
        cache_directory: cache_root.to_string_lossy().to_string(),
        segment_directories,
        preview_files,
        total_bytes,
    })
}

#[tauri::command]
pub fn clear_scrub_preview_cache(
    app_handle: tauri::AppHandle,
) -> Result<ScrubPreviewCacheStatusDto, String> {
    let cache_root = scrub_preview_cache_root(&app_handle)?;
    if cache_root.is_dir() {
        for entry in fs::read_dir(&cache_root).map_err(|error| {
            format!(
                "failed to read scrub preview cache directory {}: {error}",
                cache_root.display()
            )
        })? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
    get_scrub_preview_cache_status(app_handle)
}

pub fn clear_scrub_preview_cache_for_video_paths(
    app_handle: tauri::AppHandle,
    video_paths: &[String],
) -> Result<ScrubPreviewCacheStatusDto, String> {
    let cache_root = scrub_preview_cache_root(&app_handle)?;
    if !cache_root.is_dir() || video_paths.is_empty() {
        return get_scrub_preview_cache_status(app_handle);
    }

    let target_paths = video_paths
        .iter()
        .map(|path| {
            let path = PathBuf::from(path);
            path.canonicalize()
                .unwrap_or(path)
                .to_string_lossy()
                .to_string()
        })
        .collect::<HashSet<_>>();

    for entry in fs::read_dir(&cache_root).map_err(|error| {
        format!(
            "failed to read scrub preview cache directory {}: {error}",
            cache_root.display()
        )
    })? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(bytes) = fs::read(scrub_preview_metadata_path(&path)) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_slice::<ScrubPreviewSegmentMetadata>(&bytes) else {
            continue;
        };
        let metadata_video_path = {
            let path = PathBuf::from(&metadata.video_path);
            path.canonicalize()
                .unwrap_or(path)
                .to_string_lossy()
                .to_string()
        };
        if target_paths.contains(&metadata_video_path) {
            let _ = fs::remove_dir_all(path);
        }
    }

    get_scrub_preview_cache_status(app_handle)
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
        return get_frame_preview_inner_with_logging(
            &infra,
            &cache,
            Some(&app_handle),
            request.frame_id,
        )
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
            let result = get_frame_preview_inner_with_logging(
                &infra,
                &cache,
                Some(&app_handle),
                request.frame_id,
            )
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
pub async fn get_frame_scrub_previews(
    request: GetFrameScrubPreviewsRequest,
    state: tauri::State<'_, AppInfraState>,
    cache: tauri::State<'_, FramePreviewCacheState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
    app_handle: tauri::AppHandle,
) -> Result<FrameScrubPreviewsDto, String> {
    let started_at = Instant::now();
    let infra = Arc::clone(&*state);
    let ttl = preview_cache_ttl(&settings);
    let max_pixel_size = clamp_scrub_preview_max_pixel_size(request.max_pixel_size);
    let requested_count = request.frame_ids.len();
    let mut unique_frame_ids = Vec::new();
    for frame_id in &request.frame_ids {
        if !unique_frame_ids.contains(frame_id) {
            unique_frame_ids.push(*frame_id);
        }
    }
    let unique_count = unique_frame_ids.len();
    let scrub_cache_dir_result = ensure_generated_frame_scrub_preview_cache_dir(&app_handle);
    let scrub_cache_dir = scrub_cache_dir_result.as_ref().ok().map(PathBuf::as_path);

    let mut unique_results = HashMap::new();
    let mut video_batches: HashMap<PathBuf, Vec<PreparedVideoScrubPreview>> = HashMap::new();
    let mut cached_count = 0usize;
    let mut generated_count = 0usize;
    let mut missing_count = 0usize;
    for frame_id in unique_frame_ids {
        let frame_started_at = Instant::now();
        if !ttl.is_zero() {
            let cached_preview = cache
                .lock()
                .expect("frame preview cache poisoned")
                .get_scrub(frame_id, max_pixel_size, ttl, frame_started_at);
            if let Some(preview) = cached_preview {
                cached_count += 1;
                unique_results.insert(
                    frame_id,
                    FrameScrubPreviewResultDto {
                        frame_id,
                        preview: Some(preview),
                        missing_reason: None,
                    },
                );
                continue;
            }
        }

        match prepare_frame_scrub_preview(
            &infra,
            &app_handle,
            frame_id,
            max_pixel_size,
            scrub_cache_dir,
        )
        .await?
        {
            PreparedFrameScrubPreview::Ready(result) => {
                let source_kind = result
                    .preview
                    .as_ref()
                    .map(|preview| format!("{:?}", preview.source_kind))
                    .unwrap_or_else(|| {
                        missing_count += 1;
                        "missing".to_string()
                    });
                if result.preview.is_some() {
                    generated_count += 1;
                }
                let frame_duration_ms = frame_started_at.elapsed().as_millis();
                if frame_duration_ms >= SCRUB_PREVIEW_PERF_LOG_THRESHOLD_MS {
                    log_scrub_preview_perf(
                        "rust_scrub_preview_frame",
                        format!(
                            "frameId={frame_id} durationMs={frame_duration_ms} sourceKind={source_kind}"
                        ),
                    );
                }
                if !ttl.is_zero() {
                    if let Some(preview) = result.preview.as_ref() {
                        cache
                            .lock()
                            .expect("frame preview cache poisoned")
                            .insert_scrub(
                                frame_id,
                                max_pixel_size,
                                preview.clone(),
                                ttl,
                                Instant::now(),
                            );
                    }
                }
                unique_results.insert(frame_id, result);
            }
            PreparedFrameScrubPreview::Video(preview) => {
                video_batches
                    .entry(preview.video_path.clone())
                    .or_default()
                    .push(preview);
            }
        }
    }

    for (video_path, candidates) in video_batches {
        let Some(cache_dir) = scrub_cache_dir else {
            for candidate in candidates {
                missing_count += 1;
                unique_results.insert(
                    candidate.frame_id,
                    FrameScrubPreviewResultDto {
                        frame_id: candidate.frame_id,
                        preview: None,
                        missing_reason: Some(ScrubPreviewMissingReasonDto::CacheWriteFailed),
                    },
                );
            }
            continue;
        };

        let mut pending = candidates;
        loop {
            let mut still_pending = Vec::new();
            for candidate in pending {
                still_pending.push(candidate);
            }

            if still_pending.is_empty() {
                break;
            }

            let video_request_guard = {
                let mut preview_state = cache.lock().expect("frame preview cache poisoned");
                match preview_state.begin_video_request(&video_path) {
                    Ok(()) => Ok(()),
                    Err(rx) => Err(rx),
                }
            };

            match video_request_guard {
                Ok(()) => {
                    let batch_started_at = Instant::now();
                    let mut offsets = Vec::new();
                    for candidate in &still_pending {
                        if !offsets.contains(&candidate.video_offset_ms) {
                            offsets.push(candidate.video_offset_ms);
                        }
                    }
                    let result = extract_scrub_preview_images_from_video_batch(
                        &video_path,
                        &offsets,
                        max_pixel_size,
                    )
                    .await;
                    let notify_result = result.as_ref().map(|_| ()).map_err(Clone::clone);
                    cache
                        .lock()
                        .expect("frame preview cache poisoned")
                        .finish_video_request(&video_path, notify_result);

                    let batch_duration_ms = batch_started_at.elapsed().as_millis();
                    if batch_duration_ms >= SCRUB_PREVIEW_PERF_LOG_THRESHOLD_MS {
                        log_scrub_preview_perf(
                            "rust_scrub_preview_segment_batch",
                            format!(
                                "path={} frames={} offsets={} durationMs={batch_duration_ms}",
                                video_path.display(),
                                still_pending.len(),
                                offsets.len(),
                            ),
                        );
                    }

                    let decoded_by_offset = match result {
                        Ok(decoded_by_offset) => decoded_by_offset,
                        Err(_) => {
                            for candidate in still_pending {
                                missing_count += 1;
                                unique_results.insert(
                                    candidate.frame_id,
                                    FrameScrubPreviewResultDto {
                                        frame_id: candidate.frame_id,
                                        preview: None,
                                        missing_reason: Some(
                                            ScrubPreviewMissingReasonDto::DecodeFailed,
                                        ),
                                    },
                                );
                            }
                            break;
                        }
                    };

                    for candidate in still_pending {
                        let frame_result = match decoded_by_offset.get(&candidate.video_offset_ms) {
                            Some(Ok(bytes)) => match persist_generated_scrub_preview_in_dir(
                                cache_dir,
                                candidate.frame_id,
                                max_pixel_size,
                                &candidate.source_hash,
                                bytes,
                            ) {
                                Ok(path) => FrameScrubPreviewResultDto {
                                    frame_id: candidate.frame_id,
                                    preview: Some(frame_preview_payload(
                                        path.to_string_lossy(),
                                        "image/jpeg",
                                        FramePreviewSourceKindDto::ScrubPreview,
                                    )),
                                    missing_reason: None,
                                },
                                Err(_) => FrameScrubPreviewResultDto {
                                    frame_id: candidate.frame_id,
                                    preview: None,
                                    missing_reason: Some(
                                        ScrubPreviewMissingReasonDto::CacheWriteFailed,
                                    ),
                                },
                            },
                            _ => FrameScrubPreviewResultDto {
                                frame_id: candidate.frame_id,
                                preview: None,
                                missing_reason: Some(ScrubPreviewMissingReasonDto::DecodeFailed),
                            },
                        };

                        if let Some(preview) = frame_result.preview.as_ref() {
                            generated_count += 1;
                            if !ttl.is_zero() {
                                cache
                                    .lock()
                                    .expect("frame preview cache poisoned")
                                    .insert_scrub(
                                        candidate.frame_id,
                                        max_pixel_size,
                                        preview.clone(),
                                        ttl,
                                        Instant::now(),
                                    );
                            }
                        } else {
                            missing_count += 1;
                        }
                        unique_results.insert(candidate.frame_id, frame_result);
                    }
                    break;
                }
                Err(waiter) => match waiter.await {
                    Ok(Ok(())) => {
                        pending = still_pending;
                    }
                    _ => {
                        for candidate in still_pending {
                            missing_count += 1;
                            unique_results.insert(
                                candidate.frame_id,
                                FrameScrubPreviewResultDto {
                                    frame_id: candidate.frame_id,
                                    preview: None,
                                    missing_reason: Some(
                                        ScrubPreviewMissingReasonDto::DecodeFailed,
                                    ),
                                },
                            );
                        }
                        break;
                    }
                },
            }
        }
    }

    let total_duration_ms = started_at.elapsed().as_millis();
    if total_duration_ms >= SCRUB_PREVIEW_PERF_LOG_THRESHOLD_MS {
        log_scrub_preview_perf(
            "rust_scrub_preview_batch",
            format!(
                "requested={requested_count} unique={unique_count} cached={cached_count} generated={generated_count} missing={missing_count} durationMs={total_duration_ms}"
            ),
        );
    }

    Ok(FrameScrubPreviewsDto {
        previews: request
            .frame_ids
            .into_iter()
            .map(|frame_id| {
                unique_results
                    .get(&frame_id)
                    .cloned()
                    .unwrap_or(FrameScrubPreviewResultDto {
                        frame_id,
                        preview: None,
                        missing_reason: Some(ScrubPreviewMissingReasonDto::FrameNotFound),
                    })
            })
            .collect(),
    })
}
