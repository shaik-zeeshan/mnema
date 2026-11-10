//! About › Developer & Logs (`Developer.svelte`): developer options, the
//! developer window, and the general application log (open / delete).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use crate::settings::model::Json;
use crate::settings::row::{row, section};
use crate::settings::{click, on, Settings};
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::switch::Switch;
use crate::ui::Button;

fn log_action(s: &mut Settings, delete: bool, cx: &mut Context<Settings>) {
    let Some(core) = crate::chat::core(cx) else {
        return;
    };
    let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
        if delete {
            app_core::general_app_log::delete(&core).map(|_| ())
        } else {
            app_core::general_app_log::open(&core).map(|_| ())
        }
    });
    cx.spawn(async move |this, cx| {
        let failed = match job.await {
            Ok(Err(e)) => Some(e.message),
            _ => None,
        };
        let _ = this.update(cx, |s, cx| {
            if let Some(e) = failed {
                log::warn!("settings: app log action failed: {e}");
                s.ui.note = Some((false, "App log", e));
            }
            s.reload_all(cx)
        });
    })
    .detach();
    let _ = s;
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let dev = d.developer_options_enabled;
    let keys = crate::keymap::display(&s.kb_draft.as_ref().map_or(
        "CommandOrControl+D".to_string(),
        |k| {
            k["appShortcuts"]["openDebug"]
                .as_str()
                .unwrap_or("CommandOrControl+D")
                .to_string()
        },
    ));
    let log = s.data.log_status.clone();
    let exists = log.as_ref().is_some_and(|l| l.bool_at(&["exists"]));
    let log_body = match &log {
        None => div()
            .text_size(text::BASE)
            .text_color(p.text_muted)
            .child("Loading log status\u{2026}"),
        Some(l) => {
            let field = |k: &'static str, v: String, mono: bool| {
                div()
                    .flex()
                    .gap(space::S3)
                    .text_size(text::BASE)
                    .child(div().w(px(40.)).text_color(p.text_subtle).child(k))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(p.text)
                            .when(mono, |d| d.font_family(FONT_MONO).text_size(text::SM))
                            .child(v),
                    )
            };
            div()
                .flex()
                .flex_col()
                .gap(space::S3)
                .w_full()
                .child(field(
                    "Path",
                    l.str_at(&["path"]).unwrap_or_default().to_string(),
                    true,
                ))
                .child(field(
                    "File",
                    if exists {
                        "Exists on disk".into()
                    } else {
                        "Not found".into()
                    },
                    false,
                ))
                .child(
                    div()
                        .flex()
                        .gap(space::S2)
                        .child(
                            Button::new(
                                "log-open",
                                if exists {
                                    "Open Log File"
                                } else {
                                    "Open Containing Folder"
                                },
                            )
                            .sm()
                            .on_click(click(cx, |s, cx| log_action(s, false, cx))),
                        )
                        .when(exists, |d| {
                            d.child(
                                Button::new("log-delete", "Delete Log File")
                                    .danger()
                                    .sm()
                                    .on_click(click(cx, |s, cx| log_action(s, true, cx))),
                            )
                        }),
                )
        }
    };
    let rows = vec![
        row("Enable developer options")
            .desc("Turns on the developer window and keeps detailed entries in the app log. Changes apply immediately.")
            .control(
                Switch::new("dev-options", "Enable developer options", dev)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.developer_options_enabled = v, cx))),
            )
            .build(p)
            .into_any_element(),
        row("Developer window")
            .desc("Health, queues, capture and logs in a small window you can keep beside the app.")
            .control(
                Button::new("dev-window", "Open")
                    .sm()
                    .kbd(keys)
                    .soft_disabled(!dev)
                    // The developer window is S15; the action is the shell's ⌘D.
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(crate::settings::SettingsEvent::OpenDebug))),
            )
            .build(p)
            .when(!dev, |d| d.opacity(theme::DISABLED_OPACITY))
            .into_any_element(),
        row("General application log")
            .desc("Captures high-level runtime events and errors.")
            .full()
            .no_divider()
            .control(log_body)
            .build(p)
            .into_any_element(),
    ];
    section("Developer & Logs", None, Vec::new(), false, rows, p).into_any_element()
}
