use super::{CapturedFrameEquivalenceResolver, CapturedFrameEquivalenceScope};
use crate::{AppInfra, Frame, FrameEquivalence, NewFrame};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "app-infra-captured-frame-equivalence-{label}-{timestamp}-{unique}"
        ));
        fs::create_dir_all(&path).expect("test dir should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_async_test<F>(future: F)
where
    F: std::future::Future<Output = ()>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(future);
}

fn write_test_png_rgba(
    dir: &TestDir,
    file_name: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> String {
    let path = dir.path().join(file_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("test image parent should exist");
    }
    image::save_buffer(&path, pixels, width, height, image::ColorType::Rgba8)
        .expect("test png should be written");
    path.to_string_lossy().into_owned()
}

fn solid_rgba(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..(width as usize * height as usize) {
        pixels.extend_from_slice(&rgba);
    }
    pixels
}

fn set_pixel_rgba(pixels: &mut [u8], width: u32, x: u32, y: u32, rgba: [u8; 4]) {
    let offset = ((y * width + x) * 4) as usize;
    pixels[offset..offset + 4].copy_from_slice(&rgba);
}

fn test_equivalence(file_path: &str) -> FrameEquivalence {
    match capture_screen::captured_frame_equivalence_from_image_path(Path::new(file_path)) {
        capture_screen::CapturedFrameEquivalenceOutcome::Ready(equivalence) => {
            FrameEquivalence::ready(equivalence.hint, equivalence.proof, equivalence.version)
        }
        capture_screen::CapturedFrameEquivalenceOutcome::Quarantined(error) => {
            panic!("test image equivalence should compute: {error}");
        }
    }
}

fn test_frame_with_equivalent_image(
    dir: &TestDir,
    session_id: &str,
    file_name: &str,
    captured_at: &str,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> NewFrame {
    let file_path = write_test_png_rgba(dir, file_name, width, height, pixels);
    let equivalence = test_equivalence(&file_path);

    NewFrame::new(session_id, file_path, captured_at)
        .with_dimensions(width as i64, height as i64)
        .with_equivalence(equivalence)
}

fn test_segment_frame_with_equivalent_image(
    dir: &TestDir,
    session_id: &str,
    segment_index: u64,
    file_name: &str,
    captured_at: &str,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> NewFrame {
    let frames_dir = dir.path().join(format!(
        "2026/04/12/.{session_id}-segment-{segment_index:04}/frames"
    ));
    fs::create_dir_all(&frames_dir).expect("segment frames dir should exist");
    let relative_name =
        format!("2026/04/12/.{session_id}-segment-{segment_index:04}/frames/{file_name}");
    test_frame_with_equivalent_image(
        dir,
        session_id,
        &relative_name,
        captured_at,
        pixels,
        width,
        height,
    )
}

async fn persist_frame(infra: &AppInfra, frame: &NewFrame) -> Frame {
    infra
        .processing()
        .insert_frame(frame)
        .await
        .expect("frame should persist")
}

#[test]
fn returns_none_when_candidate_has_no_ready_equivalence() {
    run_async_test(async {
        let dir = TestDir::new("no-equivalence");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let frame = persist_frame(
            &infra,
            &NewFrame::new(
                "session-no-equivalence",
                dir.path().join("frame-1.png").to_string_lossy(),
                "2026-04-12T10:00:00Z",
            ),
        )
        .await;

        let resolved = resolver
            .find_nearest_earlier_equivalent_frame(&frame, &CapturedFrameEquivalenceScope::Session)
            .await
            .expect("lookup should succeed");

        assert_eq!(resolved, None);
    });
}

#[test]
fn returns_nearest_earlier_match_in_session_scope() {
    run_async_test(async {
        let dir = TestDir::new("nearest-session");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let width = 32;
        let height = 32;
        let repeated_pixels = solid_rgba(width, height, [64, 64, 64, 255]);
        let mut changed_pixels = repeated_pixels.clone();
        for y in 8..20 {
            for x in 8..20 {
                set_pixel_rgba(&mut changed_pixels, width, x, y, [240, 240, 240, 255]);
            }
        }

        let first = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-nearest",
                "frame-1.png",
                "2026-04-12T10:00:00Z",
                &repeated_pixels,
                width,
                height,
            ),
        )
        .await;
        let _changed = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-nearest",
                "frame-2.png",
                "2026-04-12T10:00:01Z",
                &changed_pixels,
                width,
                height,
            ),
        )
        .await;
        let repeated = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-nearest",
                "frame-3.png",
                "2026-04-12T10:00:02Z",
                &repeated_pixels,
                width,
                height,
            ),
        )
        .await;

        let resolved = resolver
            .find_nearest_earlier_equivalent_frame(
                &repeated,
                &CapturedFrameEquivalenceScope::Session,
            )
            .await
            .expect("lookup should succeed")
            .expect("match should exist");

        assert_eq!(resolved, first);
    });
}

#[test]
fn returns_earliest_earlier_match_in_session_scope() {
    run_async_test(async {
        let dir = TestDir::new("earliest-session");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let width = 32;
        let height = 32;
        let repeated_pixels = solid_rgba(width, height, [72, 72, 72, 255]);
        let mut changed_pixels = repeated_pixels.clone();
        for y in 8..20 {
            for x in 8..20 {
                set_pixel_rgba(&mut changed_pixels, width, x, y, [220, 220, 220, 255]);
            }
        }

        let first = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-earliest",
                "frame-1.png",
                "2026-04-12T10:00:00Z",
                &repeated_pixels,
                width,
                height,
            ),
        )
        .await;
        let second = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-earliest",
                "frame-2.png",
                "2026-04-12T10:00:01Z",
                &repeated_pixels,
                width,
                height,
            ),
        )
        .await;
        let _changed = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-earliest",
                "frame-3.png",
                "2026-04-12T10:00:02Z",
                &changed_pixels,
                width,
                height,
            ),
        )
        .await;
        let repeated = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-earliest",
                "frame-4.png",
                "2026-04-12T10:00:03Z",
                &repeated_pixels,
                width,
                height,
            ),
        )
        .await;

        let nearest = resolver
            .find_nearest_earlier_equivalent_frame(
                &repeated,
                &CapturedFrameEquivalenceScope::Session,
            )
            .await
            .expect("nearest lookup should succeed")
            .expect("nearest match should exist");
        assert_eq!(nearest, second);

        let earliest = resolver
            .find_earliest_earlier_equivalent_frame(
                &repeated,
                &CapturedFrameEquivalenceScope::Session,
            )
            .await
            .expect("earliest lookup should succeed")
            .expect("earliest match should exist");

        assert_eq!(earliest, first);
    });
}

#[test]
fn ignores_quarantined_earlier_candidates() {
    run_async_test(async {
        let dir = TestDir::new("quarantine");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let width = 32;
        let height = 32;
        let pixels = solid_rgba(width, height, [90, 90, 90, 255]);

        let mut quarantined = test_frame_with_equivalent_image(
            &dir,
            "session-quarantine",
            "frame-1.png",
            "2026-04-12T10:00:00Z",
            &pixels,
            width,
            height,
        );
        quarantined.equivalence = FrameEquivalence::quarantined("decode failed");
        let _quarantined = persist_frame(&infra, &quarantined).await;

        let candidate = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-quarantine",
                "frame-2.png",
                "2026-04-12T10:00:01Z",
                &pixels,
                width,
                height,
            ),
        )
        .await;

        let resolved = resolver
            .find_nearest_earlier_equivalent_frame(
                &candidate,
                &CapturedFrameEquivalenceScope::Session,
            )
            .await
            .expect("lookup should succeed");

        assert_eq!(resolved, None);
    });
}

#[test]
fn ignores_version_mismatches() {
    run_async_test(async {
        let dir = TestDir::new("version-mismatch");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let width = 32;
        let height = 32;
        let pixels = solid_rgba(width, height, [100, 100, 100, 255]);

        let mut older = test_frame_with_equivalent_image(
            &dir,
            "session-version",
            "frame-1.png",
            "2026-04-12T10:00:00Z",
            &pixels,
            width,
            height,
        );
        older.equivalence.version = Some(older.equivalence.version.expect("version") + 1);
        let _older = persist_frame(&infra, &older).await;

        let candidate = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-version",
                "frame-2.png",
                "2026-04-12T10:00:01Z",
                &pixels,
                width,
                height,
            ),
        )
        .await;

        let resolved = resolver
            .find_nearest_earlier_equivalent_frame(
                &candidate,
                &CapturedFrameEquivalenceScope::Session,
            )
            .await
            .expect("lookup should succeed");

        assert_eq!(resolved, None);
    });
}

#[test]
fn hidden_segment_workspace_scope_does_not_cross_workspaces() {
    run_async_test(async {
        let dir = TestDir::new("hidden-segment-scope");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let width = 32;
        let height = 32;
        let pixels = solid_rgba(width, height, [104, 104, 104, 255]);

        let _first = persist_frame(
            &infra,
            &test_segment_frame_with_equivalent_image(
                &dir,
                "session-segment-ui-scope",
                1,
                "frame-1.png",
                "2026-04-12T10:00:00Z",
                &pixels,
                width,
                height,
            ),
        )
        .await;
        let second = persist_frame(
            &infra,
            &test_segment_frame_with_equivalent_image(
                &dir,
                "session-segment-ui-scope",
                2,
                "frame-2.png",
                "2026-04-12T10:00:01Z",
                &pixels,
                width,
                height,
            ),
        )
        .await;

        let scope = CapturedFrameEquivalenceScope::from_frame(&second);
        let resolved = resolver
            .find_nearest_earlier_equivalent_frame(&second, &scope)
            .await
            .expect("lookup should succeed");

        assert_eq!(resolved, None);
    });
}

#[test]
fn session_scope_can_match_outside_hidden_segment_workspace() {
    run_async_test(async {
        let dir = TestDir::new("session-wide-scope");
        let infra = AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let resolver = CapturedFrameEquivalenceResolver::new(infra.processing().clone());
        let width = 32;
        let height = 32;
        let pixels = solid_rgba(width, height, [110, 110, 110, 255]);

        let plain = persist_frame(
            &infra,
            &test_frame_with_equivalent_image(
                &dir,
                "session-wide-scope",
                "frame-1.png",
                "2026-04-12T10:00:00Z",
                &pixels,
                width,
                height,
            ),
        )
        .await;
        let segment = persist_frame(
            &infra,
            &test_segment_frame_with_equivalent_image(
                &dir,
                "session-wide-scope",
                1,
                "frame-2.png",
                "2026-04-12T10:00:01Z",
                &pixels,
                width,
                height,
            ),
        )
        .await;

        let resolved = resolver
            .find_nearest_earlier_equivalent_frame(
                &segment,
                &CapturedFrameEquivalenceScope::Session,
            )
            .await
            .expect("lookup should succeed")
            .expect("match should exist");

        assert_eq!(resolved, plain);
    });
}
