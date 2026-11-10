//! Goldens: every surface × state × theme rendered offscreen through real Metal
//! and compared with `tests/visual/baselines/<surface>-<state>-<theme>.png`.
//! `harness = false`: AppKit needs the process main thread, which libtest doesn't give.
//!
//! `cargo test -p mnema-gpui --test visual -- shell` runs the goldens whose name
//! contains `shell` (`-- gallery`: the control gallery's). With `PARITY_DIR` /
//! `PARITY_STATE` / `PARITY_THEME` set (scripts/parity.sh), it renders that one
//! state at the Tauri shot's size and writes `$PARITY_DIR/gpui.png` instead.

#[path = "visual/chat.rs"]
mod chat;
#[path = "visual/compare.rs"]
mod compare;
#[path = "visual/gallery.rs"]
mod gallery;
#[path = "visual/insights.rs"]
mod insights;
#[path = "visual/quick_recall.rs"]
mod quick_recall;
#[path = "visual/onboarding.rs"]
mod onboarding;
#[path = "visual/settings.rs"]
mod settings;
#[path = "visual/small.rs"]
mod small;
#[path = "visual/timeline.rs"]
mod timeline;

use std::path::PathBuf;
use std::sync::Arc;

use app_core::fixtures::{self, FixtureId};
use app_core::AppCore;
use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{px, size, AppContext, VisualTestAppContext, WindowAppearance};
use mnema_gpui::assets::MnemaAssets;
use mnema_gpui::host::GpuiHost;
use mnema_gpui::shell::{data, fixture, Overlay, Shell, ShellModel};
use mnema_gpui::theme::{self, fonts};
use mnema_gpui::timeline::fixture::FixtureSource;
use mnema_gpui::{keymap, ui::clock};

/// The Tauri shot's viewport for the main window (`apps/desktop/parity/surfaces.ts`).
const MAIN: (f32, f32) = (1280., 800.);
/// Below both shell breakpoints (1000 / 900 px).
const NARROW: (f32, f32) = (800., 600.);

struct State {
    name: &'static str,
    fixture: FixtureId,
    overlay: Option<Overlay>,
    viewport: (f32, f32),
}

const SHELL_STATES: [State; 6] = [
    State {
        name: "populated",
        fixture: FixtureId::Populated,
        overlay: None,
        viewport: MAIN,
    },
    State {
        name: "empty",
        fixture: FixtureId::Empty,
        overlay: None,
        viewport: MAIN,
    },
    State {
        name: "recording",
        fixture: FixtureId::Populated,
        overlay: Some(Overlay::Status),
        viewport: MAIN,
    },
    State {
        name: "narrow",
        fixture: FixtureId::Populated,
        overlay: None,
        viewport: NARROW,
    },
    State {
        name: "notifications",
        fixture: FixtureId::Populated,
        overlay: Some(Overlay::Notifications),
        viewport: MAIN,
    },
    State {
        name: "shortcuts",
        fixture: FixtureId::Empty,
        overlay: Some(Overlay::Shortcuts),
        viewport: MAIN,
    },
];

const THEMES: [(&str, AppearanceSetting); 2] = [
    ("dark", AppearanceSetting::Dark),
    ("light", AppearanceSetting::Light),
];

/// The shell's model for a fixture: seed app-core's data dir, load what app-core
/// answers, then the P0 parity map (what the Svelte stub serves) through the real
/// types, so GPUI and Tauri show the same numbers.
fn shell_model(id: FixtureId) -> anyhow::Result<ShellModel> {
    let dir = tempfile::tempdir()?;
    let paths = fixtures::seed(dir.path(), id)?;
    let (host, _rx) = GpuiHost::new();
    let core = Arc::new(AppCore::new(Arc::new(host), paths)?);
    let from_core = core.block_on(data::load(&core));
    let map = fixture::load("shell", id.name())?;
    let model = fixture::shell_model(&map)?;
    // A bare core (no managed states) answers nothing; the live path is covered
    // by `src/live_tests`. The goldens show the parity map.
    debug_assert!(from_core.notifications.is_empty());
    Ok(model)
}

fn render(
    cx: &mut VisualTestAppContext,
    model: ShellModel,
    state: &State,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    cx.update(|cx| theme::set_setting(setting, cx));
    let overlay = state.overlay;
    // The Timeline under the shell shows the same parity data the Tauri shot does.
    let fixture = if state.fixture == FixtureId::Populated {
        "default"
    } else {
        "first-run"
    };
    let source = Arc::new(FixtureSource::load(fixture)?);
    let mut timeline = None;
    let window = cx.open_offscreen_window(
        size(px(state.viewport.0), px(state.viewport.1)),
        |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = Shell::with_timeline_source(model, source, window, cx);
                shell.overlay = overlay;
                shell
            });
            timeline = Some(shell.read(cx).timeline.clone());
            cx.new(|cx| Root::new(shell, window, cx))
        },
    )?;
    let timeline = timeline.expect("timeline");
    for _ in 0..3 {
        timeline::settle(cx, &timeline);
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
    }
    timeline::settle(cx, &timeline);
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

fn run() -> anyhow::Result<()> {
    compare::self_check();
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let parity = std::env::var_os("PARITY_DIR").map(PathBuf::from);

    let platform = gpui_kit::platform::current_platform(false);
    let mut cx = VisualTestAppContext::with_asset_source(platform, Arc::new(MnemaAssets));
    cx.update(|cx| -> anyhow::Result<()> {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        anyhow::ensure!(
            fonts::weights_resolve(cx),
            "fonts: 400/500/600 did not resolve"
        );
        // Goldens show end states: no cross-fade, no popover rise.
        cx.set_reduce_motion(true);
        keymap::bind(&keymap::Bindings::default(), cx);
        clock::freeze(Some(fixtures::fixture_now()), cx);
        Ok(())
    })?;

    let models = [
        (FixtureId::Populated, shell_model(FixtureId::Populated)?),
        (FixtureId::Empty, shell_model(FixtureId::Empty)?),
    ];
    let model_for = |id: FixtureId| {
        models
            .iter()
            .find(|(m, _)| *m == id)
            .map(|(_, m)| m.clone())
    };

    if let Some(dir) = parity {
        if !filters.is_empty() && filters.iter().all(|f| "gallery".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            return gallery::parity(&mut cx, &dir, &state, setting.clone());
        }
        if !filters.is_empty() && filters.iter().all(|f| "timeline".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            let shell_for = |name: &str| model_for(FixtureId::from_name(name).unwrap()).unwrap();
            return timeline::parity(&mut cx, &dir, &state, setting.clone(), &shell_for);
        }
        if !filters.is_empty() && filters.iter().all(|f| "chat".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            let model = model_for(FixtureId::Populated).unwrap();
            return chat::parity(&mut cx, &dir, &state, setting.clone(), model);
        }
        if !filters.is_empty() && filters.iter().all(|f| "insights".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            return insights::parity(&mut cx, &dir, &state, setting.clone());
        }
        if !filters.is_empty() && filters.iter().all(|f| "settings".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            // Not recording, like the stubbed Tauri page (no capture session).
            let model = model_for(FixtureId::Empty).unwrap();
            return settings::parity(&mut cx, &dir, &state, setting.clone(), model);
        }
        if !filters.is_empty() && filters.iter().all(|f| "quick-recall".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            return quick_recall::parity(&mut cx, &dir, &state, setting.clone());
        }
        if !filters.is_empty() && filters.iter().all(|f| "onboarding".contains(f.as_str())) {
            let state = std::env::var("PARITY_STATE")?;
            let theme_name = std::env::var("PARITY_THEME")?;
            let (_, setting) = THEMES
                .iter()
                .find(|(t, _)| *t == theme_name)
                .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
            return onboarding::parity(&mut cx, &dir, &state, setting.clone());
        }
        if let [f] = filters.as_slice() {
            if small::SURFACES.contains(&f.as_str()) {
                let state = std::env::var("PARITY_STATE")?;
                let theme_name = std::env::var("PARITY_THEME")?;
                let (_, setting) = THEMES
                    .iter()
                    .find(|(t, _)| *t == theme_name)
                    .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
                return small::parity(&mut cx, &dir, f, &state, setting.clone());
            }
        }
        if !filters.iter().all(|f| "shell".contains(f.as_str())) {
            return Ok(()); // another surface's parity run
        }
        let state_name = std::env::var("PARITY_STATE")?;
        let theme_name = std::env::var("PARITY_THEME")?;
        let state = SHELL_STATES
            .iter()
            .find(|s| s.name == state_name && s.viewport == MAIN)
            .ok_or_else(|| anyhow::anyhow!("no shell state {state_name}"))?;
        let (_, setting) = THEMES
            .iter()
            .find(|(t, _)| *t == theme_name)
            .ok_or_else(|| anyhow::anyhow!("no theme {theme_name}"))?;
        let shot = render(
            &mut cx,
            model_for(state.fixture).unwrap(),
            state,
            setting.clone(),
        )?;
        // Same size as the Tauri shot (Playwright renders at 1x; we capture at 2x).
        let shot = image::imageops::resize(
            &shot,
            MAIN.0 as u32,
            MAIN.1 as u32,
            image::imageops::FilterType::Triangle,
        );
        shot.save(dir.join("gpui.png"))?;
        println!("  parity: wrote {}", dir.join("gpui.png").display());
        return Ok(());
    }

    let mut failures = Vec::new();
    for state in &SHELL_STATES {
        for (theme_name, setting) in &THEMES {
            let name = format!("shell-{}-{theme_name}", state.name);
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(
                &mut cx,
                model_for(state.fixture).unwrap(),
                state,
                setting.clone(),
            )?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    failures.extend(gallery::goldens(&mut cx, &filters, &THEMES)?);
    let shell_for = |name: &str| model_for(FixtureId::from_name(name).unwrap()).unwrap();
    failures.extend(timeline::goldens(&mut cx, &filters, &THEMES, &shell_for)?);
    let populated = model_for(FixtureId::Populated).unwrap();
    failures.extend(chat::goldens(&mut cx, &filters, &THEMES, &populated)?);
    let idle = model_for(FixtureId::Empty).unwrap();
    failures.extend(settings::goldens(&mut cx, &filters, &THEMES, &idle)?);
    failures.extend(insights::goldens(&mut cx, &filters, &THEMES)?);
    failures.extend(small::goldens(&mut cx, &filters, &THEMES)?);
    failures.extend(quick_recall::goldens(&mut cx, &filters, &THEMES)?);
    failures.extend(onboarding::goldens(&mut cx, &filters, &THEMES)?);
    anyhow::ensure!(failures.is_empty(), failures.join("\n"));
    Ok(())
}

fn main() {
    println!("visual:");
    match std::panic::catch_unwind(run) {
        Ok(Ok(())) => println!("visual: ok"),
        Ok(Err(e)) => {
            eprintln!("visual FAILED: {e:#}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("visual panicked");
            std::process::exit(1);
        }
    }
}
