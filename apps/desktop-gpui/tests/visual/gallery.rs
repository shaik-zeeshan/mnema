//! Gallery goldens: one PNG per control group per theme
//! (`gallery-<group>-<theme>.png`), and the parity shots against the kit
//! gallery (`docs/mockups/redesign/index.html`): `components` and `feedback`.

use std::path::Path;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{px, size, AppContext, VisualTestAppContext};
use mnema_gpui::gallery::{Gallery, Group};
use mnema_gpui::theme;

use super::compare;

fn shoot(
    cx: &mut VisualTestAppContext,
    groups: Vec<Group>,
    viewport: (f32, f32),
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    cx.update(|cx| theme::set_setting(setting, cx));
    let window = cx.open_offscreen_window(size(px(viewport.0), px(viewport.1)), |window, cx| {
        let gallery = cx.new(|cx| Gallery::new(groups, window, cx));
        cx.new(|cx| Root::new(gallery, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _| window.refresh())?;
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

/// Every group × theme whose golden name contains a filter (all when none).
pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for g in Group::ALL {
        for (theme_name, setting) in themes {
            let name = format!("gallery-{}-{theme_name}", g.name());
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = shoot(cx, vec![g], g.viewport(), setting.clone())?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}

/// The parity states: the mockup's "Components" and "Feedback parts" panes.
pub fn parity_groups(state: &str) -> Option<Vec<Group>> {
    match state {
        "components" => Some(vec![
            Group::Buttons,
            Group::Fields,
            Group::Choice,
            Group::Date,
            Group::Status,
        ]),
        "feedback" => Some(vec![
            Group::Empty,
            Group::Feedback,
            Group::Loading,
            Group::Overlays,
        ]),
        _ => None,
    }
}

/// Writes `<dir>/gpui.png` for one parity state, at 1x.
pub fn parity(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<()> {
    let groups = parity_groups(state).ok_or_else(|| anyhow::anyhow!("no gallery state {state}"))?;
    // One window per group, stacked at 1x: a single tall window gets clamped
    // to the display and comes back stretched.
    let mut shots = Vec::new();
    for g in groups {
        let (w, h) = g.viewport();
        let shot = shoot(cx, vec![g], (w, h), setting.clone())?;
        shots.push(image::imageops::resize(
            &shot,
            w as u32,
            h as u32,
            image::imageops::FilterType::Triangle,
        ));
    }
    let width = shots.iter().map(|s| s.width()).max().unwrap_or(0);
    let mut shot = image::RgbaImage::new(width, shots.iter().map(|s| s.height()).sum());
    let mut y = 0;
    for s in &shots {
        image::imageops::replace(&mut shot, s, 0, y as i64);
        y += s.height();
    }
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
