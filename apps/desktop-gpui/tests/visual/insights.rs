//! Insights goldens (`insights-<state>-<theme>.png`): the shell on the Insights
//! surface over `apps/desktop/parity/fixtures/insights/<set>.json` (every tab ×
//! {populated, empty}, a subject's page with its drawer, and an open receipt),
//! and the parity shot for `scripts/parity.sh insights <state>`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{point, px, AppContext, VisualTestAppContext};
use mnema_gpui::insights::fixture::{FixtureBackend, Open, STATES};
use mnema_gpui::shell::{fixture, Shell, Surface};
use mnema_gpui::theme;

use super::compare;

pub const VIEWPORT: (f32, f32) = (1280., 800.);

pub fn render(
    cx: &mut VisualTestAppContext,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    let st = STATES
        .iter()
        .find(|s| s.name == state)
        .ok_or_else(|| anyhow::anyhow!("no insights state {state}"))?;
    // The shell around it reads the same map the Svelte stub serves.
    let shell_model = fixture::shell_model(&fixture::load("insights", st.set)?)?;
    let backend = Arc::new(FixtureBackend::load(st.set)?);
    cx.update(|cx| theme::set_setting(setting, cx));
    let mut insights = None;
    let window = crate::pointer::open(cx, compare::viewport(VIEWPORT), |window, cx| {
        let shell = cx.new(|cx| {
            let mut shell = Shell::new(shell_model, window, cx);
            shell.go(Surface::Insights, cx);
            shell
        });
        let i = shell.read(cx).insights.clone();
        i.update(cx, |i, cx| {
            i.backend = backend;
            i.tab = st.tab;
            i.reload(cx);
        });
        insights = Some(i);
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    let insights = insights.expect("insights");
    cx.run_until_parked();
    match st.open {
        Open::None => {}
        Open::Subject => {
            let first = cx.read(|cx| {
                insights
                    .read(cx)
                    .subjects
                    .as_ref()
                    .and_then(|s| s.conclusions.first().map(|c| c.subject.clone()))
            });
            if let Some(s) = first {
                cx.update(|cx| insights.update(cx, |i, cx| i.open_subject(s, cx)));
                cx.run_until_parked();
                let lead = cx.read(|cx| {
                    insights
                        .read(cx)
                        .detail
                        .as_ref()
                        .and_then(|d| d.view.as_ref())
                        .map(|v| v.conclusions[0].id)
                });
                cx.update(|cx| {
                    insights.update(cx, |i, cx| {
                        i.open_conclusion = lead;
                        cx.notify();
                    })
                });
            }
        }
        Open::Receipt => {
            let newest = cx.read(|cx| {
                insights.read(cx).journal.as_ref().and_then(|j| {
                    j.activities
                        .iter()
                        .max_by_key(|a| a.started_at_ms)
                        .map(|a| (a.id, (a.started_at_ms, a.ended_at_ms)))
                })
            });
            if let Some((id, span)) = newest {
                cx.update(|cx| {
                    insights.update(cx, |i, cx| {
                        i.toggle_receipt(id, span, cx);
                        // The card sits under the lede and the spine.
                        i.scroll.set_offset(point(px(0.), px(-560.)));
                    })
                });
            }
        }
    }
    // Loads land, images decode (FrameImages loads in the draw), then settle.
    let deadline = Instant::now() + Duration::from_secs(10);
    for _ in 0..6 {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        cx.advance_clock(Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(20));
        if Instant::now() > deadline {
            break;
        }
    }
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for st in &STATES {
        for (theme_name, setting) in themes {
            let name = format!("insights-{}-{theme_name}", st.name);
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(cx, st.name, setting.clone())?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}

/// Writes `<dir>/gpui.png` for one parity state, at the Tauri shot's 1x size.
pub fn parity(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<()> {
    let shot = render(cx, state, setting)?;
    let shot = image::imageops::resize(
        &shot,
        VIEWPORT.0 as u32,
        VIEWPORT.1 as u32,
        image::imageops::FilterType::Triangle,
    );
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
