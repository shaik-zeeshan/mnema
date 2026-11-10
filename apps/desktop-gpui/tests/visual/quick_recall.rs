//! Quick Recall goldens (`quick-recall-<state>-<theme>.png`): the bar at its
//! measured height over `apps/desktop/parity/fixtures/quick-recall/<state>.json`,
//! driven the way the user gets there (summon text, the pauses, ↓, a pill, Quick
//! Look, an answer), and the parity shot for `scripts/parity.sh quick-recall`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    px, size, transparent_black, AnyWindowHandle, AppContext, Entity, VisualTestAppContext,
};
use mnema_gpui::quick_recall::data::Backend;
use mnema_gpui::quick_recall::fixture::{FixtureBackend, STATES};
use mnema_gpui::quick_recall::{QuickRecall, ASK_PAUSE, COMMIT, SEARCH_DEBOUNCE, WIDTH};
use mnema_gpui::theme;

use super::compare;

/// The Tauri shot's viewport (`surfaces.ts`).
pub const VIEWPORT: (f32, f32) = (800., 700.);

fn settle(cx: &mut VisualTestAppContext, window: AnyWindowHandle) -> anyhow::Result<()> {
    for _ in 0..6 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, _| window.refresh())?;
        // Thumbnails decode on the background executor.
        std::thread::sleep(Duration::from_millis(25));
    }
    cx.run_until_parked();
    Ok(())
}

fn wait(cx: &mut VisualTestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

/// Drive the bar into `state`.
fn drive(cx: &mut VisualTestAppContext, qr: &Entity<QuickRecall>, b: &FixtureBackend, state: &str) {
    if let Some(q) = b.summon_query() {
        qr.update(cx, |qr, cx| qr.take_query(&q, cx));
        wait(cx, COMMIT.max(SEARCH_DEBOUNCE) + ASK_PAUSE);
    }
    match state {
        "quicklook" => qr.update(cx, |qr, cx| {
            if let Some(r) = qr.visible_results().first().cloned() {
                qr.select_result(0);
                qr.look_show(r, cx);
            }
        }),
        "filter" => qr.update(cx, |qr, cx| qr.open_popover(0, cx)),
        "recent" => qr.update(cx, |qr, cx| qr.open_recent(cx)),
        "ask-streaming" | "ask-done" => {
            if let Some(snap) = b.ask_snapshot("") {
                qr.update(cx, |qr, cx| qr.adopt_snapshot(snap, cx));
            }
        }
        _ => {}
    }
    cx.run_until_parked();
}

pub fn render(
    cx: &mut VisualTestAppContext,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    anyhow::ensure!(STATES.contains(&state), "no quick-recall state {state}");
    cx.update(|cx| theme::set_setting(setting, cx));
    let backend = Arc::new(FixtureBackend::load(state)?);
    let b: Arc<dyn Backend> = backend.clone();
    let mut view = None;
    let window = crate::pointer::open(cx, size(px(VIEWPORT.0), px(VIEWPORT.1)), |window, cx| {
        let qr = cx.new(|cx| {
            let mut qr = QuickRecall::new(b, window, cx);
            qr.native_fit = false;
            qr
        });
        view = Some(qr.clone());
        cx.new(|cx| Root::new(qr, window, cx).bg(transparent_black()))
    })?;
    let window: AnyWindowHandle = window.into();
    let qr = view.expect("view");
    settle(cx, window)?;
    drive(cx, &qr, &backend, state);
    settle(cx, window)?;
    let shot = cx.capture_screenshot(window)?;
    // The panel is the bar's own height: crop the offscreen window to it.
    let h = qr.read_with(cx, |qr, _| qr.content_height());
    let scale = shot.width() as f32 / VIEWPORT.0;
    let rows = ((h * scale).ceil() as u32).min(shot.height());
    let shot = image::imageops::crop_imm(&shot, 0, 0, shot.width(), rows).to_image();
    cx.update_window(window, |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for state in STATES {
        for (theme_name, setting) in themes {
            let name = format!("quick-recall-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(cx, state, setting.clone())?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}

/// Writes `<dir>/gpui.png` at the Tauri shot's 1x width.
pub fn parity(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<()> {
    let shot = render(cx, state, setting)?;
    let scale = shot.width() as f32 / WIDTH;
    let shot = image::imageops::resize(
        &shot,
        WIDTH as u32,
        (shot.height() as f32 / scale) as u32,
        image::imageops::FilterType::Triangle,
    );
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
