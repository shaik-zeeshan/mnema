//! Resolving one frame's preview (original file, segment fallback, or a video
//! extract) and preparing a frame's scrub preview.

use super::*;

pub(super) async fn get_frame_preview_inner(
    infra: &::app_infra::AppInfra,
    cache: &FramePreviewCacheState,
    core: Option<&AppCore>,
    frame_id: i64,
    video_scope: VideoPreviewRequestScope,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let Some(frame) = infra.get_frame(frame_id).await? else {
        return Ok(None);
    };
    let secret_redaction_count = infra.frame_secret_redaction_count(frame_id).await?;

    let frame_file_path = PathBuf::from(&frame.file_path);
    if frame_file_path.is_file() {
        if let Some(core) = core {
            allow_preview_file(core, &frame_file_path);
        }
        return Ok(Some(frame_preview_payload(
            frame_file_path.to_string_lossy(),
            frame_image_mime_type(&frame_file_path),
            FramePreviewSourceKindDto::OriginalFrame,
            secret_redaction_count,
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
            core,
            secret_redaction_count,
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
            core,
            secret_redaction_count,
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
            core,
            secret_redaction_count,
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
            match preview_state.begin_video_request(&segment_paths.video_path, video_scope) {
                Ok(token) => Ok(token),
                Err(rx) => Err(rx),
            }
        };

        match video_request_guard {
            Ok(token) => {
                let mut result = extract_preview_image_from_video(
                    &segment_paths.video_path,
                    &frame,
                    exact_offset_ms,
                    offset_seconds,
                    require_exact_time,
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

                match result {
                    Ok(result) => break result,
                    Err(video_error) => {
                        if video_error == PREVIEW_GENERATION_CANCELLED {
                            return Err(::app_infra::AppInfraError::Io(std::io::Error::new(
                                std::io::ErrorKind::Interrupted,
                                video_error,
                            )));
                        }
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
                            core,
                            secret_redaction_count,
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
                    if video_error == PREVIEW_GENERATION_CANCELLED {
                        return Err(::app_infra::AppInfraError::Io(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            video_error,
                        )));
                    }
                    return read_segment_frame_preview_or_return_video_error(
                        &frame,
                        infra,
                        &related_frames,
                        &segment_paths.video_path,
                        core,
                        secret_redaction_count,
                        video_error,
                    );
                }
            }
        }
    };

    let persisted_path = if let Some(core) = core {
        persist_generated_frame_preview(core, frame.id, &bytes, mime_type)
    } else {
        let cache_dir = std::env::temp_dir().join("mnema-preview-test-cache");
        persist_generated_frame_preview_in_dir(&cache_dir, frame.id, &bytes, mime_type)
    }
    .map_err(::app_infra::AppInfraError::OcrEngine)?;

    Ok(Some(frame_preview_payload(
        persisted_path.to_string_lossy(),
        mime_type,
        FramePreviewSourceKindDto::VideoFallback,
        secret_redaction_count,
    )))
}

pub(super) async fn get_frame_preview_inner_with_logging(
    infra: &::app_infra::AppInfra,
    cache: &FramePreviewCacheState,
    core: Option<&AppCore>,
    frame_id: i64,
    video_scope: VideoPreviewRequestScope,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let started_at = Instant::now();
    let result = get_frame_preview_inner(infra, cache, core, frame_id, video_scope).await;
    let elapsed_ms = started_at.elapsed().as_millis();

    match &result {
        Ok(Some(_preview)) => {}
        Ok(None) => crate::debug_log::log(format!(
            "[DEBUG-frame-preview] frame_id={} missing elapsed_ms={}",
            frame_id, elapsed_ms,
        )),
        Err(error) => crate::debug_log::log_error(format!(
            "[DEBUG-frame-preview] frame_id={} failed elapsed_ms={} error={}",
            frame_id, elapsed_ms, error,
        )),
    }

    result
}

#[derive(Debug, Clone)]
pub(super) struct PreparedVideoScrubPreview {
    pub(super) frame_id: i64,
    pub(super) video_path: PathBuf,
    pub(super) video_offset_ms: u64,
    pub(super) source_hash: String,
}

#[derive(Debug, Clone)]
pub(super) enum PreparedFrameScrubPreview {
    Ready(FrameScrubPreviewResultDto),
    Video(PreparedVideoScrubPreview),
}

pub(super) async fn prepare_frame_scrub_preview(
    infra: &::app_infra::AppInfra,
    core: &AppCore,
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
            allow_preview_file(core, &cached_path);
            return Ok(PreparedFrameScrubPreview::Ready(
                FrameScrubPreviewResultDto {
                    frame_id,
                    preview: Some(frame_preview_payload(
                        cached_path.to_string_lossy(),
                        "image/jpeg",
                        FramePreviewSourceKindDto::ScrubPreview,
                        0,
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
        // Decoding a full PNG, resizing and JPEG-encoding is blocking work: keep it
        // off the runtime workers.
        let (cache_dir_owned, source_path) = (cache_dir.to_path_buf(), frame_file_path.clone());
        let derivative_path = core
            .run_blocking(move || {
                generate_scrub_preview_derivative_in_dir(
                    &cache_dir_owned,
                    frame_id,
                    max_pixel_size,
                    &source_path,
                )
            })
            .await?;
        allow_preview_file(core, &derivative_path);
        return Ok(PreparedFrameScrubPreview::Ready(
            FrameScrubPreviewResultDto {
                frame_id,
                preview: Some(frame_preview_payload(
                    derivative_path.to_string_lossy(),
                    "image/jpeg",
                    FramePreviewSourceKindDto::ScrubPreview,
                    0,
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

pub(super) fn preview_cache_ttl(
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Duration {
    let ttl_seconds = settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .preview_cache_ttl_seconds;

    Duration::from_secs(ttl_seconds)
}

pub(super) fn scrub_preview_perf_debug_enabled() -> bool {
    matches!(
        std::env::var("MNEMA_SCRUB_PERF_DEBUG").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

pub(super) fn log_scrub_preview_perf(event: &str, fields: impl AsRef<str>) {
    if !scrub_preview_perf_debug_enabled() {
        return;
    }
    crate::debug_log::log(format!(
        "[DEBUG-scrub-perf] event={event} {}",
        fields.as_ref()
    ));
}
