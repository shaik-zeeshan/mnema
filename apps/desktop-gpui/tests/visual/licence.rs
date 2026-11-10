//! Goldens for the shell's licence banner and the deep-link receipt:
//! `licence-<state>-<theme>.png`, the shell over the first-run Timeline with a
//! licence status pushed in (and, for the receipt, a deep-link episode).

use std::sync::Arc;

use capture_types::{Activation, AppearanceSetting, LicenseStatus};
use gpui_kit::base::Root;
use gpui_kit::{AppContext, VisualTestAppContext};
use mnema_gpui::licence::receipt::Flow;
use mnema_gpui::shell::{Shell, ShellModel};
use mnema_gpui::theme;
use mnema_gpui::timeline::fixture::FixtureSource;

use super::{compare, timeline};

struct State {
    name: &'static str,
    status: fn() -> LicenseStatus,
    /// Open the receipt (working face over the status above), then land `result`.
    episode: Option<(Flow, fn() -> LicenseStatus)>,
}

fn licensed(activation: Activation) -> LicenseStatus {
    LicenseStatus::Licensed {
        update_through_ms: 1_819_702_920_000,
        in_window: true,
        email: "owner@example.com".into(),
        name: "Ada Lovelace".into(),
        activation,
    }
}

const STATES: [State; 4] = [
    State {
        name: "banner-trial",
        status: || LicenseStatus::Trial {
            days_left: 2,
            trial_end_ms: 1_700_000_000_000,
        },
        episode: None,
    },
    State {
        name: "banner-lapsed",
        status: || licensed(Activation::Lapsed),
        episode: None,
    },
    State {
        name: "receipt-activated",
        status: || LicenseStatus::TrialNotStarted { trial_days: 30 },
        episode: Some((Flow::Claim, || licensed(Activation::Activated))),
    },
    State {
        name: "receipt-overcap",
        status: || LicenseStatus::TrialNotStarted { trial_days: 30 },
        episode: Some((Flow::Activate, || {
            licensed(Activation::RefusedOverCap {
                reset_url: "https://license.example/reset".into(),
                buy_url: "https://mnema.day/#pricing".into(),
            })
        })),
    },
];

fn shoot(
    cx: &mut VisualTestAppContext,
    state: &State,
    model: ShellModel,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    cx.update(|cx| theme::set_setting(setting, cx));
    let source = Arc::new(FixtureSource::load("first-run")?);
    let mut parts = None;
    let window = cx.open_offscreen_window(compare::viewport((1280., 800.)), |window, cx| {
        let shell = cx.new(|cx| Shell::with_timeline_source(model, source, window, cx));
        let s = shell.read(cx);
        parts = Some((s.timeline.clone(), s.licence.clone()));
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    let (tl, licence) = parts.expect("shell parts");
    cx.update(|cx| {
        licence.update(cx, |l, cx| {
            l.on_status(Some((state.status)()), cx);
            if let Some((flow, result)) = state.episode {
                l.start(flow, cx);
                l.on_status(Some(result()), cx);
            }
        })
    });
    for _ in 0..3 {
        timeline::settle(cx, &tl);
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
    }
    timeline::settle(cx, &tl);
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
    idle: &ShellModel,
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for state in &STATES {
        for (theme_name, setting) in themes {
            let name = format!("licence-{}-{theme_name}", state.name);
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = shoot(cx, state, idle.clone(), setting.clone())?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}
