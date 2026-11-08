use super::*;

#[test]
fn frame_preview_cache_returns_entries_within_ttl() {
    let dir = TestDir::new("frame-preview-cache-hit");
    let preview_path = dir.path().join("frame-preview.png");
    fs::write(&preview_path, b"preview").expect("preview fixture should exist");
    let mut cache = FramePreviewState::default();
    let now = Instant::now();
    let preview = FramePreviewDto {
        mime_type: "image/png".to_string(),
        file_path: preview_path.to_string_lossy().to_string(),
        source_kind: FramePreviewSourceKindDto::OriginalFrame,
        has_secret_redactions: false,
        secret_redaction_count: 0,
    };

    cache.insert(42, preview.clone(), Duration::from_secs(60), now);

    assert_eq!(cache.get(42, Duration::from_secs(60), now), Some(preview));
}

#[test]
fn frame_preview_cache_evicts_expired_entries() {
    let mut cache = FramePreviewState::default();
    let now = Instant::now();

    cache.insert(
        42,
        FramePreviewDto {
            mime_type: "image/png".to_string(),
            file_path: "/tmp/frame-preview.png".to_string(),
            source_kind: FramePreviewSourceKindDto::OriginalFrame,
            has_secret_redactions: false,
            secret_redaction_count: 0,
        },
        Duration::from_secs(1),
        now,
    );

    assert_eq!(
        cache.get(42, Duration::from_secs(1), now + Duration::from_secs(1)),
        None
    );
    assert_eq!(cache.len(), 0);
}

#[test]
fn frame_preview_cache_clear_removes_existing_entries() {
    let mut cache = FramePreviewState::default();
    cache.insert(
        42,
        FramePreviewDto {
            mime_type: "image/png".to_string(),
            file_path: "/tmp/frame-preview.png".to_string(),
            source_kind: FramePreviewSourceKindDto::OriginalFrame,
            has_secret_redactions: false,
            secret_redaction_count: 0,
        },
        Duration::from_secs(60),
        Instant::now(),
    );

    cache.clear();

    assert_eq!(cache.len(), 0);
}

#[test]
fn frame_preview_state_collapses_duplicate_in_flight_video_requests() {
    run_async_test(async {
        let mut state = FramePreviewState::default();
        let video_path = Path::new("/tmp/segment-0001.mov");

        let token = state
            .begin_video_request(video_path, VideoPreviewRequestScope::Shared)
            .expect("first request should become the in-flight video leader");
        let waiter = state
            .begin_video_request(video_path, VideoPreviewRequestScope::Shared)
            .expect_err("second request should subscribe to the in-flight video leader");
        assert_eq!(state.video_in_flight_len(), 1);

        state.finish_video_request(&token, Ok(()));

        assert_eq!(state.video_in_flight_len(), 0);
        assert_eq!(waiter.await.expect("waiter should receive result"), Ok(()));
    });
}

#[test]
fn frame_preview_state_cancels_in_flight_video_requests() {
    run_async_test(async {
        let mut state = FramePreviewState::default();
        let video_path = Path::new("/tmp/segment-0001.mov");

        let token = state
            .begin_video_request(video_path, VideoPreviewRequestScope::ActiveFrame)
            .expect("first request should become the in-flight video leader");
        let waiter = state
            .begin_video_request(video_path, VideoPreviewRequestScope::ActiveFrame)
            .expect_err("second request should subscribe to the in-flight video leader");

        assert_eq!(state.cancel_active_video_requests(), 1);
        assert_eq!(state.video_in_flight_len(), 0);
        assert!(waiter
            .await
            .expect("waiter should receive cancellation")
            .is_err());

        state.finish_video_request(&token, Ok(()));
        assert_eq!(state.video_in_flight_len(), 0);
    });
}

#[test]
fn frame_preview_state_cancel_active_video_requests_preserves_shared_work() {
    run_async_test(async {
        let mut state = FramePreviewState::default();
        let video_path = Path::new("/tmp/segment-0001.mov");

        let shared_token = state
            .begin_video_request(video_path, VideoPreviewRequestScope::Shared)
            .expect("shared request should become the shared in-flight leader");
        let mut shared_waiter = state
            .begin_video_request(video_path, VideoPreviewRequestScope::Shared)
            .expect_err("second shared request should subscribe to shared work");
        let active_token = state
            .begin_video_request(video_path, VideoPreviewRequestScope::ActiveFrame)
            .expect("active request should use a separate in-flight lane");
        let active_waiter = state
            .begin_video_request(video_path, VideoPreviewRequestScope::ActiveFrame)
            .expect_err("second active request should subscribe to active work");

        assert_eq!(state.video_in_flight_len(), 2);
        assert_eq!(state.cancel_active_video_requests(), 1);
        assert_eq!(state.video_in_flight_len(), 1);
        assert!(active_waiter
            .await
            .expect("active waiter should receive cancellation")
            .is_err());
        assert!(shared_waiter.try_recv().is_err());

        state.finish_video_request(&active_token, Ok(()));
        assert_eq!(state.video_in_flight_len(), 1);
        state.finish_video_request(&shared_token, Ok(()));
        assert_eq!(state.video_in_flight_len(), 0);
        assert_eq!(
            shared_waiter
                .await
                .expect("shared waiter should receive result"),
            Ok(())
        );
    });
}

#[test]
fn frame_preview_cache_returns_video_failure_within_ttl() {
    let mut cache = FramePreviewCache::default();
    let now = Instant::now();
    let video_path = Path::new("/tmp/segment-0001.mov");

    cache.insert_video_failure(video_path, "cannot open".to_string(), now);

    assert_eq!(
        cache.get_video_failure(video_path, now + Duration::from_secs(1)),
        Some("cannot open".to_string())
    );
}

#[test]
fn mov_file_appears_openable_for_preview_requires_moov_atom() {
    let dir = TestDir::new("frame-preview-moov-check");
    let missing_moov_path = dir.path().join("missing-moov.mov");
    let with_moov_path = dir.path().join("with-moov.mov");

    fs::write(&missing_moov_path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  ")
        .expect("mov fixture without moov should be written");
    fs::write(&with_moov_path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  moov")
        .expect("mov fixture with moov should be written");

    assert!(!mov_file_appears_openable_for_preview(&missing_moov_path)
        .expect("missing-moov fixture should read"));
    assert!(mov_file_appears_openable_for_preview(&with_moov_path)
        .expect("with-moov fixture should read"));
}

#[test]
fn generated_scrub_preview_derivative_is_low_res_jpeg() {
    let dir = TestDir::new("scrub-preview-derivative");
    let source_path = dir.path().join("source.png");
    let cache_dir = dir.path().join("cache");
    let image = image::RgbImage::from_fn(640, 360, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, 96])
    });
    image
        .save(&source_path)
        .expect("source image fixture should be written");

    let derivative_path =
        generate_scrub_preview_derivative_in_dir(&cache_dir, 42, 200, &source_path)
            .expect("scrub derivative should generate");

    let derivative_file_name = derivative_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("derivative file name should be valid UTF-8");
    assert!(derivative_file_name.starts_with("scrub-v3-frame-42-200-"));
    assert!(derivative_file_name.ends_with(".jpg"));
    let derivative = image::open(&derivative_path).expect("derivative should decode");
    assert!(derivative.width() <= 200);
    assert!(derivative.height() <= 200);
    assert_eq!(frame_image_mime_type(&derivative_path), "image/jpeg");
}

#[test]
fn generated_scrub_preview_derivative_reuses_cached_file() {
    let dir = TestDir::new("scrub-preview-derivative-cache");
    let source_path = dir.path().join("source.png");
    let cache_dir = dir.path().join("cache");
    image::RgbImage::from_pixel(320, 240, image::Rgb([32, 64, 128]))
        .save(&source_path)
        .expect("source image fixture should be written");

    let first = generate_scrub_preview_derivative_in_dir(&cache_dir, 7, 200, &source_path)
        .expect("first scrub derivative should generate");
    let first_modified = fs::metadata(&first)
        .and_then(|metadata| metadata.modified())
        .expect("first derivative modified time should read");
    let second = generate_scrub_preview_derivative_in_dir(&cache_dir, 7, 200, &source_path)
        .expect("second scrub derivative should reuse cache");
    let second_modified = fs::metadata(&second)
        .and_then(|metadata| metadata.modified())
        .expect("second derivative modified time should read");

    assert_eq!(first, second);
    assert_eq!(first_modified, second_modified);
}

#[test]
fn generated_scrub_preview_derivative_prunes_cache_after_write() {
    let dir = TestDir::new("scrub-preview-derivative-prune");
    let source_path = dir.path().join("source.png");
    let cache_dir = dir.path().join("cache");
    fs::create_dir_all(&cache_dir).expect("cache dir should exist");
    for index in 0..GENERATED_FRAME_PREVIEW_CACHE_MAX_FILES {
        fs::write(cache_dir.join(format!("old-{index}.jpg")), b"old")
            .expect("old preview fixture should be written");
    }
    std::thread::sleep(Duration::from_millis(5));
    image::RgbImage::from_pixel(320, 240, image::Rgb([32, 64, 128]))
        .save(&source_path)
        .expect("source image fixture should be written");

    let derivative_path =
        generate_scrub_preview_derivative_in_dir(&cache_dir, 99, 200, &source_path)
            .expect("scrub derivative should generate");

    let files = fs::read_dir(&cache_dir)
        .expect("cache dir should list")
        .flatten()
        .filter(|entry| entry.path().is_file())
        .count();
    assert_eq!(files, GENERATED_FRAME_PREVIEW_CACHE_MAX_FILES);
    assert!(
        derivative_path.is_file(),
        "newly generated derivative should be retained"
    );
}

#[test]
fn scrub_preview_cache_keys_include_max_pixel_size() {
    let dir = TestDir::new("scrub-preview-cache-key");
    let preview_200_path = dir.path().join("scrub-v2-frame-42-200.jpg");
    let preview_400_path = dir.path().join("scrub-v2-frame-42-400.jpg");
    fs::write(&preview_200_path, b"200").expect("200px preview should exist");
    fs::write(&preview_400_path, b"400").expect("400px preview should exist");
    let mut cache = FramePreviewState::default();
    let now = Instant::now();
    let ttl = Duration::from_secs(60);

    cache.insert_scrub(
        42,
        200,
        FramePreviewDto {
            mime_type: "image/jpeg".to_string(),
            file_path: preview_200_path.to_string_lossy().to_string(),
            source_kind: FramePreviewSourceKindDto::ScrubPreview,
            has_secret_redactions: false,
            secret_redaction_count: 0,
        },
        ttl,
        now,
    );
    cache.insert_scrub(
        42,
        400,
        FramePreviewDto {
            mime_type: "image/jpeg".to_string(),
            file_path: preview_400_path.to_string_lossy().to_string(),
            source_kind: FramePreviewSourceKindDto::ScrubPreview,
            has_secret_redactions: false,
            secret_redaction_count: 0,
        },
        ttl,
        now,
    );

    assert_eq!(
        cache
            .get_scrub(42, 200, ttl, now)
            .expect("200px scrub preview should be cached")
            .file_path,
        preview_200_path.to_string_lossy()
    );
    assert_eq!(
        cache
            .get_scrub(42, 400, ttl, now)
            .expect("400px scrub preview should be cached")
            .file_path,
        preview_400_path.to_string_lossy()
    );
}

#[test]
fn scrub_preview_cache_invalidates_deleted_derivative() {
    let dir = TestDir::new("scrub-preview-cache-invalidates");
    let preview_path = dir.path().join("scrub-v2-frame-42-200.jpg");
    fs::write(&preview_path, b"preview").expect("preview should exist");
    let mut cache = FramePreviewState::default();
    let now = Instant::now();
    let ttl = Duration::from_secs(60);

    cache.insert_scrub(
        42,
        200,
        FramePreviewDto {
            mime_type: "image/jpeg".to_string(),
            file_path: preview_path.to_string_lossy().to_string(),
            source_kind: FramePreviewSourceKindDto::ScrubPreview,
            has_secret_redactions: false,
            secret_redaction_count: 0,
        },
        ttl,
        now,
    );
    fs::remove_file(&preview_path).expect("preview should be deleted");

    assert_eq!(cache.get_scrub(42, 200, ttl, now), None);
}

#[test]
fn frame_preview_cache_evicts_expired_video_failures() {
    let mut cache = FramePreviewCache::default();
    let now = Instant::now();
    let video_path = Path::new("/tmp/segment-0001.mov");

    cache.insert_video_failure(video_path, "cannot open".to_string(), now);

    assert_eq!(
        cache.get_video_failure(
            video_path,
            now + FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL + Duration::from_secs(1)
        ),
        None
    );
}

#[test]
fn frame_preview_cache_evicts_oldest_entries_when_max_size_is_reached() {
    let dir = TestDir::new("frame-preview-cache-max-size");
    let mut cache = FramePreviewState::default();
    let now = Instant::now();
    let ttl = Duration::from_secs(60);

    for frame_id in 0..=FRAME_PREVIEW_CACHE_MAX_ENTRIES as i64 {
        let preview_path = dir.path().join(format!("frame-preview-{frame_id}.png"));
        fs::write(&preview_path, frame_id.to_string()).expect("preview fixture should exist");
        cache.insert(
            frame_id,
            FramePreviewDto {
                mime_type: "image/png".to_string(),
                file_path: preview_path.to_string_lossy().to_string(),
                source_kind: FramePreviewSourceKindDto::OriginalFrame,
                has_secret_redactions: false,
                secret_redaction_count: 0,
            },
            ttl,
            now + Duration::from_millis(frame_id as u64),
        );
    }

    assert_eq!(cache.len(), FRAME_PREVIEW_CACHE_MAX_ENTRIES);
    assert_eq!(cache.get(0, ttl, now + Duration::from_secs(1)), None);
    assert!(cache
        .get(
            FRAME_PREVIEW_CACHE_MAX_ENTRIES as i64,
            ttl,
            now + Duration::from_secs(1)
        )
        .is_some());
}

#[test]
fn frame_preview_state_collapses_duplicate_in_flight_requests() {
    run_async_test(async {
        let mut state = FramePreviewState::default();

        assert!(state.begin_request(42).is_ok());
        let waiter = state
            .begin_request(42)
            .expect_err("second request should subscribe to the in-flight leader");
        assert_eq!(state.in_flight_len(), 1);

        let preview = Some(FramePreviewDto {
            mime_type: "image/png".to_string(),
            file_path: "/tmp/frame-preview.png".to_string(),
            source_kind: FramePreviewSourceKindDto::OriginalFrame,
            has_secret_redactions: false,
            secret_redaction_count: 0,
        });
        state.finish_request(42, Ok(preview.clone()));

        assert_eq!(state.in_flight_len(), 0);
        assert_eq!(
            waiter.await.expect("waiter should receive result"),
            Ok(preview)
        );
    });
}

#[test]
fn frame_preview_state_clear_removes_in_flight_requests() {
    let mut state = FramePreviewState::default();

    assert!(state.begin_request(7).is_ok());
    assert_eq!(state.in_flight_len(), 1);

    state.clear();

    assert_eq!(state.in_flight_len(), 0);
    assert_eq!(state.video_in_flight_len(), 0);
    assert_eq!(state.len(), 0);
}
