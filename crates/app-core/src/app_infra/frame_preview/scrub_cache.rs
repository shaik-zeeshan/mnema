//! The scrub-preview cache on disk: the startup prune, its root, per-segment
//! metadata and the interval buckets.

use super::*;

pub fn run_generated_frame_preview_cache_startup_pass(core: &AppCore) {
    let cache_dir = core.paths().cache.join(GENERATED_FRAME_PREVIEW_CACHE_DIR);
    if let Err(error) = cleanup_generated_frame_preview_cache_dir(&cache_dir) {
        crate::debug_log::log_warn(format!(
            "failed generated frame preview cache startup cleanup at {}: {error}",
            cache_dir.display()
        ));
    }

    let cache_dir = core.paths().cache.join(GENERATED_SCRUB_PREVIEW_CACHE_DIR);
    let frame_scrub_cache_dir = cache_dir.join(GENERATED_FRAME_SCRUB_PREVIEW_CACHE_DIR);
    if let Err(error) = cleanup_generated_frame_preview_cache_dir(&frame_scrub_cache_dir) {
        crate::debug_log::log_warn(format!(
            "failed generated frame scrub preview cache startup cleanup at {}: {error}",
            frame_scrub_cache_dir.display()
        ));
    }
    let cache_dir = cache_dir.join(SCRUB_PREVIEW_RENDITION);
    if let Err(error) = cleanup_generated_scrub_preview_cache_dir(&cache_dir) {
        crate::debug_log::log_warn(format!(
            "failed generated scrub preview cache startup cleanup at {}: {error}",
            cache_dir.display()
        ));
    }
}

pub(super) fn scrub_preview_cache_root(core: &AppCore) -> Result<PathBuf, String> {
    let cache_dir = core
        .paths()
        .cache
        .join(GENERATED_SCRUB_PREVIEW_CACHE_DIR)
        .join(SCRUB_PREVIEW_RENDITION);
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create scrub preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    allow_preview_dir(core, &cache_dir);
    cleanup_generated_scrub_preview_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

pub(super) fn unix_ms_to_rfc3339(unix_ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub(super) fn rfc3339_to_unix_ms(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(|dt| i64::try_from(dt.unix_timestamp_nanos() / 1_000_000).ok())
}

pub(super) fn file_freshness(path: &Path) -> (u64, u64) {
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
pub(super) struct ScrubPreviewSegmentMetadata {
    pub(super) rendition: String,
    pub(super) segment_cache_key: String,
    pub(super) video_path: String,
    pub(super) video_size: u64,
    pub(super) video_modified_unix_ms: u64,
    pub(super) frame_index_size: u64,
    pub(super) frame_index_modified_unix_ms: u64,
}

pub(super) fn screen_frame_index_existing_path(video_path: &Path) -> Option<PathBuf> {
    let binary = capture_screen::screen_segment_frame_index_path(video_path);
    binary.is_file().then_some(binary)
}

pub(super) fn segment_cache_key(video_path: &Path, frame_index_path: Option<&Path>) -> String {
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

pub(super) fn scrub_preview_segment_dir(root: &Path, segment_cache_key: &str) -> PathBuf {
    root.join(segment_cache_key)
}

pub(super) fn scrub_preview_interval_path(
    segment_dir: &Path,
    interval_start_video_offset_ms: u64,
) -> PathBuf {
    segment_dir.join(format!("{interval_start_video_offset_ms:06}.jpg"))
}

pub(super) fn scrub_preview_metadata_path(segment_dir: &Path) -> PathBuf {
    segment_dir.join("metadata.json")
}

pub(super) fn write_scrub_preview_metadata(
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

pub(super) fn scrub_preview_metadata_valid(
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

pub(super) fn indexed_scrub_preview_offsets(
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

pub(super) fn scrub_preview_last_bucket(duration_ms: u64, has_indexed_offsets: bool) -> u64 {
    if duration_ms == 0 && has_indexed_offsets {
        return SCRUB_PREVIEW_INTERVAL_MS;
    }
    ((duration_ms + SCRUB_PREVIEW_INTERVAL_MS - 1) / SCRUB_PREVIEW_INTERVAL_MS)
        * SCRUB_PREVIEW_INTERVAL_MS
}

pub(super) fn scrub_preview_interval_end_unix_ms(
    interval_start_unix_ms: i64,
    segment_ended_unix_ms: i64,
    next_segment_started_unix_ms: Option<i64>,
) -> i64 {
    let inclusive_segment_end = interval_start_unix_ms
        .saturating_add(SCRUB_PREVIEW_INTERVAL_MS as i64)
        .min(segment_ended_unix_ms.saturating_add(1));
    match next_segment_started_unix_ms {
        Some(next_start) => inclusive_segment_end
            .min(next_start)
            .max(interval_start_unix_ms),
        _ => inclusive_segment_end,
    }
}

pub(super) fn scrub_preview_adjacent_next_segment_started_unix_ms(
    next_segment_started_unix_ms: Option<i64>,
    segment_ended_unix_ms: i64,
) -> Option<i64> {
    next_segment_started_unix_ms.filter(|next_start| *next_start <= segment_ended_unix_ms)
}

pub(super) fn scrub_preview_segment_bounds_unix_ms(
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

pub(super) fn persist_scrub_preview_interval(
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
