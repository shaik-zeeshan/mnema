//! Generated preview files on disk: names, cache-dir pruning, the asset-scope
//! grants and the persisted derivatives.

use super::*;

pub(super) fn frame_preview_payload(
    file_path: impl Into<String>,
    mime_type: &str,
    source_kind: FramePreviewSourceKindDto,
    secret_redaction_count: u32,
) -> FramePreviewDto {
    FramePreviewDto {
        mime_type: mime_type.to_string(),
        file_path: file_path.into(),
        source_kind,
        has_secret_redactions: secret_redaction_count > 0,
        secret_redaction_count,
    }
}

pub(super) fn generated_frame_preview_file_name(frame_id: i64, mime_type: &str) -> String {
    let ext = match mime_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    format!("frame-{frame_id}.{ext}")
}

pub(super) fn scrub_preview_source_hash(source_path: &Path, extra: impl AsRef<str>) -> String {
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

pub(super) fn generated_scrub_preview_file_name(
    frame_id: i64,
    max_pixel_size: u32,
    source_hash: &str,
) -> String {
    format!("scrub-v3-frame-{frame_id}-{max_pixel_size}-{source_hash}.jpg")
}

pub(super) fn generated_scrub_preview_path(
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

pub(super) fn clamp_scrub_preview_max_pixel_size(max_pixel_size: Option<u32>) -> u32 {
    max_pixel_size
        .unwrap_or(SCRUB_PREVIEW_DEFAULT_MAX_PIXEL_SIZE)
        .clamp(
            SCRUB_PREVIEW_MIN_MAX_PIXEL_SIZE,
            SCRUB_PREVIEW_MAX_MAX_PIXEL_SIZE,
        )
}

pub(super) fn cleanup_generated_frame_preview_cache_dir(cache_dir: &Path) -> Result<(), String> {
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

pub(super) fn cleanup_generated_scrub_preview_cache_dir(cache_root: &Path) -> Result<(), String> {
    cleanup_generated_scrub_preview_cache_dir_with_limits(
        cache_root,
        GENERATED_SCRUB_PREVIEW_CACHE_MAX_FILES,
        GENERATED_SCRUB_PREVIEW_CACHE_MAX_BYTES,
        GENERATED_SCRUB_PREVIEW_CACHE_MAX_AGE,
    )
}

pub(super) fn cleanup_generated_scrub_preview_cache_dir_with_limits(
    cache_root: &Path,
    max_files: usize,
    max_bytes: u64,
    max_age: Duration,
) -> Result<(), String> {
    if !cache_root.is_dir() {
        return Ok(());
    }

    let now = std::time::SystemTime::now();
    let mut files = Vec::new();
    for entry in fs::read_dir(cache_root).map_err(|error| {
        format!(
            "failed to read scrub preview cache directory {}: {error}",
            cache_root.display()
        )
    })? {
        let Ok(entry) = entry else { continue };
        let segment_dir = entry.path();
        if !segment_dir.is_dir() {
            continue;
        }
        let Ok(segment_files) = fs::read_dir(&segment_dir) else {
            continue;
        };
        for file in segment_files.flatten() {
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jpg") {
                continue;
            }
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let modified = metadata
                .modified()
                .ok()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((path, modified, metadata.len()));
        }
    }

    for (path, modified, _) in &files {
        if now.duration_since(*modified).unwrap_or_default() > max_age {
            let _ = fs::remove_file(path);
        }
    }

    files.retain(|(path, _, _)| path.is_file());
    files.sort_by_key(|(_, modified, _)| *modified);
    let mut total_bytes = files.iter().map(|(_, _, len)| *len).sum::<u64>();
    while files.len() > max_files || total_bytes > max_bytes {
        let (path, _, len) = files.remove(0);
        if fs::remove_file(&path).is_ok() {
            total_bytes = total_bytes.saturating_sub(len);
        }
    }

    for entry in fs::read_dir(cache_root).map_err(|error| {
        format!(
            "failed to read scrub preview cache directory {}: {error}",
            cache_root.display()
        )
    })? {
        let Ok(entry) = entry else { continue };
        let segment_dir = entry.path();
        if !segment_dir.is_dir() {
            continue;
        }
        let has_preview = fs::read_dir(&segment_dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|file| file.path().extension().and_then(|ext| ext.to_str()) == Some("jpg"));
        if !has_preview {
            let _ = fs::remove_dir_all(segment_dir);
        }
    }

    Ok(())
}

pub(super) fn ensure_generated_frame_preview_cache_dir(core: &AppCore) -> Result<PathBuf, String> {
    let cache_dir = core.paths().cache.join(GENERATED_FRAME_PREVIEW_CACHE_DIR);
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create app preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    allow_preview_dir(core, &cache_dir);
    cleanup_generated_frame_preview_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

pub(super) fn ensure_generated_frame_scrub_preview_cache_dir(
    core: &AppCore,
) -> Result<PathBuf, String> {
    let cache_dir = core
        .paths()
        .cache
        .join(GENERATED_SCRUB_PREVIEW_CACHE_DIR)
        .join(GENERATED_FRAME_SCRUB_PREVIEW_CACHE_DIR);
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create app frame scrub preview cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    allow_preview_dir(core, &cache_dir);
    cleanup_generated_frame_preview_cache_dir(&cache_dir)?;
    Ok(cache_dir)
}

pub(super) fn allow_preview_file(core: &AppCore, file_path: &Path) {
    core.host().window_signal(WindowSignal::AllowAssetPath {
        path: file_path.to_path_buf(),
        recursive: false,
    });
}

pub(super) fn allow_preview_dir(core: &AppCore, dir: &Path) {
    core.host().window_signal(WindowSignal::AllowAssetPath {
        path: dir.to_path_buf(),
        recursive: true,
    });
}

pub(super) fn persist_generated_frame_preview_in_dir(
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
    // Always overwrite: a later tier can produce a better image than what an
    // earlier fallback persisted under the same frame id (e.g. a nearest-
    // sibling image written while the video was still finalizing must not
    // shadow a later exact video extract).
    let created = !output_path.is_file();
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
    if created {
        cleanup_generated_frame_preview_cache_dir(cache_dir)?;
    }
    Ok(output_path)
}

pub(super) fn persist_generated_frame_preview(
    core: &AppCore,
    frame_id: i64,
    bytes: &[u8],
    mime_type: &str,
) -> Result<PathBuf, String> {
    let cache_dir = ensure_generated_frame_preview_cache_dir(core)?;
    let output_path =
        persist_generated_frame_preview_in_dir(&cache_dir, frame_id, bytes, mime_type)?;
    allow_preview_file(core, &output_path);
    Ok(output_path)
}

pub(super) fn persist_generated_scrub_preview_in_dir(
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
    let created = !output_path.is_file();
    if created {
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
    if created {
        cleanup_generated_frame_preview_cache_dir(cache_dir)?;
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
    cleanup_generated_frame_preview_cache_dir(cache_dir)?;
    Ok(cached_path)
}
