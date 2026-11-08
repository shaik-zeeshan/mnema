//! Seeded data dirs for tests, goldens and the parity dumper, so the GPUI app, the
//! Svelte stub and the tests all see the same data. Everything is written through
//! app-infra at a fixed "now".
//!
//! A seed dir holds `data/` (the capture root: DB + frame JPEGs), `config/`
//! (incl. the file-backed DB key), `cache/` and `log/`, i.e. a [`Paths`].

use std::{io, path::Path};

use app_infra::{AppInfra, NewFrame, ProcessingResultDraft};
use capture_metadata::FrameMetadataSnapshot;
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

use crate::paths::Paths;

/// The fixture clock. Every seeded timestamp is relative to it; the parity
/// harness pins the page clock to the same instant (`apps/desktop/parity/shoot.ts`).
pub const FIXTURE_NOW: &str = "2026-11-04T14:32:00+05:30";

/// Frames in the `populated` seed, newest first, `(seconds before now, app)`.
const POPULATED_FRAMES: &[(i64, App)] = &[
    (4, App::SLACK),
    (64, App::SLACK),
    (300, App::CODE),
    (360, App::CODE),
    (420, App::CODE),
    (900, App::SAFARI),
    (960, App::SAFARI),
    (1800, App::CODE),
    (1860, App::CODE),
    (3000, App::SLACK),
    (3300, App::SAFARI),
    (3540, App::CODE),
];
const FRAME_WIDTH: u32 = 640;
const FRAME_HEIGHT: u32 = 400;
const SESSION_ID: &str = "fixture-session";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureId {
    /// A fresh install: migrated DB, no captures.
    Empty,
    /// An hour of OCR'd screen frames across three apps.
    Populated,
}

impl FixtureId {
    pub const ALL: [FixtureId; 2] = [FixtureId::Empty, FixtureId::Populated];

    pub fn name(self) -> &'static str {
        match self {
            FixtureId::Empty => "empty",
            FixtureId::Populated => "populated",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct App {
    bundle_id: &'static str,
    name: &'static str,
    window_title: &'static str,
    ocr_text: &'static str,
    rgb: [u8; 3],
}

impl App {
    const SLACK: App = App {
        bundle_id: "com.tinyspeck.slackmacgap",
        name: "Slack",
        window_title: "mnema-dev",
        ocr_text: "standup: ship the fixture seed today",
        rgb: [74, 21, 75],
    };
    const CODE: App = App {
        bundle_id: "com.microsoft.VSCode",
        name: "Code",
        window_title: "lib.rs — mnema",
        ocr_text: "pub fn seed(dir: &Path, id: FixtureId)",
        rgb: [30, 30, 30],
    };
    const SAFARI: App = App {
        bundle_id: "com.apple.Safari",
        name: "Safari",
        window_title: "Release notes",
        ocr_text: "Mnema 0.1.27 release notes",
        rgb: [235, 238, 242],
    };
}

pub fn fixture_now() -> OffsetDateTime {
    OffsetDateTime::parse(FIXTURE_NOW, &Rfc3339).expect("FIXTURE_NOW is RFC 3339")
}

/// The layout a seed in `dir` uses. Also points `MNEMA_CAPTURE_INDEX_KEY_DIR` at
/// its file key store, so app-infra opens the DB without the keychain: call it
/// (or [`seed`]) before opening a seed in this process.
pub fn seed_paths(dir: &Path) -> Paths {
    let paths = Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        cache: dir.join("cache"),
        log: dir.join("log"),
    };
    std::env::set_var(
        "MNEMA_CAPTURE_INDEX_KEY_DIR",
        paths.config.join("capture-index-keys"),
    );
    paths
}

/// Write fixture `id` into `dir` (must not already hold a seed) and return its paths.
pub fn seed(dir: &Path, id: FixtureId) -> io::Result<Paths> {
    let paths = seed_paths(dir);
    for path in [&paths.data, &paths.config, &paths.cache, &paths.log] {
        std::fs::create_dir_all(path)?;
    }
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let infra = AppInfra::initialize(&paths.data)
            .await
            .map_err(io::Error::other)?;
        if id == FixtureId::Populated {
            write_populated(&infra, &paths.data).await?;
        }
        Ok::<_, io::Error>(())
    })?;
    Ok(paths)
}

async fn write_populated(infra: &AppInfra, data: &Path) -> io::Result<()> {
    let frames_dir = data.join("fixture-frames");
    std::fs::create_dir_all(&frames_dir)?;
    let now = fixture_now();
    // Oldest first, so frame ids rise with time like a real capture.
    for (index, (seconds_ago, app)) in POPULATED_FRAMES.iter().rev().enumerate() {
        let file_path = frames_dir.join(format!("frame-{index:02}.jpg"));
        frame_image(app.rgb, index)
            .save(&file_path)
            .map_err(io::Error::other)?;
        let captured_at = (now - Duration::seconds(*seconds_ago))
            .to_offset(time::UtcOffset::UTC)
            .format(&Rfc3339)
            .map_err(io::Error::other)?;
        let frame = NewFrame::new(SESSION_ID, file_path.to_string_lossy(), captured_at)
            .with_dimensions(FRAME_WIDTH.into(), FRAME_HEIGHT.into())
            .with_metadata_snapshot(FrameMetadataSnapshot {
                app_bundle_id: Some(app.bundle_id.to_string()),
                app_name: Some(app.name.to_string()),
                window_title: Some(app.window_title.to_string()),
                display_id: Some(1),
                ..Default::default()
            });
        // Through the live capture path, then OCR "finishes" with the fixture text,
        // which also writes the frame's search documents.
        let captured = infra
            .capture_frame(&frame, None)
            .await
            .map_err(io::Error::other)?;
        let job = captured
            .job
            .ok_or_else(|| io::Error::other("fixture frame was not admitted to OCR"))?;
        infra
            .claim_queued_processing_job(job.id)
            .await
            .map_err(io::Error::other)?;
        infra
            .complete_processing_job(
                job.id,
                &ProcessingResultDraft::new().with_result_text(app.ocr_text),
            )
            .await
            .map_err(io::Error::other)?;
    }
    Ok(())
}

/// The app's colour with a lighter band whose height encodes the frame index, so
/// neighbouring frames are visibly different in a timeline or golden.
fn frame_image(rgb: [u8; 3], index: usize) -> image::RgbImage {
    let band_top = FRAME_HEIGHT - (index as u32 + 1) * (FRAME_HEIGHT / 16);
    image::RgbImage::from_fn(FRAME_WIDTH, FRAME_HEIGHT, |_, y| {
        if y >= band_top {
            image::Rgb(rgb.map(|c| c / 2 + 100))
        } else {
            image::Rgb(rgb)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(paths: &Paths) -> (tokio::runtime::Runtime, AppInfra) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let infra = runtime
            .block_on(AppInfra::initialize(&paths.data))
            .expect("seed opens with app-infra");
        (runtime, infra)
    }

    #[test]
    fn fixture_now_is_the_parity_clock() {
        assert_eq!(fixture_now().unix_timestamp(), 1_793_782_920);
    }

    #[test]
    fn names_round_trip() {
        for id in FixtureId::ALL {
            assert_eq!(FixtureId::from_name(id.name()), Some(id));
        }
        assert_eq!(FixtureId::from_name("nope"), None);
    }

    // One test so the process-wide key-dir env var is never set concurrently.
    #[test]
    fn seeds_open_with_app_infra() {
        let empty_dir = tempfile::tempdir().unwrap();
        let paths = seed(empty_dir.path(), FixtureId::Empty).unwrap();
        let (runtime, infra) = open(&paths);
        let frames = runtime
            .block_on(infra.list_frames(None, None, None, None))
            .unwrap();
        assert!(frames.is_empty());
        drop((infra, runtime));

        let dir = tempfile::tempdir().unwrap();
        let paths = seed(dir.path(), FixtureId::Populated).unwrap();
        let (runtime, infra) = open(&paths);
        let frames = runtime
            .block_on(infra.list_frames(None, None, Some(100), None))
            .unwrap();
        assert_eq!(frames.len(), POPULATED_FRAMES.len());
        let newest = frames
            .iter()
            .max_by(|a, b| a.captured_at.cmp(&b.captured_at))
            .unwrap();
        assert_eq!(newest.captured_at, "2026-11-04T09:01:56Z");
        for frame in &frames {
            let image = image::open(&frame.file_path).expect("frame JPEG decodes");
            assert_eq!((image.width(), image.height()), (FRAME_WIDTH, FRAME_HEIGHT));
        }
        let apps = runtime.block_on(infra.list_searchable_apps()).unwrap();
        assert_eq!(apps.len(), 3, "Slack, Code and Safari");
    }
}
