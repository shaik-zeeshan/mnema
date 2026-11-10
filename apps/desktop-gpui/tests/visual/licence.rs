//! Goldens for the shell's licence banner and the deep-link receipt:
//! `licence-<state>-<theme>.png`, the shell over the first-run Timeline with a
//! licence status pushed in (and, for the receipt, a deep-link episode). With
//! `RECEIPT_SHOTS_OUT=<dir>` it instead writes every receipt face (dialog
//! crops, and frames across the ruler's scrub) for design review.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_types::{Activation, AppearanceSetting, LicenseStatus, ResetDevicesOutcome};
use gpui_kit::base::Root;
use gpui_kit::{AppContext, Context, VisualTestAppContext};
use mnema_gpui::licence::receipt::{self, Done, Flow};
use mnema_gpui::licence::Licence;
use mnema_gpui::shell::{Shell, ShellModel};
use mnema_gpui::theme;
use mnema_gpui::timeline::fixture::FixtureSource;

use super::{compare, timeline};

type Land = fn(&mut Licence, &mut Context<Licence>);

struct State {
    name: &'static str,
    status: fn() -> LicenseStatus,
    /// Open the receipt (working face over the status above), then land the result.
    episode: Option<(Flow, Land)>,
}

const THROUGH_MS: i64 = 1_819_702_920_000;
/// A year (and a leap day) later: the renewed window.
const RENEWED_MS: i64 = THROUGH_MS + 366 * 86_400_000;

fn licensed_through(update_through_ms: i64, activation: Activation) -> LicenseStatus {
    LicenseStatus::Licensed {
        update_through_ms,
        in_window: true,
        email: "owner@example.com".into(),
        name: "Ada Lovelace".into(),
        activation,
    }
}

fn licensed(activation: Activation) -> LicenseStatus {
    licensed_through(THROUGH_MS, activation)
}

fn over_cap() -> LicenseStatus {
    licensed(Activation::RefusedOverCap {
        reset_url: "https://license.example/reset".into(),
        buy_url: "https://mnema.day/#pricing".into(),
    })
}

fn fresh() -> LicenseStatus {
    LicenseStatus::TrialNotStarted { trial_days: 30 }
}

const STATES: [State; 6] = [
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
        status: fresh,
        episode: Some((Flow::Claim, |l, cx| {
            l.on_status(Some(licensed(Activation::Activated)), cx)
        })),
    },
    State {
        name: "receipt-overcap",
        status: fresh,
        episode: Some((Flow::Activate, |l, cx| l.on_status(Some(over_cap()), cx))),
    },
    State {
        name: "receipt-renewed",
        status: || licensed(Activation::Activated),
        episode: Some((Flow::Renewed, |l, cx| {
            l.on_status(
                Some(licensed_through(RENEWED_MS, Activation::Activated)),
                cx,
            )
        })),
    },
    State {
        name: "receipt-pending",
        status: fresh,
        episode: Some((Flow::Claim, |l, cx| {
            let pending = Activation::Pending {
                provisional_days_left: 3,
            };
            l.on_status(Some(licensed(pending)), cx)
        })),
    },
];

/// Review-only faces with no golden of their own.
const EXTRA: [State; 3] = [
    State {
        name: "receipt-working",
        status: fresh,
        episode: Some((Flow::Claim, |_, _| {})),
    },
    State {
        name: "receipt-failed",
        status: || licensed(Activation::Activated),
        episode: Some((Flow::Renewed, |l, cx| {
            l.on_done(
                Done::Failed("We couldn't confirm your renewal yet.".into()),
                cx,
            )
        })),
    },
    State {
        name: "receipt-overcap-note",
        status: fresh,
        episode: Some((Flow::Activate, |l, cx| {
            l.on_status(Some(over_cap()), cx);
            let outcome = Ok(ResetDevicesOutcome::RateLimited {
                retry_at_ms: Some(1_796_000_000_000),
            });
            l.receipt.reset_note = Some(receipt::reset_note(&outcome, time::UtcOffset::UTC));
        })),
    },
];

fn shoot(
    cx: &mut VisualTestAppContext,
    state: &State,
    model: ShellModel,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    let mut shots = shoot_frames(cx, state, model, setting, &[])?;
    Ok(shots.pop().expect("one shot"))
}

/// The state at rest, or (with `frames`, in ms since the receipt first paints,
/// ascending) one shot per frame with motion on.
fn shoot_frames(
    cx: &mut VisualTestAppContext,
    state: &State,
    model: ShellModel,
    setting: AppearanceSetting,
    frames: &[u64],
) -> anyhow::Result<Vec<image::RgbaImage>> {
    cx.update(|cx| theme::set_setting(setting, cx));
    let source = Arc::new(FixtureSource::load("first-run")?);
    let mut parts = None;
    let window = crate::pointer::open(cx, compare::viewport((1280., 800.)), |window, cx| {
        let shell = cx.new(|cx| Shell::with_timeline_source(model, source, window, cx));
        let s = shell.read(cx);
        parts = Some((s.timeline.clone(), s.licence.clone()));
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    let (tl, licence) = parts.expect("shell parts");
    let episode = |cx: &mut VisualTestAppContext| {
        cx.update(|cx| {
            licence.update(cx, |l, cx| {
                if let Some((flow, land)) = state.episode {
                    l.start(flow, cx);
                    land(l, cx);
                }
            })
        })
    };
    cx.update(|cx| licence.update(cx, |l, cx| l.on_status(Some((state.status)()), cx)));
    episode(cx);
    for _ in 0..3 {
        timeline::settle(cx, &tl);
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
    }
    timeline::settle(cx, &tl);
    let mut shots = Vec::new();
    if frames.is_empty() {
        shots.push(cx.capture_screenshot(window.into())?);
    } else {
        // Re-open the receipt now the shell is still, so its first paint is t = 0;
        // one frame without it drops its element (and animation) state.
        cx.update(|cx| {
            cx.set_reduce_motion(false);
            licence.update(cx, |l, cx| {
                l.close_receipt(cx);
            })
        });
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        cx.run_until_parked();
        cx.update(|cx| licence.update(cx, |l, cx| l.on_status(Some((state.status)()), cx)));
        episode(cx);
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        cx.run_until_parked();
        let mut at = 0;
        for &ms in frames {
            cx.advance_clock(Duration::from_millis(ms - at));
            at = ms;
            cx.update_window(window.into(), |_, window, _| window.refresh())?;
            cx.run_until_parked();
            shots.push(cx.capture_screenshot(window.into())?);
        }
        cx.update(|cx| cx.set_reduce_motion(true));
    }
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shots)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
    idle: &ShellModel,
) -> anyhow::Result<Vec<String>> {
    if let Some(dir) = std::env::var_os("RECEIPT_SHOTS_OUT") {
        review_shots(cx, Path::new(&dir), themes, idle)?;
        return Ok(Vec::new());
    }
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

/// Design review only: `<face>-<theme>[-f<ms>].png` dialog crops in `dir`
/// (full windows in `dir/full`). Checks no golden, writes no baseline.
fn review_shots(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    themes: &[(&str, AppearanceSetting)],
    idle: &ShellModel,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir.join("full"))?;
    for state in STATES.iter().chain(&EXTRA) {
        let Some(face) = state.name.strip_prefix("receipt-") else {
            continue;
        };
        let frames: &[u64] = match face {
            "activated" => &[100, 250, 450, 700, 1000, 1400],
            "renewed" => &[250, 450, 1000],
            _ => &[],
        };
        for (theme_name, setting) in themes {
            let name = format!("{face}-{theme_name}");
            let rest = shoot(cx, state, idle.clone(), setting.clone())?;
            rest.save(dir.join("full").join(format!("{name}.png")))?;
            zoom(&rest).save(dir.join(format!("{name}.png")))?;
            if !frames.is_empty() {
                let shots = shoot_frames(cx, state, idle.clone(), setting.clone(), frames)?;
                for (ms, shot) in frames.iter().zip(shots) {
                    zoom(&shot).save(dir.join(format!("{name}-f{ms}.png")))?;
                }
            }
            println!("  receipt shots: {name}");
        }
    }
    Ok(())
}

/// The middle of the window, where the receipt sits.
fn zoom(shot: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (1100.min(shot.width()), 1150.min(shot.height()));
    image::imageops::crop_imm(shot, (shot.width() - w) / 2, (shot.height() - h) / 2, w, h)
        .to_image()
}
