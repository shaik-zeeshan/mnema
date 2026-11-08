//! Pulling still images out of a segment video with AVFoundation.

use super::*;

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
pub(super) fn run_test_video_preview_extractor(
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
pub(super) fn image_bytes_from_cg_image(
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
pub(super) fn preview_image_bytes_from_cg_image(
    image: &cidre::cg::Image,
) -> Result<(Vec<u8>, &'static str), String> {
    use cidre::ut;

    image_bytes_from_cg_image(image, ut::Type::webp(), "WebP", "image/webp").or_else(
        |_webp_error| image_bytes_from_cg_image(image, ut::Type::jpeg(), "JPEG", "image/jpeg"),
    )
}

#[cfg(target_os = "macos")]
pub(super) fn scrub_preview_image_bytes_from_cg_image(
    image: &cidre::cg::Image,
) -> Result<Vec<u8>, String> {
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
    if let Some(message) = video_preview_exact_miss_message(
        video_path,
        frame,
        used_indexed_offset,
        require_exact_time,
        offset_seconds,
        requested_time,
        actual_time,
    ) {
        crate::debug_log::log(message);
    }
}

/// The exact-miss debug line, or `None` when the miss is under the threshold.
#[cfg(target_os = "macos")]
pub(super) fn video_preview_exact_miss_message(
    video_path: &Path,
    frame: &::app_infra::Frame,
    used_indexed_offset: bool,
    require_exact_time: bool,
    offset_seconds: f64,
    requested_time: cidre::cm::Time,
    actual_time: cidre::cm::Time,
) -> Option<String> {
    let delta_ms = actual_time.sub(requested_time).abs().as_secs() * 1000.0;
    if delta_ms < FRAME_PREVIEW_EXACT_MISS_LOG_THRESHOLD_MS {
        return None;
    }

    let frame_identity = parse_frame_identity_from_path(Path::new(&frame.file_path))
        .map(|(captured_at_unix_ms, frame_index)| format!("{captured_at_unix_ms}:{frame_index}"))
        .unwrap_or_else(|| "unknown".to_string());

    Some(format!(
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
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn extract_preview_image_from_video_blocking(
    video_path: PathBuf,
    frame: &::app_infra::Frame,
    exact_offset_ms: Option<u64>,
    offset_seconds: f64,
    require_exact_time: bool,
    cancel_requested: Arc<AtomicBool>,
) -> Result<(Vec<u8>, &'static str), String> {
    #[cfg(test)]
    if let Some(result) = run_test_video_preview_extractor(&video_path, offset_seconds) {
        return result;
    }

    if cancel_requested.load(Ordering::SeqCst) {
        return Err(PREVIEW_GENERATION_CANCELLED.to_string());
    }

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let result = {
        use cidre::{av, blocks, cg, cm, ns};

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

        struct ExactPreviewState {
            result: Option<Result<(Vec<u8>, &'static str, cm::Time), String>>,
        }

        let state = Arc::new((
            Mutex::new(ExactPreviewState { result: None }),
            Condvar::new(),
        ));
        let callback_state = Arc::clone(&state);
        let callback_cancel_requested = Arc::clone(&cancel_requested);
        let mut block = blocks::EscBlock::new3(
            move |image: Option<&cg::Image>, actual_time: cm::Time, error: Option<&ns::Error>| {
                let image_result = if callback_cancel_requested.load(Ordering::SeqCst) {
                    Err(PREVIEW_GENERATION_CANCELLED.to_string())
                } else if let Some(error) = error {
                    Err(format!(
                        "failed to generate preview image from video: {error}"
                    ))
                } else {
                    image
                        .ok_or_else(|| "preview generation succeeded without image".to_string())
                        .and_then(preview_image_bytes_from_cg_image)
                        .map(|(bytes, mime_type)| (bytes, mime_type, actual_time))
                };

                let (lock, cvar) = &*callback_state;
                let mut state = lock.lock().expect("exact preview state poisoned");
                state.result = Some(image_result);
                cvar.notify_one();
            },
        );
        image_generator.cg_image_for_time_ch(requested_time, &mut block);

        let (lock, cvar) = &*state;
        let mut state = lock.lock().expect("exact preview state poisoned");
        let wait_started_at = Instant::now();
        let mut cancelled = false;
        while state.result.is_none() {
            if cancel_requested.load(Ordering::SeqCst) {
                cancelled = true;
                break;
            }

            let remaining_timeout = FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT
                .checked_sub(wait_started_at.elapsed())
                .unwrap_or_default();
            if remaining_timeout.is_zero() {
                break;
            }
            let wait_for = remaining_timeout.min(PREVIEW_GENERATION_CANCEL_POLL_INTERVAL);
            let (next_state, _) = cvar
                .wait_timeout(state, wait_for)
                .expect("exact preview state poisoned while waiting");
            state = next_state;
        }

        let result = state.result.take();
        drop(state);
        image_generator.cancel_all_cg_image_gen();

        if cancelled {
            return Err(PREVIEW_GENERATION_CANCELLED.to_string());
        }

        let (bytes, mime_type, actual_time) = match result {
            Some(Ok(result)) => result,
            Some(Err(error)) => return Err(error),
            None if cancel_requested.load(Ordering::SeqCst) => {
                return Err(PREVIEW_GENERATION_CANCELLED.to_string());
            }
            None => {
                return Err(format!(
                    "timed out generating exact frame preview after {}s",
                    FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT.as_secs()
                ));
            }
        };

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

        Ok((bytes, mime_type))
    };

    result
}

#[cfg(target_os = "macos")]
pub(super) async fn extract_preview_image_from_video(
    video_path: &Path,
    frame: &::app_infra::Frame,
    exact_offset_ms: Option<u64>,
    offset_seconds: f64,
    require_exact_time: bool,
    cancel_requested: Arc<AtomicBool>,
) -> Result<(Vec<u8>, &'static str), String> {
    let cancel_on_timeout = Arc::clone(&cancel_requested);
    tokio::time::timeout(
        FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT,
        tokio::task::spawn_blocking({
            let video_path = video_path.to_path_buf();
            let frame = frame.clone();
            let cancel_requested = Arc::clone(&cancel_requested);
            move || {
                extract_preview_image_from_video_blocking(
                    video_path,
                    &frame,
                    exact_offset_ms,
                    offset_seconds,
                    require_exact_time,
                    cancel_requested,
                )
            }
        }),
    )
    .await
    .map_err(|_| {
        cancel_on_timeout.store(true, Ordering::SeqCst);
        format!(
            "timed out generating exact frame preview after {}s",
            FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT.as_secs()
        )
    })?
    .map_err(|error| format!("failed to join video preview extraction task: {error}"))?
}

#[cfg(target_os = "macos")]
pub(super) fn extract_scrub_preview_images_from_video_batch_blocking(
    video_path: PathBuf,
    video_offset_ms: Vec<u64>,
    max_pixel_size: u32,
    cancel_requested: Arc<AtomicBool>,
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
    if cancel_requested.load(Ordering::SeqCst) {
        return Err(PREVIEW_GENERATION_CANCELLED.to_string());
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
    let mut cancelled = false;
    while state.remaining > 0 {
        if cancel_requested.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        let remaining_timeout = SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT
            .checked_sub(wait_started_at.elapsed())
            .unwrap_or_default();
        if remaining_timeout.is_zero() {
            break;
        }
        let wait_for = remaining_timeout.min(PREVIEW_GENERATION_CANCEL_POLL_INTERVAL);
        let waited_until_timeout = wait_for == remaining_timeout;
        let (next_state, wait_result) = cvar
            .wait_timeout(state, wait_for)
            .expect("scrub preview batch state poisoned while waiting");
        state = next_state;
        if wait_result.timed_out() && waited_until_timeout {
            break;
        }
    }
    let timed_out = state.remaining > 0;
    let results_by_time = std::mem::take(&mut state.results);
    drop(state);
    image_generator.cancel_all_cg_image_gen();

    if cancelled {
        return Err(PREVIEW_GENERATION_CANCELLED.to_string());
    }

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
pub(super) async fn extract_scrub_preview_images_from_video_batch(
    video_path: &Path,
    video_offset_ms: &[u64],
    max_pixel_size: u32,
    cancel_requested: Arc<AtomicBool>,
) -> Result<HashMap<u64, Result<Vec<u8>, String>>, String> {
    let cancel_on_timeout = Arc::clone(&cancel_requested);
    tokio::time::timeout(
        SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT,
        tokio::task::spawn_blocking({
            let video_path = video_path.to_path_buf();
            let video_offset_ms = video_offset_ms.to_vec();
            let cancel_requested = Arc::clone(&cancel_requested);
            move || {
                extract_scrub_preview_images_from_video_batch_blocking(
                    video_path,
                    video_offset_ms,
                    max_pixel_size,
                    cancel_requested,
                )
            }
        }),
    )
    .await
    .map_err(|_| {
        cancel_on_timeout.store(true, Ordering::SeqCst);
        format!(
            "timed out joining scrub preview extraction task after {}s",
            SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT.as_secs()
        )
    })?
    .map_err(|error| format!("failed to join scrub preview extraction task: {error}"))?
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn extract_scrub_preview_images_from_video_batch(
    _video_path: &Path,
    _video_offset_ms: &[u64],
    _max_pixel_size: u32,
    _cancel_requested: Arc<AtomicBool>,
) -> Result<HashMap<u64, Result<Vec<u8>, String>>, String> {
    Err("scrub preview video generation is only supported on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn extract_preview_image_from_video(
    _video_path: &Path,
    _frame: &::app_infra::Frame,
    _exact_offset_ms: Option<u64>,
    _offset_seconds: f64,
    _require_exact_time: bool,
    _cancel_requested: Arc<AtomicBool>,
) -> Result<(Vec<u8>, &'static str), String> {
    Err("video frame preview fallback is only supported on macOS".to_string())
}
