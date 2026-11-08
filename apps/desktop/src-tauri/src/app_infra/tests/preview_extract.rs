use super::*;

#[test]
fn resolve_segment_preview_paths_maps_hidden_workspace_to_visible_video() {
    let frame_path =
        Path::new("/tmp/2026/04/12/.session-abc-segment-0004/frames/frame-1744459200123-7.png");

    let resolved = resolve_segment_preview_paths(frame_path).expect("segment paths should resolve");

    assert_eq!(
        resolved.workspace_dir,
        PathBuf::from("/tmp/2026/04/12/.session-abc-segment-0004")
    );
    assert_eq!(
        resolved.video_path,
        PathBuf::from("/tmp/2026/04/12/session-abc-segment-0004.mov")
    );
}

#[test]
fn estimate_frame_preview_offset_seconds_uses_segment_frame_times() {
    let frame = ::app_infra::Frame {
        id: 2,
        session_id: "session-estimate".to_string(),
        file_path: "/tmp/.session-estimate-segment-0004/frames/frame-1744459201500-1.png"
            .to_string(),
        captured_at: "2025-04-12T10:00:01.500Z".to_string(),
        width: None,
        height: None,
        equivalence: ::app_infra::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        created_at: String::new(),
        updated_at: String::new(),
        metadata_snapshot: None,
    };
    let related_frames = vec![::app_infra::Frame {
        id: 1,
        session_id: "session-estimate".to_string(),
        file_path: "/tmp/.session-estimate-segment-0004/frames/frame-1744459200000-0.png"
            .to_string(),
        captured_at: "2025-04-12T10:00:00Z".to_string(),
        width: None,
        height: None,
        equivalence: ::app_infra::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        created_at: String::new(),
        updated_at: String::new(),
        metadata_snapshot: None,
    }];

    let offset_seconds = estimate_frame_preview_offset_seconds(&frame, &related_frames);

    assert!((offset_seconds - 1.5).abs() < f64::EPSILON);
}

#[test]
fn indexed_frame_preview_offset_prefers_exact_frame_identity_match() {
    let dir = TestDir::new("frame-preview-indexed-exact");
    let video_path = dir.path().join("session-preview-segment-0001.mov");
    fs::write(&video_path, b"fake mov").expect("video fixture should exist");
    let index_path = capture_screen::screen_segment_frame_index_path(&video_path);
    let index = capture_screen::ScreenSegmentFrameIndex {
        version: capture_screen::SCREEN_SEGMENT_FRAME_INDEX_VERSION,
        entries: vec![capture_screen::ScreenSegmentFrameIndexEntry {
            captured_at_unix_ms: 1_744_459_201_500,
            frame_index: 42,
            video_offset_ms: 875,
        }],
    };
    fs::write(
        &index_path,
        capture_screen::encode_screen_segment_frame_index(&index),
    )
    .expect("index file should be written");

    let frame = ::app_infra::Frame {
        id: 2,
        session_id: "session-preview".to_string(),
        file_path: dir
            .path()
            .join(".session-preview-segment-0001/frames/frame-1744459201500-000042.jpg")
            .to_string_lossy()
            .to_string(),
        captured_at: "2025-04-12T10:00:01.500Z".to_string(),
        width: None,
        height: None,
        equivalence: ::app_infra::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        created_at: String::new(),
        updated_at: String::new(),
        metadata_snapshot: None,
    };

    let offset = indexed_frame_preview_offset(&frame, &video_path)
        .expect("index lookup should succeed")
        .expect("index entry should exist");

    assert_eq!(offset.video_offset_ms, 875);
    assert!(offset.exact_match);
}

#[cfg(target_os = "macos")]
#[test]
fn exact_preview_requested_time_rounds_up_to_video_tick() {
    let requested = exact_preview_requested_time(56_133);

    assert_eq!(requested.as_secs(), 56.13333333333333);
}

#[cfg(target_os = "macos")]
#[test]
fn video_preview_exact_miss_log_includes_requested_actual_and_delta() {
    let dir = TestDir::new("frame-preview-exact-miss-log");
    let requested = cidre::cm::Time::with_secs(1.5, 600);
    // Six video ticks past the requested time (timescale 600 => 10ms) so the
    // delta clears FRAME_PREVIEW_EXACT_MISS_LOG_THRESHOLD_MS and is logged.
    let actual = cidre::cm::Time::with_secs(6.0 / 600.0 + 1.5, 600);
    let frame = ::app_infra::Frame {
        id: 2,
        session_id: "session-preview".to_string(),
        file_path: dir
            .path()
            .join(".session-preview-segment-0001/frames/frame-1744459201500-000042.jpg")
            .to_string_lossy()
            .to_string(),
        captured_at: "2025-04-12T10:00:01.500Z".to_string(),
        width: None,
        height: None,
        equivalence: ::app_infra::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        created_at: String::new(),
        updated_at: String::new(),
        metadata_snapshot: None,
    };

    let contents = video_preview_exact_miss_message(
        Path::new("/tmp/session-preview-segment-0001.mov"),
        &frame,
        true,
        true,
        1.5,
        requested,
        actual,
    )
    .expect("a 10ms miss should be logged");
    assert!(contents.contains("[DEBUG-frame-preview] event=video_exact_miss"));
    assert!(contents.contains("path=/tmp/session-preview-segment-0001.mov"));
    assert!(contents.contains("frame_id=2"));
    assert!(contents.contains("frame_identity=1744459201500:42"));
    assert!(contents.contains("used_indexed_offset=true"));
    assert!(contents.contains("require_exact_time=true"));
    assert!(contents.contains("offset_seconds=1.5"));
    assert!(contents.contains("requested_time=1.5"));
    assert!(contents.contains("actual_time=1.51"));
    assert!(contents.contains("delta_ms=10.000"));
}

#[test]
fn get_frame_preview_inner_returns_original_frame_bytes_when_png_exists() {
    run_async_test(async {
        let dir = TestDir::new("frame-preview-original");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let cache = FramePreviewCacheState::default();
        let frame_path = dir.path().join("frame-preview.png");
        fs::write(&frame_path, b"not-a-real-png-but-preview-bytes")
            .expect("frame preview file should be written");

        let stored_frame = infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-preview",
                frame_path.to_string_lossy().to_string(),
                "2026-04-12T10:00:00Z",
            ))
            .await
            .expect("frame should be inserted");

        let preview = get_frame_preview_inner(
            &infra,
            &cache,
            None,
            stored_frame.id,
            VideoPreviewRequestScope::Shared,
        )
        .await
        .expect("preview should load")
        .expect("preview should exist");

        assert_eq!(preview.mime_type, "image/png");
        assert_eq!(
            preview.source_kind,
            FramePreviewSourceKindDto::OriginalFrame
        );
        assert_eq!(preview.file_path, frame_path.to_string_lossy());
    });
}

#[test]
fn get_frame_preview_inner_returns_segment_frame_bytes_when_exact_png_and_video_are_missing() {
    run_async_test(async {
        let dir = TestDir::new("frame-preview-segment-fallback");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let cache = FramePreviewCacheState::default();
        let workspace_dir = dir.path().join("2026/04/12/.session-preview-segment-0001");
        let frames_dir = workspace_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames directory should be created");

        let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
        let sibling_frame_path = frames_dir.join("frame-1744459201000-0.png");
        let sibling_bytes = b"segment-frame-preview-bytes";
        fs::write(&sibling_frame_path, sibling_bytes)
            .expect("sibling frame preview file should be written");

        let target_frame = infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-preview",
                target_frame_path.to_string_lossy().to_string(),
                "2025-04-12T10:00:01.500Z",
            ))
            .await
            .expect("target frame should be inserted");

        infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-preview",
                sibling_frame_path.to_string_lossy().to_string(),
                "2025-04-12T10:00:01.000Z",
            ))
            .await
            .expect("sibling frame should be inserted");

        let preview = get_frame_preview_inner(
            &infra,
            &cache,
            None,
            target_frame.id,
            VideoPreviewRequestScope::Shared,
        )
        .await
        .expect("preview should load")
        .expect("preview should exist");

        assert_eq!(preview.mime_type, "image/png");
        assert_eq!(
            preview.source_kind,
            FramePreviewSourceKindDto::SegmentFrameFallback
        );
        assert_ne!(preview.file_path, sibling_frame_path.to_string_lossy());
        assert!(Path::new(&preview.file_path).is_file());
    });
}

#[test]
fn get_frame_preview_inner_falls_back_to_segment_frame_when_visible_video_cannot_be_opened() {
    run_async_test(async {
        let dir = TestDir::new("frame-preview-video-preferred");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let cache = FramePreviewCacheState::default();
        let segment_dir = dir.path().join("2026/04/12");
        let workspace_dir = segment_dir.join(".session-preview-segment-0001");
        let frames_dir = workspace_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames directory should be created");

        let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
        let sibling_frame_path = frames_dir.join("frame-1744459201000-0.png");
        let video_path = segment_dir.join("session-preview-segment-0001.mov");
        let sibling_bytes = b"segment-frame-preview-bytes";
        fs::write(&sibling_frame_path, sibling_bytes)
            .expect("sibling frame preview file should be written");
        fs::write(&video_path, b"not-a-real-video")
            .expect("visible segment video should be written");

        let target_frame = infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-preview",
                target_frame_path.to_string_lossy().to_string(),
                "2025-04-12T10:00:01.500Z",
            ))
            .await
            .expect("target frame should be inserted");

        infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-preview",
                sibling_frame_path.to_string_lossy().to_string(),
                "2025-04-12T10:00:01.000Z",
            ))
            .await
            .expect("sibling frame should be inserted");

        let preview = get_frame_preview_inner(
            &infra,
            &cache,
            None,
            target_frame.id,
            VideoPreviewRequestScope::Shared,
        )
        .await
        .expect("preview should load")
        .expect("preview should exist");

        assert_eq!(preview.mime_type, "image/png");
        assert_eq!(
            preview.source_kind,
            FramePreviewSourceKindDto::SegmentFrameFallback
        );
        assert_ne!(preview.file_path, sibling_frame_path.to_string_lossy());
        assert!(Path::new(&preview.file_path).is_file());
    });
}

#[test]
fn get_frame_preview_inner_returns_error_immediately_when_visible_video_is_empty_and_no_segment_frame_exists(
) {
    run_async_test(async {
        let dir = TestDir::new("frame-preview-empty-video");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let cache = FramePreviewCacheState::default();
        let segment_dir = dir.path().join("2026/04/12");
        let workspace_dir = segment_dir.join(".session-preview-segment-0001");
        let frames_dir = workspace_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames directory should be created");

        let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
        let video_path = segment_dir.join("session-preview-segment-0001.mov");
        fs::write(&video_path, b"").expect("visible segment video should be written");

        let target_frame = infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-preview",
                target_frame_path.to_string_lossy().to_string(),
                "2025-04-12T10:00:01.500Z",
            ))
            .await
            .expect("target frame should be inserted");

        let error = get_frame_preview_inner(
            &infra,
            &cache,
            None,
            target_frame.id,
            VideoPreviewRequestScope::Shared,
        )
        .await
        .expect_err("empty visible video without persisted fallback should error");

        let error_message = error.to_string();
        assert!(error_message.contains("segment video is empty"));
        assert!(error_message.contains(&video_path.display().to_string()));
    });
}

#[test]
fn get_frame_preview_inner_serializes_video_extraction_per_segment() {
    run_multithread_async_test(async {
        let dir = TestDir::new("frame-preview-video-serialization");
        let infra = Arc::new(
            ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize"),
        );
        let cache = Arc::new(FramePreviewCacheState::default());
        let segment_dir = dir.path().join("2026/04/12");
        let workspace_dir = segment_dir.join(".session-preview-segment-0001");
        let frames_dir = workspace_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames directory should be created");

        let video_path = segment_dir.join("session-preview-segment-0001.mov");
        fs::write(&video_path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  moov mdat")
            .expect("visible segment video should be written");

        let mut frame_ids = Vec::new();
        for index in 0..4 {
            let frame_path = frames_dir.join(format!("frame-1744459201{index}00-{index}.png"));
            let captured_at = format!("2025-04-12T10:00:01.{index}00Z");
            let frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    captured_at,
                ))
                .await
                .expect("frame should be inserted");
            frame_ids.push(frame.id);
        }

        let concurrent_calls = Arc::new(AtomicUsize::new(0));
        let max_concurrent_calls = Arc::new(AtomicUsize::new(0));
        let _extractor_guard = TestVideoPreviewExtractorGuard::install(Arc::new({
            let concurrent_calls = Arc::clone(&concurrent_calls);
            let max_concurrent_calls = Arc::clone(&max_concurrent_calls);
            move |path, _offset_seconds| {
                struct ActiveCallGuard {
                    concurrent_calls: Arc<AtomicUsize>,
                }

                impl Drop for ActiveCallGuard {
                    fn drop(&mut self) {
                        self.concurrent_calls.fetch_sub(1, Ordering::SeqCst);
                    }
                }

                let active = concurrent_calls.fetch_add(1, Ordering::SeqCst) + 1;
                max_concurrent_calls.fetch_max(active, Ordering::SeqCst);
                let _active_call_guard = ActiveCallGuard {
                    concurrent_calls: Arc::clone(&concurrent_calls),
                };

                thread::sleep(Duration::from_millis(40));
                if active > 1 {
                    return Err(format!(
                        "test extractor saw concurrent access for {}",
                        path.display()
                    ));
                }

                Ok((b"preview-bytes".to_vec(), "image/png"))
            }
        }));

        let mut tasks = Vec::new();
        for frame_id in frame_ids {
            let infra = Arc::clone(&infra);
            let cache = Arc::clone(&cache);
            tasks.push(tokio::spawn(async move {
                get_frame_preview_inner(
                    &infra,
                    &cache,
                    None,
                    frame_id,
                    VideoPreviewRequestScope::Shared,
                )
                .await
            }));
        }

        for task in tasks {
            let preview = task
                .await
                .expect("preview task should complete")
                .expect("preview should load")
                .expect("preview should exist");
            assert_eq!(
                preview.source_kind,
                FramePreviewSourceKindDto::VideoFallback
            );
            assert!(Path::new(&preview.file_path).is_file());
        }

        assert_eq!(max_concurrent_calls.load(Ordering::SeqCst), 1);
    });
}
