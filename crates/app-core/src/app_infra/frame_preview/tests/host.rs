//! The `Some(core)` paths, on a `FakeHost`: asset-scope grants go out as
//! `WindowSignal::AllowAssetPath`, cache changes as `ScrubPreviewCacheChanged`.

use super::*;
use crate::test_support::{core_in, FakeHost};

fn grant(path: impl Into<PathBuf>, recursive: bool) -> WindowSignal {
    WindowSignal::AllowAssetPath {
        path: path.into(),
        recursive,
    }
}

#[test]
fn original_frame_preview_grants_just_that_file() {
    let dir = TestDir::new("frame-preview-host-original");
    let host = Arc::new(FakeHost::default());
    let core = core_in(Arc::clone(&host), dir.path());
    core.block_on(async {
        let infra = ::app_infra::AppInfra::initialize(&dir.path().join("infra"))
            .await
            .expect("app infra should initialize");
        let frame_path = dir.path().join("frame.png");
        fs::write(&frame_path, b"png").expect("frame file should be written");
        let frame = infra
            .insert_frame(&::app_infra::NewFrame::new(
                "session-host",
                frame_path.to_string_lossy().to_string(),
                "2026-04-12T10:00:00Z",
            ))
            .await
            .expect("frame should be inserted");

        get_frame_preview_inner(
            &infra,
            &FramePreviewCacheState::default(),
            Some(&core),
            frame.id,
            VideoPreviewRequestScope::Shared,
        )
        .await
        .expect("preview should load")
        .expect("preview should exist");

        assert_eq!(
            *host.window_signals.lock().unwrap(),
            vec![grant(&frame_path, false)]
        );
    });
}

#[test]
fn persisted_frame_preview_grants_its_cache_dir_recursively_then_the_file() {
    let dir = TestDir::new("frame-preview-host-persist");
    let host = Arc::new(FakeHost::default());
    let core = core_in(Arc::clone(&host), dir.path());

    let path = persist_generated_frame_preview(&core, 42, b"jpeg", "image/jpeg")
        .expect("preview should persist");

    let cache_dir = core.paths().cache.join("frame-previews");
    assert_eq!(path.parent(), Some(cache_dir.as_path()));
    assert_eq!(
        *host.window_signals.lock().unwrap(),
        vec![grant(&cache_dir, true), grant(&path, false)]
    );
}

#[test]
fn scrub_cache_root_keeps_its_layout_and_grants_it_recursively() {
    let dir = TestDir::new("frame-preview-host-scrub-root");
    let host = Arc::new(FakeHost::default());
    let core = core_in(Arc::clone(&host), dir.path());

    let status = get_scrub_preview_cache_status(&core).expect("status should load");

    let root = core
        .paths()
        .cache
        .join("scrub-previews")
        .join("v1-jpeg-q72-max360-1fps");
    assert_eq!(status.cache_directory, root.to_string_lossy());
    assert_eq!(
        *host.window_signals.lock().unwrap(),
        vec![grant(&root, true)]
    );
}

#[test]
fn scrub_preview_generation_emits_cache_changed_and_grants_each_interval() {
    let dir = TestDir::new("frame-preview-host-scrub-job");
    let host = Arc::new(FakeHost::default());
    let core = Arc::new(core_in(Arc::clone(&host), dir.path()));
    core.manage(FramePreviewCacheState::default());
    let _extractor = TestVideoPreviewExtractorGuard::install(Arc::new(|_path, _offset| {
        Ok((b"jpeg".to_vec(), "image/jpeg"))
    }));
    let segment_dir = dir.path().join("segment");
    let job = ScrubPreviewGenerationJob {
        segment_cache_key: "segment-key".to_string(),
        segment_dir: segment_dir.clone(),
        video_path: dir.path().join("segment.mov"),
        frame_index_path: None,
        started_unix_ms: 1_000_000,
        intervals: vec![(0, 120), (1000, 1200)],
    };

    core.block_on(generate_scrub_preview_job(job, Arc::clone(&core)));

    assert_eq!(
        *host.events.lock().unwrap(),
        vec![CoreEvent::ScrubPreviewCacheChanged(
            ScrubPreviewCacheChangedPayload {
                rendition: "v1-jpeg-q72-max360-1fps".to_string(),
                start_unix_ms: 1_000_000,
                end_unix_ms: 1_002_000,
            }
        )]
    );
    assert_eq!(
        *host.window_signals.lock().unwrap(),
        vec![
            grant(scrub_preview_interval_path(&segment_dir, 0), false),
            grant(scrub_preview_interval_path(&segment_dir, 1000), false),
        ]
    );
}
