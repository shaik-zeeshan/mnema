use std::{
    collections::HashMap,
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
use tauri::{path::BaseDirectory, Manager};
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
const GENERATED_FRAME_PREVIEW_CACHE_DIR: &str = "frame-previews";
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

fn generated_scrub_preview_file_name(frame_id: i64, max_pixel_size: u32) -> String {
    format!("scrub-v2-frame-{frame_id}-{max_pixel_size}.jpg")
}

fn generated_scrub_preview_path(cache_dir: &Path, frame_id: i64, max_pixel_size: u32) -> PathBuf {
    cache_dir.join(generated_scrub_preview_file_name(frame_id, max_pixel_size))
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
    bytes: &[u8],
) -> Result<PathBuf, String> {
    fs::create_dir_all(cache_dir).map_err(|error| {
        format!(
            "failed to create preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    let output_path = generated_scrub_preview_path(cache_dir, frame_id, max_pixel_size);
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
    let cached_path = generated_scrub_preview_path(cache_dir, frame_id, max_pixel_size);
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
        let cached_path = generated_scrub_preview_path(cache_dir, frame_id, max_pixel_size);
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
    let scrub_cache_dir_result = ensure_generated_frame_preview_cache_dir(&app_handle);
    let scrub_cache_dir = scrub_cache_dir_result.as_ref().ok().map(PathBuf::as_path);

    let mut unique_results = HashMap::new();
    let mut video_batches: HashMap<PathBuf, Vec<PreparedVideoScrubPreview>> = HashMap::new();
    let mut cached_count = 0usize;
    let mut generated_count = 0usize;
    let mut missing_count = 0usize;
    for frame_id in unique_frame_ids {
        let frame_started_at = Instant::now();
        if !ttl.is_zero() {
            let cached = cache
                .lock()
                .expect("frame preview cache poisoned")
                .get_scrub(frame_id, max_pixel_size, ttl, Instant::now());
            if let Some(preview) = cached {
                unique_results.insert(
                    frame_id,
                    FrameScrubPreviewResultDto {
                        frame_id,
                        preview: Some(preview),
                        missing_reason: None,
                    },
                );
                cached_count += 1;
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
                if !ttl.is_zero() {
                    let cached = cache
                        .lock()
                        .expect("frame preview cache poisoned")
                        .get_scrub(candidate.frame_id, max_pixel_size, ttl, Instant::now());
                    if let Some(preview) = cached {
                        unique_results.insert(
                            candidate.frame_id,
                            FrameScrubPreviewResultDto {
                                frame_id: candidate.frame_id,
                                preview: Some(preview),
                                missing_reason: None,
                            },
                        );
                        cached_count += 1;
                        continue;
                    }
                }
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
