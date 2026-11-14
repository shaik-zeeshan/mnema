//! About › Developer & Logs (`Developer.svelte`): developer options, the
//! developer window, and the general application log (open / delete).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use crate::settings::model::Json;
use crate::settings::row::{row, section};
use crate::settings::{click, on, Settings};
use crate::theme::{space, text, Palette, FONT_MONO};
use crate::ui::switch::Switch;
use crate::ui::Button;
use crate::ui::Tracking as _;

const OPENING: &str = "Open app log";
const DELETING: &str = "Delete app log";

/// Open (or reveal the folder) / delete the general app log; its new status
/// replaces the old one, a failure stays on the row until dismissed.
fn log_action(s: &mut Settings, delete: bool, cx: &mut Context<Settings>) {
    let Some(core) = crate::chat::core(cx) else {
        return;
    };
    let what = if delete { DELETING } else { OPENING };
    if !s.ui.busy.insert(what) {
        return;
    }
    s.ui.log_error = None;
    s.ui.log_deleted = false;
    let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
        if delete {
            app_core::general_app_log::delete(&core)
        } else {
            app_core::general_app_log::open(&core)
        }
        .map_err(|e| e.message)
        .and_then(|status| serde_json::to_value(status).map_err(|e| e.to_string()))
    });
    cx.spawn(async move |this, cx| {
        let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
        let _ = this.update(cx, |s, cx| {
            s.ui.busy.remove(what);
            match result {
                Ok(status) => {
                    s.data.log_status = Some(status);
                    if delete {
                        s.ui.log_deleted = true;
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(2200))
                                .await;
                            let _ = this.update(cx, |s, cx| {
                                s.ui.log_deleted = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                }
                Err(e) => {
                    log::warn!("settings: app log action failed: {e}");
                    s.ui.log_error = Some(e);
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
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
    let opening = s.ui.busy.contains(OPENING);
    let deleting = s.ui.busy.contains(DELETING);
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
                    .child(
                        div()
                            .w(px(40.))
                            .tracking(0.1)
                            .text_color(p.text_subtle)
                            .child(k),
                    )
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
                        .items_center()
                        .gap(space::S2)
                        .child(
                            Button::new(
                                "log-open",
                                if opening {
                                    "Opening\u{2026}"
                                } else if exists {
                                    "Open Log File"
                                } else {
                                    "Open Containing Folder"
                                },
                            )
                            .sm()
                            .busy(opening)
                            .disabled(opening)
                            .on_click(click(cx, |s, cx| log_action(s, false, cx))),
                        )
                        .when(exists, |d| {
                            d.child(
                                Button::new(
                                    "log-delete",
                                    if deleting {
                                        "Deleting\u{2026}"
                                    } else {
                                        "Delete Log File"
                                    },
                                )
                                .danger()
                                .sm()
                                .busy(deleting)
                                .disabled(deleting)
                                .on_click(click(cx, |_, cx| {
                                    crate::settings::confirm(
                                        cx,
                                        "Delete general application log",
                                        "Delete the general application log file?",
                                        "Delete",
                                        |s, cx| log_action(s, true, cx),
                                    )
                                })),
                            )
                        })
                        .when(s.ui.log_deleted, |d| {
                            d.child(
                                div()
                                    .text_size(text::SM)
                                    .tracking(0.06)
                                    .text_color(p.accent)
                                    .child("\u{2713} Deleted"),
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
                    .disabled(!dev)
                    // The developer window is S15; the action is the shell's ⌘D.
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(crate::settings::SettingsEvent::OpenDebug))),
            )
            // `.dev-win.is-off`: only the copy dims; the button is disabled.
            .dim_copy(!dev)
            .build(p)
            .into_any_element(),
        row("General application log")
            .desc("Captures high-level runtime events and errors.")
            .full()
            .control(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .w_full()
                    .child(log_body)
                    .children(s.ui.log_error.clone().map(|e| {
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S2)
                            .child(crate::settings::row::line(crate::ui::Tone::Danger, crate::settings::icons::ALERT, e, p))
                            .child(
                                Button::icon_only("log-error-dismiss", crate::settings::icons::X, "Dismiss error")
                                    .ghost()
                                    .sm()
                                    .on_click(click(cx, |s, cx| {
                                        s.ui.log_error = None;
                                        cx.notify();
                                    })),
                            )
                    })),
            )
            .build(p)
            .into_any_element(),
    ];
    section("Developer & Logs", None, Vec::new(), false, rows, p).into_any_element()
}
