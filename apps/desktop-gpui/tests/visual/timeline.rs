//! Timeline goldens (`timeline-<state>-<theme>.png`) and parity shots: the whole
//! main window over the parity fixtures the Svelte stub serves
//! (`apps/desktop/parity/fixtures/timeline/*.json`).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{px, size, AppContext, Entity, VisualTestAppContext};
use mnema_gpui::shell::{Shell, ShellModel};
use mnema_gpui::theme;
use mnema_gpui::timeline::audio::fixture::{FakePlayer, FixtureAudio};
use mnema_gpui::timeline::audio::Deps;
use mnema_gpui::timeline::fixture::FixtureSource;
use mnema_gpui::timeline::TimelineView;

use super::compare;

pub struct State {
    pub name: &'static str,
    /// `surfaces.ts`'s timeline state (the Tauri / mockup side).
    pub parity: &'static str,
    /// Which parity fixture feeds the source and the shell.
    fixture: &'static str,
    shell: &'static str,
    viewport: (f32, f32),
    /// List fails on the refresh after the first load.
    stale: bool,
    drawer: Drawer,
}

/// Which AudioDrawer state a shot opens (on the fixture's `get_audio_segment`).
#[derive(Clone, Copy, PartialEq)]
enum Drawer {
    None,
    Peek,
    /// Expanded, parked mid-paragraph so the karaoke word shows.
    Expanded,
    /// The repair slide-over on the suggested voice.
    Repair,
}

/// The fixture segment's length (09:00:21 → 09:01:11).
const SEGMENT_S: f64 = 50.;

const MAIN: (f32, f32) = (1280., 800.);
const NARROW: (f32, f32) = (800., 600.);

pub const STATES: [State; 9] = [
    State {
        name: "populated",
        parity: "default",
        fixture: "default",
        shell: "populated",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::None,
    },
    State {
        name: "empty",
        parity: "first-run",
        fixture: "first-run",
        shell: "empty",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::None,
    },
    State {
        name: "stale",
        parity: "stale",
        fixture: "default",
        shell: "populated",
        viewport: MAIN,
        stale: true,
        drawer: Drawer::None,
    },
    State {
        name: "matches",
        parity: "landed",
        fixture: "landed",
        shell: "populated",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::None,
    },
    State {
        name: "narrow",
        parity: "",
        fixture: "default",
        shell: "populated",
        viewport: NARROW,
        stale: false,
        drawer: Drawer::None,
    },
    State {
        name: "drawer-peek",
        parity: "drawer",
        fixture: "drawer",
        shell: "populated",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::Peek,
    },
    State {
        name: "drawer-expanded",
        parity: "drawer-expanded",
        fixture: "drawer",
        shell: "populated",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::Expanded,
    },
    State {
        name: "drawer-repair",
        parity: "repair",
        fixture: "drawer",
        shell: "populated",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::Repair,
    },
    State {
        name: "drawer-no-transcript",
        parity: "drawer-not-transcribed",
        fixture: "drawer-not-transcribed",
        shell: "populated",
        viewport: MAIN,
        stale: false,
        drawer: Drawer::Peek,
    },
];

/// Pump until the timeline has painted its frame (images decode on the
/// background executor) or there is nothing to paint.
pub fn settle(cx: &mut VisualTestAppContext, t: &Entity<TimelineView>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let done = cx.read(|cx| {
            let v = t.read(cx);
            !v.paging.loading && (v.paging.frames.is_empty() || v.painted_frame().is_some())
        });
        if done || Instant::now() > deadline {
            break;
        }
        // The exact frame waits out its 500 ms settle on the simulated clock.
        cx.advance_clock(Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Open the drawer the way a lane click does, then pump its loads and the
/// 420 ms rise to rest.
fn open_drawer(
    cx: &mut VisualTestAppContext,
    window: gpui_kit::AnyWindowHandle,
    timeline: &Entity<TimelineView>,
    state: &State,
) -> anyhow::Result<()> {
    let audio = Arc::new(FixtureAudio::load(state.fixture)?);
    let id = audio
        .opened()
        .map(|s| s.id)
        .expect("fixture get_audio_segment");
    let player = FakePlayer::new(SEGMENT_S);
    cx.update(|cx| {
        timeline.update(cx, |t, cx| {
            t.drawer_deps = Deps {
                backend: audio.clone(),
                open_player: player.factory(),
            };
            t.selected_audio = Some((id, 0));
            cx.notify();
        })
    });
    let pump = |cx: &mut VisualTestAppContext| {
        for _ in 0..8 {
            cx.run_until_parked();
            cx.advance_clock(Duration::from_millis(100));
        }
        cx.run_until_parked();
    };
    pump(cx);
    let drawer = cx
        .read(|cx| timeline.read(cx).drawer.clone())
        .ok_or_else(|| anyhow::anyhow!("the drawer didn't open"))?;
    match state.drawer {
        Drawer::Expanded => cx.update(|cx| {
            drawer.update(cx, |d, cx| {
                d.expanded = true;
                d.seek_to_ms(19_500);
                cx.notify();
            })
        }),
        Drawer::Repair => cx.update_window(window, |_, window, cx| {
            drawer.update(cx, |d, cx| d.open_repair(1, window, cx))
        })?,
        _ => {}
    }
    pump(cx);
    Ok(())
}

pub fn shoot(
    cx: &mut VisualTestAppContext,
    state: &State,
    shell: ShellModel,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    cx.update(|cx| theme::set_setting(setting, cx));
    let source = Arc::new(FixtureSource::load(state.fixture)?);
    let src = source.clone();
    let mut timeline = None;
    let window = cx.open_offscreen_window(
        size(px(state.viewport.0), px(state.viewport.1)),
        |window, cx| {
            let shell = cx.new(|cx| Shell::with_timeline_source(shell, src, window, cx));
            timeline = Some(shell.read(cx).timeline.clone());
            cx.new(|cx| Root::new(shell, window, cx))
        },
    )?;
    let timeline = timeline.expect("timeline");
    settle(cx, &timeline);
    if state.stale {
        *source.fail_list.lock().unwrap() = true;
        cx.update(|cx| timeline.update(cx, |t, cx| t.refresh(cx)));
        settle(cx, &timeline);
    }
    if state.drawer != Drawer::None {
        open_drawer(cx, window.into(), &timeline, state)?;
    }
    // One more pass so the readout and overlays lay out against measured sizes.
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        settle(cx, &timeline);
    }
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
    shell_model: &dyn Fn(&str) -> ShellModel,
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for state in &STATES {
        for (theme_name, setting) in themes {
            let name = format!("timeline-{}-{theme_name}", state.name);
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = shoot(cx, state, shell_model(state.shell), setting.clone())?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}

/// Writes `<dir>/gpui.png` for one `surfaces.ts` timeline state, at 1x.
pub fn parity(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    parity_state: &str,
    setting: AppearanceSetting,
    shell_model: &dyn Fn(&str) -> ShellModel,
) -> anyhow::Result<()> {
    let state = STATES
        .iter()
        .find(|s| s.parity == parity_state)
        .ok_or_else(|| anyhow::anyhow!("no GPUI timeline state for {parity_state}"))?;
    let shot = shoot(cx, state, shell_model(state.shell), setting)?;
    let shot = image::imageops::resize(
        &shot,
        state.viewport.0 as u32,
        state.viewport.1 as u32,
        image::imageops::FilterType::Triangle,
    );
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
