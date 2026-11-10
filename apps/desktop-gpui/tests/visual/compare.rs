//! Golden comparator, ported from zed `crates/zed/src/visual_test_runner.rs` @ f816e1f9:
//! a pixel matches when every channel is within 2; a golden passes at a 0.99 match.
//! Baselines live in `tests/visual/baselines/<name>.png`; `UPDATE_BASELINE=1` rewrites
//! them (founder sign-off only). Actual and diff PNGs go to `$VISUAL_TEST_OUTPUT_DIR`
//! (default `<target>/visual_tests`).

use std::path::PathBuf;

use image::RgbaImage;

pub const TOLERANCE: i16 = 2;
pub const MATCH_THRESHOLD: f64 = 0.99;

pub struct ImageComparison {
    pub match_percentage: f64,
    pub diff_image: RgbaImage,
    pub diff_pixel_count: u32,
}

pub fn baseline_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/visual/baselines")
        .join(format!("{name}.png"))
}

fn output_dir() -> PathBuf {
    std::env::var_os("VISUAL_TEST_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
            target.join("visual_tests")
        })
}

/// `VISUAL_VIEWPORT=<w>x<h>` renders every state at that window size instead of
/// its golden size (a resize check: shots are saved, never compared).
pub fn viewport_override() -> Option<(f32, f32)> {
    let v = std::env::var("VISUAL_VIEWPORT").ok()?;
    let (w, h) = v.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// The window size a golden renders at: its own, unless `VISUAL_VIEWPORT` is set.
pub fn viewport(golden: (f32, f32)) -> gpui_kit::Size<gpui_kit::Pixels> {
    let (w, h) = viewport_override().unwrap_or(golden);
    gpui_kit::size(gpui_kit::px(w), gpui_kit::px(h))
}

/// Saves `actual`, then compares it with (or, under `UPDATE_BASELINE`, writes) the baseline.
pub fn check_golden(name: &str, actual: &RgbaImage) -> anyhow::Result<()> {
    let out = output_dir();
    std::fs::create_dir_all(&out)?;
    let actual_path = out.join(format!("{name}.png"));
    actual.save(&actual_path)?;
    println!("  {name}: actual saved to {}", actual_path.display());
    if viewport_override().is_some() {
        return Ok(());
    }

    let baseline = baseline_path(name);
    if std::env::var_os("UPDATE_BASELINE").is_some() {
        std::fs::create_dir_all(baseline.parent().expect("baseline dir"))?;
        actual.save(&baseline)?;
        println!("  {name}: baseline updated at {}", baseline.display());
        return Ok(());
    }
    if !baseline.exists() {
        anyhow::bail!(
            "baseline missing: {} (UPDATE_BASELINE=1 creates it)",
            baseline.display()
        );
    }
    let expected = image::open(&baseline)?.to_rgba8();
    let cmp = compare_images(actual, &expected);
    println!(
        "  {name}: match {:.3}% ({} differing pixels)",
        cmp.match_percentage * 100.0,
        cmp.diff_pixel_count
    );
    if cmp.match_percentage >= MATCH_THRESHOLD {
        return Ok(());
    }
    let diff_path = out.join(format!("{name}_diff.png"));
    cmp.diff_image.save(&diff_path)?;
    anyhow::bail!(
        "{name}: {:.3}% match < {:.0}% (diff: {})",
        cmp.match_percentage * 100.0,
        MATCH_THRESHOLD * 100.0,
        diff_path.display()
    )
}

pub fn compare_images(actual: &RgbaImage, expected: &RgbaImage) -> ImageComparison {
    let width = actual.width().max(expected.width());
    let height = actual.height().max(expected.height());
    let total_pixels = width * height;
    let mut diff_image = RgbaImage::new(width, height);
    let mut matching_pixels = 0u32;
    let blank = image::Rgba([0, 0, 0, 0]);
    for y in 0..height {
        for x in 0..width {
            let a = if x < actual.width() && y < actual.height() {
                *actual.get_pixel(x, y)
            } else {
                blank
            };
            let e = if x < expected.width() && y < expected.height() {
                *expected.get_pixel(x, y)
            } else {
                blank
            };
            if pixels_are_similar(&a, &e) {
                matching_pixels += 1;
                diff_image.put_pixel(x, y, image::Rgba([0, 255, 0, 64]));
            } else {
                diff_image.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
    }
    ImageComparison {
        match_percentage: matching_pixels as f64 / total_pixels as f64,
        diff_image,
        diff_pixel_count: total_pixels - matching_pixels,
    }
}

fn pixels_are_similar(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> bool {
    (0..4).all(|i| (a.0[i] as i16 - b.0[i] as i16).abs() <= TOLERANCE)
}

/// Self-check of the comparator's boundaries (run from each golden binary's main).
pub fn self_check() {
    let base = RgbaImage::from_pixel(10, 10, image::Rgba([100, 100, 100, 255]));
    let mut within = base.clone();
    within.put_pixel(0, 0, image::Rgba([102, 98, 100, 255]));
    assert_eq!(
        compare_images(&within, &base).diff_pixel_count,
        0,
        "delta 2 must match"
    );
    let mut beyond = base.clone();
    beyond.put_pixel(0, 0, image::Rgba([103, 100, 100, 255]));
    assert_eq!(
        compare_images(&beyond, &base).diff_pixel_count,
        1,
        "delta 3 must differ"
    );
    // 1 of 100 differing = exactly 0.99, which passes; 2 of 100 fails.
    assert!(compare_images(&beyond, &base).match_percentage >= MATCH_THRESHOLD);
    beyond.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
    assert!(compare_images(&beyond, &base).match_percentage < MATCH_THRESHOLD);
}
