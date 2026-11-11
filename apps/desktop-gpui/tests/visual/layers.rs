//! Floating layers opened over the content that follows them in the tree (the
//! class of bug where a later sibling paints over an `absolute` menu): each
//! golden opens one layer over real rows at the surface's real viewport.

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext, VisualTestAppContext};
use mnema_gpui::chat::fixture::chat_fixture;
use mnema_gpui::settings::fixture::settings_data;
use mnema_gpui::settings::Settings;
use mnema_gpui::shell::{Shell, ShellModel, Surface};
use mnema_gpui::theme;

use super::compare;

/// `(golden state, settings tab, what to open, then what to click)`.
type Open = fn(&mut Settings);
const SETTINGS: [(&str, &str, Open, Option<&str>); 3] = [
    // The menu over the providers, default model and Test connection rows.
    (
        "add-provider-open",
        "intelligence",
        |s| s.ui.add_provider_open = true,
        None,
    ),
    // The Add connector picker (a modal) over the MCP rows.
    (
        "add-connector-open",
        "intelligence",
        |s| s.ui.mcp_picker_open = true,
        None,
    ),
    // "Pause capture when idle": a Select ~70 px above the window's bottom edge.
    ("idle-timeout-open", "capture", |_| {}, Some("idle-timeout")),
];

/// Clicks element `id` (a real hit test), then parks the pointer away from it.
pub fn click(
    cx: &mut VisualTestAppContext,
    window: AnyWindowHandle,
    id: &str,
) -> anyhow::Result<()> {
    let id = gpui_kit::ElementId::Name(id.to_string().into());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
        crate::pointer::park(window, cx);
    })?;
    cx.run_until_parked();
    Ok(())
}

/// Chat with a thread open (docked composer at the window's foot), the scope
/// panel shown and its range calendar clicked open; `span` = scoped Sat Oct 24 –
/// Sat Oct 31 (both caps and a row break inside one month), else all time.
fn chat_scope(
    cx: &mut VisualTestAppContext,
    model: ShellModel,
    setting: AppearanceSetting,
    span: bool,
) -> anyhow::Result<image::RgbaImage> {
    let fx = chat_fixture("done")?;
    cx.update(|cx| theme::set_setting(setting, cx));
    let window = crate::pointer::open(
        cx,
        compare::viewport(crate::chat::VIEWPORT),
        |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = Shell::new(model, window, cx);
                shell.go(Surface::Chat, cx);
                shell
            });
            let chat = shell.read(cx).chat.clone();
            chat.update(cx, |chat, cx| {
                chat.set_data(fx.data.clone(), cx);
                if let Some(convo) = &fx.open {
                    chat.show(convo, fx.snapshot.as_ref(), cx);
                }
                chat.ctx_open = true;
                if span {
                    let today = mnema_gpui::ui::clock::now(cx).date();
                    chat.scope.start = Some(today - time::Duration::days(11));
                    chat.scope.end = Some(today - time::Duration::days(4));
                }
            });
            cx.new(|cx| Root::new(shell, window, cx))
        },
    )?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _| window.refresh())?;
    cx.run_until_parked();
    click(cx, window.into(), "chat-scope-range")?;
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
    idle: &ShellModel,
    populated: &ShellModel,
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for (state, tab, open, click) in SETTINGS {
        for (theme_name, setting) in themes {
            let name = format!("settings-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = crate::settings::render_open(
                cx,
                idle.clone(),
                tab,
                settings_data()?,
                setting.clone(),
                click,
                move |s, _, cx| {
                    open(s);
                    cx.notify();
                },
            )?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    for (state, span) in [("range-open", false), ("span-open", true)] {
        for (theme_name, setting) in themes {
            let name = format!("chat-scope-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = chat_scope(cx, populated.clone(), setting.clone(), span)?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}
