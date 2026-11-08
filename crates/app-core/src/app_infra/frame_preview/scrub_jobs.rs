//! Background scrub-preview generation and the availability query that
//! schedules it.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct ScrubPreviewGenerationJob {
    pub(super) segment_cache_key: String,
    pub(super) segment_dir: PathBuf,
    pub(super) video_path: PathBuf,
    pub(super) frame_index_path: Option<PathBuf>,
    pub(super) started_unix_ms: i64,
    pub(super) intervals: Vec<(u64, u64)>,
}

pub(super) async fn generate_scrub_preview_job(job: ScrubPreviewGenerationJob, core: Arc<AppCore>) {
    let keys = job
        .intervals
        .iter()
        .map(|(interval_start, _)| ScrubIntervalWorkKey {
            segment_cache_key: job.segment_cache_key.clone(),
            interval_start_video_offset_ms: *interval_start,
        })
        .collect::<Vec<_>>();

    let accepted = {
        core.state::<FramePreviewCacheState>()
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
            Arc::new(AtomicBool::new(false)),
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
                        allow_preview_file(&core, &path);
                    }
                }
            }
            let start_unix_ms = job.started_unix_ms + chunk[0].0 as i64;
            let end_unix_ms = job.started_unix_ms
                + chunk
                    .last()
                    .map(|(interval_start, _)| *interval_start + SCRUB_PREVIEW_INTERVAL_MS)
                    .unwrap_or(0) as i64;
            core.host().emit(CoreEvent::ScrubPreviewCacheChanged(
                ScrubPreviewCacheChangedPayload {
                    rendition: SCRUB_PREVIEW_RENDITION.to_string(),
                    start_unix_ms,
                    end_unix_ms,
                },
            ));
        }
    }

    core.state::<FramePreviewCacheState>()
        .lock()
        .expect("frame preview cache poisoned")
        .finish_scrub_interval_work(&accepted);
}

pub(super) fn scrub_preview_generation_job_for_video_path(
    cache_root: &Path,
    video_path: PathBuf,
) -> Result<Option<ScrubPreviewGenerationJob>, String> {
    if !cfg!(target_os = "macos")
        || !video_path.is_file()
        || fs::metadata(&video_path)
            .map(|metadata| metadata.len() == 0)
            .unwrap_or(true)
        || !mov_file_appears_openable_for_preview(&video_path).unwrap_or(false)
    {
        return Ok(None);
    }

    let index = match load_screen_segment_frame_index(&video_path) {
        Ok(Some(index)) if !index.entries.is_empty() => index,
        Ok(_) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to load screen segment frame index for {}: {error}",
                video_path.display()
            ));
        }
    };
    let indexed_start_unix_ms = index
        .entries
        .iter()
        .map(|entry| entry.captured_at_unix_ms as i64)
        .min()
        .unwrap_or(0);
    let indexed_end_unix_ms = index
        .entries
        .iter()
        .map(|entry| entry.captured_at_unix_ms as i64)
        .max()
        .unwrap_or(indexed_start_unix_ms);
    let (started_unix_ms, _) =
        scrub_preview_segment_bounds_unix_ms(indexed_start_unix_ms, indexed_end_unix_ms, &index);
    let frame_index_path = screen_frame_index_existing_path(&video_path);
    let segment_cache_key = segment_cache_key(&video_path, frame_index_path.as_deref());
    let segment_dir = scrub_preview_segment_dir(cache_root, &segment_cache_key);
    let metadata_valid = scrub_preview_metadata_valid(
        &segment_dir,
        &segment_cache_key,
        &video_path,
        frame_index_path.as_deref(),
    );
    let mut intervals = indexed_scrub_preview_offsets(&index)
        .into_iter()
        .filter(|(bucket, _)| {
            !(metadata_valid && scrub_preview_interval_path(&segment_dir, *bucket).is_file())
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|(bucket, _)| *bucket);

    if intervals.is_empty() {
        return Ok(None);
    }

    Ok(Some(ScrubPreviewGenerationJob {
        segment_cache_key,
        segment_dir,
        video_path,
        frame_index_path,
        started_unix_ms,
        intervals,
    }))
}

pub fn enqueue_scrub_preview_generation_for_screen_files(
    core: &Arc<AppCore>,
    screen_files: &[String],
) -> Result<usize, String> {
    if !cfg!(target_os = "macos") || screen_files.is_empty() {
        return Ok(0);
    }

    let cache_root = scrub_preview_cache_root(core)?;
    let mut seen = HashSet::new();
    let mut scheduled_intervals = 0usize;

    for screen_file in screen_files {
        if !seen.insert(screen_file.clone()) {
            continue;
        }
        match scrub_preview_generation_job_for_video_path(&cache_root, PathBuf::from(screen_file)) {
            Ok(Some(job)) => {
                scheduled_intervals += job.intervals.len();
                let task_core = Arc::clone(core);
                core.spawn(async move {
                    generate_scrub_preview_job(job, task_core).await;
                });
            }
            Ok(None) => {}
            Err(error) => {
                crate::debug_log::log_warn(format!(
                    "failed to enqueue finalized screen segment scrub previews for {screen_file}: {error}"
                ));
            }
        }
    }

    Ok(scheduled_intervals)
}

pub async fn get_scrub_preview_availability(
    core: &Arc<AppCore>,
    request: GetScrubPreviewAvailabilityRequest,
) -> Result<ScrubPreviewAvailabilityDto, String> {
    let start_unix_ms = request.start_unix_ms.min(request.end_unix_ms);
    let end_unix_ms = request.start_unix_ms.max(request.end_unix_ms);
    let enqueue_missing = request.enqueue_missing.unwrap_or(true);
    let infra = core.state::<AppInfraState>().as_ref().clone();
    let cache_root = scrub_preview_cache_root(core)?;
    let segments = infra
        .list_finalized_screen_segments_overlapping_window(
            &unix_ms_to_rfc3339(start_unix_ms),
            &unix_ms_to_rfc3339(end_unix_ms),
        )
        .await
        .map_err(|error| format!("failed to list screen segments for scrub previews: {error}"))?;

    let mut intervals = Vec::new();
    let mut jobs = Vec::new();
    for (segment_index, segment) in segments.iter().enumerate() {
        let segment_started_unix_ms =
            rfc3339_to_unix_ms(&segment.started_at).unwrap_or(start_unix_ms);
        let segment_ended_unix_ms =
            rfc3339_to_unix_ms(&segment.ended_at).unwrap_or(segment_started_unix_ms);
        let next_segment_started_unix_ms = segments
            .get(segment_index + 1)
            .and_then(|next_segment| rfc3339_to_unix_ms(&next_segment.started_at));
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
        let next_segment_started_unix_ms = scrub_preview_adjacent_next_segment_started_unix_ms(
            next_segment_started_unix_ms,
            segment_ended_unix_ms,
        );
        let duration_ms = (segment_ended_unix_ms - segment_started_unix_ms).max(0) as u64;
        let last_bucket = scrub_preview_last_bucket(duration_ms, !indexed_offsets.is_empty());
        let mut missing_for_generation = Vec::new();
        let mut bucket = 0;
        while bucket < last_bucket {
            let interval_start_unix_ms = segment_started_unix_ms + bucket as i64;
            let interval_end_unix_ms = scrub_preview_interval_end_unix_ms(
                interval_start_unix_ms,
                segment_ended_unix_ms,
                next_segment_started_unix_ms,
            );
            if interval_end_unix_ms <= interval_start_unix_ms {
                bucket += SCRUB_PREVIEW_INTERVAL_MS;
                continue;
            }
            if interval_end_unix_ms < start_unix_ms || interval_start_unix_ms > end_unix_ms {
                bucket += SCRUB_PREVIEW_INTERVAL_MS;
                continue;
            }
            let interval_path = scrub_preview_interval_path(&segment_dir, bucket);
            if metadata_valid && interval_path.is_file() {
                allow_preview_file(core, &interval_path);
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
        let task_core = Arc::clone(core);
        core.spawn(async move {
            generate_scrub_preview_job(job, task_core).await;
        });
    }

    Ok(ScrubPreviewAvailabilityDto {
        rendition: SCRUB_PREVIEW_RENDITION.to_string(),
        intervals,
    })
}
