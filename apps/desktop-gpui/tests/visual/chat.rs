//! Chat goldens (`chat-<state>-<theme>.png`): the shell on the Chat surface,
//! filled from `apps/desktop/parity/fixtures/chat/<state>.json` through the
//! real types, and the parity shot for `scripts/parity.sh chat <state>`.

use std::path::Path;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{AppContext, VisualTestAppContext};
use mnema_gpui::chat::fixture::{chat_fixture, STATES};
use mnema_gpui::shell::{Shell, ShellModel, Surface};
use mnema_gpui::theme;

use super::compare;

pub const VIEWPORT: (f32, f32) = (1280., 800.);

pub fn render(
    cx: &mut VisualTestAppContext,
    shell_model: ShellModel,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    let fx = chat_fixture(state)?;
    cx.update(|cx| theme::set_setting(setting, cx));
    let window = crate::pointer::open(cx, compare::viewport(VIEWPORT), |window, cx| {
        let shell = cx.new(|cx| {
            let mut shell = Shell::new(shell_model, window, cx);
            shell.go(Surface::Chat, cx);
            shell
        });
        let chat = shell.read(cx).chat.clone();
        chat.update(cx, |chat, cx| {
            chat.set_data(fx.data.clone(), cx);
            if let Some(convo) = &fx.open {
                chat.show(convo, fx.snapshot.as_ref(), cx);
            }
        });
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _| window.refresh())?;
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
    shell_model: &ShellModel,
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for state in STATES {
        for (theme_name, setting) in themes {
            let name = format!("chat-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(cx, shell_model.clone(), state, setting.clone())?;
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
    shell_model: ShellModel,
) -> anyhow::Result<()> {
    anyhow::ensure!(STATES.contains(&state), "no chat parity state {state}");
    let shot = render(cx, shell_model, state, setting)?;
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
