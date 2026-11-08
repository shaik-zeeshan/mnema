//! Mapping a frame to its segment video and its offset in it, and the
//! persisted-segment-frame fallback.

use super::*;

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

pub(super) fn parse_frame_unix_ms_from_path(frame_file_path: &Path) -> Option<i128> {
    let stem = frame_file_path.file_stem()?.to_str()?;
    let raw = stem.strip_prefix("frame-")?;
    let (unix_ms, _) = raw.rsplit_once('-')?;
    unix_ms.parse().ok()
}

pub(super) fn parse_frame_identity_from_path(frame_file_path: &Path) -> Option<(u64, u64)> {
    let stem = frame_file_path.file_stem()?.to_str()?;
    let raw = stem.strip_prefix("frame-")?;
    let (captured_at_unix_ms, frame_index) = raw.rsplit_once('-')?;
    Some((captured_at_unix_ms.parse().ok()?, frame_index.parse().ok()?))
}

pub(super) fn parse_captured_at_unix_ms(captured_at: &str) -> Option<i128> {
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

pub(super) fn frame_preview_unix_ms(frame: &::app_infra::Frame) -> Option<i128> {
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

pub(super) fn load_screen_segment_frame_index(
    video_path: &Path,
) -> std::io::Result<Option<capture_screen::ScreenSegmentFrameIndex>> {
    let index_path = capture_screen::screen_segment_frame_index_path(video_path);
    if !index_path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&index_path)?;
    capture_screen::decode_screen_segment_frame_index(&bytes)
        .map(Some)
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse screen segment frame index {}: {error}",
                    index_path.display()
                ),
            )
        })
}

pub(super) fn find_indexed_frame_preview_offset(
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

// A sibling frame further away than this is a different moment on screen —
// serving it under this frame's id lies to the metadata overlay. Beyond the
// cap the caller returns the underlying video error instead.
pub(super) const SEGMENT_FRAME_FALLBACK_MAX_DISTANCE_MS: i128 = 5_000;

pub(super) fn read_nearest_segment_frame_preview(
    frame: &::app_infra::Frame,
    related_frames: &[::app_infra::Frame],
) -> std::io::Result<Option<Vec<u8>>> {
    let Some(target_unix_ms) = frame_preview_unix_ms(frame) else {
        return Ok(None);
    };
    let mut best_match: Option<(i128, &str)> = None;

    for related_frame in related_frames {
        let candidate_path = Path::new(&related_frame.file_path);
        if !candidate_path.is_file() {
            continue;
        }
        let Some(candidate_unix_ms) = frame_preview_unix_ms(related_frame) else {
            continue;
        };
        let distance = (target_unix_ms - candidate_unix_ms).abs();
        if distance > SEGMENT_FRAME_FALLBACK_MAX_DISTANCE_MS {
            continue;
        }
        if best_match.is_none_or(|(best_distance, _)| distance < best_distance) {
            best_match = Some((distance, &related_frame.file_path));
        }
    }

    best_match
        .map(|(_, file_path)| fs::read(file_path))
        .transpose()
}

pub(super) fn read_segment_frame_preview_or_return_video_error(
    frame: &::app_infra::Frame,
    _infra: &::app_infra::AppInfra,
    related_frames: &[::app_infra::Frame],
    video_path: &Path,
    core: Option<&AppCore>,
    secret_redaction_count: u32,
    video_error: impl Into<String>,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let video_error = video_error.into();

    if let Some(bytes) = read_nearest_segment_frame_preview(frame, related_frames)? {
        crate::debug_log::log_warn(format!(
            "[DEBUG-frame-preview] frame_id={} falling back to persisted segment frame after video preview failure at {}: {}",
            frame.id,
            video_path.display(),
            video_error,
        ));
        let persisted_path = if let Some(core) = core {
            persist_generated_frame_preview(
                core,
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
            secret_redaction_count,
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
