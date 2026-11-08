use super::*;

fn nearest_preview_test_frame(id: i64, file_path: &str, captured_at: &str) -> ::app_infra::Frame {
    ::app_infra::Frame {
        id,
        session_id: "session".to_string(),
        file_path: file_path.to_string(),
        captured_at: captured_at.to_string(),
        width: None,
        height: None,
        equivalence: ::app_infra::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        metadata_snapshot: None,
        created_at: captured_at.to_string(),
        updated_at: captured_at.to_string(),
    }
}

#[test]
fn nearest_segment_frame_preview_rejects_siblings_beyond_distance_cap() {
    let unique = format!(
        "mnema-nearest-preview-cap-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos()
    );
    let base_dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&base_dir).expect("base dir should be created");

    // Filenames carry the capture timestamp (frame-{unix_ms}-{index}).
    let near_path = base_dir.join("frame-1000004000-000001.jpg");
    let far_path = base_dir.join("frame-1000060000-000002.jpg");
    fs::write(&near_path, b"near").expect("near sibling should be written");
    fs::write(&far_path, b"far").expect("far sibling should be written");

    let target = nearest_preview_test_frame(
        1,
        &base_dir
            .join("frame-1000000000-000000.jpg")
            .to_string_lossy(),
        "",
    );
    let near = nearest_preview_test_frame(2, &near_path.to_string_lossy(), "");
    let far = nearest_preview_test_frame(3, &far_path.to_string_lossy(), "");

    // 4s away: within the cap, chosen.
    let bytes = read_nearest_segment_frame_preview(&target, &[near, far.clone()])
        .expect("read should succeed");
    assert_eq!(bytes.as_deref(), Some(b"near".as_slice()));

    // Only sibling is 60s away: beyond the cap, no fallback.
    let bytes = read_nearest_segment_frame_preview(&target, &[far]).expect("read should succeed");
    assert!(bytes.is_none());

    fs::remove_dir_all(&base_dir).expect("base dir should be removed");
}

#[test]
fn persist_generated_frame_preview_overwrites_existing_file() {
    let unique = format!(
        "mnema-preview-overwrite-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos()
    );
    let cache_dir = std::env::temp_dir().join(unique);

    let first = persist_generated_frame_preview_in_dir(&cache_dir, 7, b"stale", "image/jpeg")
        .expect("first persist should succeed");
    let second = persist_generated_frame_preview_in_dir(&cache_dir, 7, b"fresh", "image/jpeg")
        .expect("second persist should succeed");

    assert_eq!(first, second);
    assert_eq!(
        fs::read(&second).expect("preview should be readable"),
        b"fresh"
    );

    fs::remove_dir_all(&cache_dir).expect("cache dir should be removed");
}

#[test]
fn scrub_preview_last_bucket_emits_one_interval_for_indexed_zero_duration_segment() {
    assert_eq!(
        scrub_preview_last_bucket(0, true),
        SCRUB_PREVIEW_INTERVAL_MS
    );
}

#[test]
fn scrub_preview_last_bucket_keeps_empty_zero_duration_segment_empty() {
    assert_eq!(scrub_preview_last_bucket(0, false), 0);
}

#[test]
fn scrub_preview_interval_end_includes_segment_end_frame_without_adjacent_segment() {
    assert_eq!(
        scrub_preview_interval_end_unix_ms(1_000, 1_000, None),
        1_001
    );
    assert_eq!(
        scrub_preview_interval_end_unix_ms(1_000, 1_500, None),
        1_501
    );
    assert_eq!(
        scrub_preview_interval_end_unix_ms(1_000, 3_000, None),
        2_000
    );
}

#[test]
fn scrub_preview_interval_end_stays_exclusive_at_adjacent_segment_boundary() {
    assert_eq!(
        scrub_preview_interval_end_unix_ms(1_000, 2_000, Some(2_000)),
        2_000
    );
    assert_eq!(
        scrub_preview_interval_end_unix_ms(1_000, 3_000, Some(2_000)),
        2_000
    );
    assert_eq!(
        scrub_preview_interval_end_unix_ms(1_000, 2_000, Some(1_000)),
        1_000
    );
}

#[test]
fn scrub_preview_next_segment_clamp_uses_expanded_index_bounds() {
    let index = capture_screen::ScreenSegmentFrameIndex {
        version: capture_screen::SCREEN_SEGMENT_FRAME_INDEX_VERSION,
        entries: vec![capture_screen::ScreenSegmentFrameIndexEntry {
            captured_at_unix_ms: 3_000,
            frame_index: 0,
            video_offset_ms: 0,
        }],
    };
    let (_, expanded_end_unix_ms) = scrub_preview_segment_bounds_unix_ms(1_000, 2_000, &index);

    assert_eq!(expanded_end_unix_ms, 3_000);
    assert_eq!(
        scrub_preview_adjacent_next_segment_started_unix_ms(Some(2_500), expanded_end_unix_ms),
        Some(2_500)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn finalized_screen_segment_scrub_preview_job_plans_indexed_buckets() {
    let temp_dir = tempfile::tempdir().unwrap();
    let video_path = temp_dir.path().join("session-segment-0001.mov");
    fs::write(
        &video_path,
        b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak",
    )
    .unwrap();
    let index = capture_screen::ScreenSegmentFrameIndex {
        version: capture_screen::SCREEN_SEGMENT_FRAME_INDEX_VERSION,
        entries: vec![
            capture_screen::ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 10_000,
                frame_index: 0,
                video_offset_ms: 0,
            },
            capture_screen::ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 10_400,
                frame_index: 1,
                video_offset_ms: 400,
            },
            capture_screen::ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 11_000,
                frame_index: 2,
                video_offset_ms: 1_000,
            },
        ],
    };
    fs::write(
        capture_screen::screen_segment_frame_index_path(&video_path),
        capture_screen::encode_screen_segment_frame_index(&index),
    )
    .unwrap();

    let job = scrub_preview_generation_job_for_video_path(temp_dir.path(), video_path)
        .unwrap()
        .expect("finalized segment should queue scrub previews");

    assert_eq!(job.started_unix_ms, 10_000);
    assert_eq!(job.intervals, vec![(0, 0), (1_000, 1_000)]);
}

#[cfg(target_os = "macos")]
#[test]
fn finalized_screen_segment_scrub_preview_job_skips_cached_intervals() {
    let temp_dir = tempfile::tempdir().unwrap();
    let video_path = temp_dir.path().join("session-segment-0002.mov");
    fs::write(
        &video_path,
        b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak",
    )
    .unwrap();
    let index = capture_screen::ScreenSegmentFrameIndex {
        version: capture_screen::SCREEN_SEGMENT_FRAME_INDEX_VERSION,
        entries: vec![
            capture_screen::ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 20_000,
                frame_index: 0,
                video_offset_ms: 0,
            },
            capture_screen::ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 21_000,
                frame_index: 1,
                video_offset_ms: 1_000,
            },
        ],
    };
    let frame_index_path = capture_screen::screen_segment_frame_index_path(&video_path);
    fs::write(
        &frame_index_path,
        capture_screen::encode_screen_segment_frame_index(&index),
    )
    .unwrap();
    let segment_cache_key = segment_cache_key(&video_path, Some(&frame_index_path));
    let segment_dir = scrub_preview_segment_dir(temp_dir.path(), &segment_cache_key);
    write_scrub_preview_metadata(
        &segment_dir,
        &segment_cache_key,
        &video_path,
        Some(&frame_index_path),
    )
    .unwrap();
    fs::write(scrub_preview_interval_path(&segment_dir, 0), b"cached").unwrap();

    let job = scrub_preview_generation_job_for_video_path(temp_dir.path(), video_path)
        .unwrap()
        .expect("uncached interval should still queue");

    assert_eq!(job.intervals, vec![(1_000, 1_000)]);
}

fn count_scrub_preview_jpegs(cache_root: &Path) -> usize {
    fs::read_dir(cache_root)
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .flat_map(|entry| fs::read_dir(entry.path()).unwrap().flatten())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("jpg"))
        .count()
}

#[test]
fn scrub_preview_cache_cleanup_evicts_by_file_count_and_removes_empty_segment_dirs() {
    let temp_dir = tempfile::tempdir().unwrap();
    let segment_a = temp_dir.path().join("segment-a");
    let segment_b = temp_dir.path().join("segment-b");
    fs::create_dir_all(&segment_a).unwrap();
    fs::create_dir_all(&segment_b).unwrap();
    fs::write(scrub_preview_metadata_path(&segment_a), b"{}").unwrap();
    fs::write(scrub_preview_metadata_path(&segment_b), b"{}").unwrap();
    fs::write(segment_a.join("000000.jpg"), b"a").unwrap();
    std::thread::sleep(Duration::from_millis(5));
    fs::write(segment_b.join("000000.jpg"), b"b").unwrap();

    cleanup_generated_scrub_preview_cache_dir_with_limits(
        temp_dir.path(),
        1,
        u64::MAX,
        Duration::from_secs(60 * 60),
    )
    .unwrap();

    assert_eq!(count_scrub_preview_jpegs(temp_dir.path()), 1);
    let segment_dirs = fs::read_dir(temp_dir.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(segment_dirs, 1);
}

#[test]
fn scrub_preview_cache_cleanup_evicts_by_total_bytes() {
    let temp_dir = tempfile::tempdir().unwrap();
    let segment = temp_dir.path().join("segment");
    fs::create_dir_all(&segment).unwrap();
    fs::write(scrub_preview_metadata_path(&segment), b"{}").unwrap();
    fs::write(segment.join("000000.jpg"), b"1234").unwrap();
    std::thread::sleep(Duration::from_millis(5));
    fs::write(segment.join("001000.jpg"), b"5678").unwrap();

    cleanup_generated_scrub_preview_cache_dir_with_limits(
        temp_dir.path(),
        usize::MAX,
        4,
        Duration::from_secs(60 * 60),
    )
    .unwrap();

    assert_eq!(count_scrub_preview_jpegs(temp_dir.path()), 1);
}
