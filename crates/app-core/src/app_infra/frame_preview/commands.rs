//! The frame-preview commands: cache status/clear, cancel, and the frame and
//! scrub preview lookups.

use super::*;

pub fn get_scrub_preview_cache_status(
    core: &AppCore,
) -> Result<ScrubPreviewCacheStatusDto, String> {
    let cache_root = scrub_preview_cache_root(core)?;
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

pub fn clear_scrub_preview_cache(core: &AppCore) -> Result<ScrubPreviewCacheStatusDto, String> {
    let cache_root = scrub_preview_cache_root(core)?;
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
    get_scrub_preview_cache_status(core)
}

pub fn cancel_active_frame_preview_video_requests(core: &AppCore) -> Result<usize, String> {
    Ok(core
        .state::<FramePreviewCacheState>()
        .lock()
        .expect("frame preview cache poisoned")
        .cancel_active_video_requests())
}

pub fn clear_scrub_preview_cache_for_video_paths(
    core: &AppCore,
    video_paths: &[String],
) -> Result<ScrubPreviewCacheStatusDto, String> {
    let cache_root = scrub_preview_cache_root(core)?;
    if !cache_root.is_dir() || video_paths.is_empty() {
        return get_scrub_preview_cache_status(core);
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

    get_scrub_preview_cache_status(core)
}

pub async fn get_frame_preview(
    core: &AppCore,
    request: GetFramePreviewRequest,
) -> Result<Option<FramePreviewDto>, String> {
    let infra = core.state::<AppInfraState>().as_ref().clone();
    let cache = core.state::<FramePreviewCacheState>();
    let settings = core.state::<crate::recording_settings::RecordingSettingsState>();
    let ttl = preview_cache_ttl(&settings);
    let video_scope = VideoPreviewRequestScope::from(request.video_scope);

    if ttl.is_zero() {
        cache.lock().expect("frame preview cache poisoned").clear();
        return get_frame_preview_inner_with_logging(
            &infra,
            &cache,
            Some(core),
            request.frame_id,
            video_scope,
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
                Some(core),
                request.frame_id,
                video_scope,
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

pub async fn get_frame_scrub_previews(
    core: &AppCore,
    request: GetFrameScrubPreviewsRequest,
) -> Result<FrameScrubPreviewsDto, String> {
    let started_at = Instant::now();
    let infra = core.state::<AppInfraState>().as_ref().clone();
    let cache = core.state::<FramePreviewCacheState>();
    let settings = core.state::<crate::recording_settings::RecordingSettingsState>();
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
    let scrub_cache_dir_result = ensure_generated_frame_scrub_preview_cache_dir(core);
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

        match prepare_frame_scrub_preview(&infra, core, frame_id, max_pixel_size, scrub_cache_dir)
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
                match preview_state
                    .begin_video_request(&video_path, VideoPreviewRequestScope::Shared)
                {
                    Ok(token) => Ok(token),
                    Err(rx) => Err(rx),
                }
            };

            match video_request_guard {
                Ok(token) => {
                    let batch_started_at = Instant::now();
                    let mut offsets = Vec::new();
                    for candidate in &still_pending {
                        if !offsets.contains(&candidate.video_offset_ms) {
                            offsets.push(candidate.video_offset_ms);
                        }
                    }
                    let mut result = extract_scrub_preview_images_from_video_batch(
                        &video_path,
                        &offsets,
                        max_pixel_size,
                        Arc::clone(&token.cancel_requested),
                    )
                    .await;
                    if token.cancel_requested.load(Ordering::SeqCst) {
                        result = Err(PREVIEW_GENERATION_CANCELLED.to_string());
                    }
                    let notify_result = result.as_ref().map(|_| ()).map_err(Clone::clone);
                    cache
                        .lock()
                        .expect("frame preview cache poisoned")
                        .finish_video_request(&token, notify_result);

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
                                        0,
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
