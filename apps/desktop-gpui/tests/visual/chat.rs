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
    for (theme_name, setting) in themes {
        let name = format!("chat-peek-{theme_name}");
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        let shot = render_peek(cx, shell_model.clone(), setting.clone())?;
        if let Err(e) = compare::check_golden(&name, &shot) {
            failures.push(e.to_string());
        }
    }
    Ok(failures)
}

/// `chat-peek`: the `done` chat with its first frame chip's peek open
/// (`FrameDetailModal`): the redaction badge and the app avatar's letter, from
/// the same frame `parity/fixtures/chat/peek.json` serves the Svelte side.
pub fn render_peek(
    cx: &mut VisualTestAppContext,
    shell_model: ShellModel,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    use mnema_gpui::chat::fixture::FixtureSources;
    let fx = chat_fixture("done")?;
    let source = fx
        .open
        .as_ref()
        .and_then(|c| c.turns.first())
        .map(|t| mnema_gpui::chat::model::sources(&t.sources))
        .and_then(|s| s.into_iter().find(|s| s.kind == "frame"))
        .ok_or_else(|| anyhow::anyhow!("the done chat cites no frame"))?;
    let id = source.frame_id.unwrap_or_default();
    let shot_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/assets/shots/desk-code.webp");
    let sources = std::sync::Arc::new(FixtureSources {
        frames: [(id, (Some(2880), Some(1800)))].into(),
        previews: [(id, shot_path)].into(),
        redactions: [(id, 2)].into(),
        bundles: [(id, "dev.zed.Zed".to_string())].into(),
        ..Default::default()
    });
    cx.update(|cx| theme::set_setting(setting, cx));
    let mut chat_entity = None;
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
            chat.sources = sources;
        });
        chat_entity = Some(chat);
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    let chat = chat_entity.expect("chat");
    cx.update_window(window.into(), |_, window, cx| {
        chat.update(cx, |c, cx| c.peek_frame(source, window, cx))
    })?;
    // Past the modal's 420 ms rise and scrim fade (animations read real time).
    for _ in 0..25 {
        cx.run_until_parked();
        cx.advance_clock(std::time::Duration::from_millis(50));
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

/// Writes `<dir>/gpui.png` for one parity state, at the Tauri shot's 1x size.
pub fn parity(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    state: &str,
    setting: AppearanceSetting,
    shell_model: ShellModel,
) -> anyhow::Result<()> {
    let shot = if state == "picker" {
        crate::layers::chat_picker(cx, shell_model, setting)?
    } else {
        anyhow::ensure!(STATES.contains(&state), "no chat parity state {state}");
        render(cx, shell_model, state, setting)?
    };
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
