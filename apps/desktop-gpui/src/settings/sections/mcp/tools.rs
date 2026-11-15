//! The tool list (`McpToolListModal`): connects the connector on demand, lists
//! its tools with a switch each, and writes the choice to the connector's
//! `enabledTools` in the draft (the ai_runtime autosave persists it). `None` =
//! the default offer (first 32); the first toggle makes the list explicit; a
//! curated list keeps tools that newly appear on the server off.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, FocusHandle, SharedString, Window};

use super::logic;
use crate::settings::{click, icons, on, Settings};
use crate::theme::{space, text, Palette};
use crate::ui::dialog::Modal;
use crate::ui::switch::Switch;
use crate::ui::Button;

pub struct ToolList {
    pub id: String,
    pub loading: bool,
    pub error: Option<String>,
    /// `(name, description)` in server order.
    pub tools: Vec<(String, Option<String>)>,
    /// Focus sits in the modal so Esc reaches it.
    pub focus: FocusHandle,
}

/// Open the list and load it fresh (each open lists again, as Svelte).
pub fn open(s: &mut Settings, id: &str, window: &mut Window, cx: &mut Context<Settings>) {
    let focus = cx.focus_handle();
    focus.focus(window, cx);
    s.ui.mcp.tools = Some(ToolList {
        id: id.to_string(),
        loading: true,
        error: None,
        tools: Vec::new(),
        focus,
    });
    load(s, cx);
}

fn load(s: &mut Settings, cx: &mut Context<Settings>) {
    let Some(list) = s.ui.mcp.tools.as_mut() else {
        return;
    };
    list.loading = true;
    list.error = None;
    let id = list.id.clone();
    let job = s.backend.mcp_tools(&id).spawn(cx);
    // Another list (or a retry) replaces this one, cancelling it.
    let task = cx.spawn(async move |this, cx| {
        let listed = job.await;
        let _ = this.update(cx, |s, cx| {
            let Some(list) = s.ui.mcp.tools.as_mut().filter(|l| l.id == id) else {
                return; // closed, or another connector's list since
            };
            list.loading = false;
            match listed {
                Ok(tools) => {
                    s.ui.mcp.counts.insert(id, tools.len());
                    list.tools = tools;
                }
                Err(e) => {
                    log::warn!("settings: couldn't list the tools of MCP connector {id}");
                    list.error = Some(e);
                    list.tools.clear();
                }
            }
            cx.notify();
        });
    });
    s.jobs.insert("mcp-tools".into(), task);
    cx.notify();
}

/// Write a curation onto the draft connector (`curate`).
fn curate(s: &mut Settings, id: &str, enabled: Option<Vec<String>>, cx: &mut Context<Settings>) {
    let id = id.to_string();
    s.edit(
        move |d| {
            if let Some(m) = d.ai_runtime.mcp_servers.iter_mut().find(|m| m.id == id) {
                m.enabled_tools = enabled;
            }
        },
        cx,
    );
}

fn status(p: &Palette) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .text_size(text::MD)
        .text_color(p.text_muted)
}

pub fn modal(s: &Settings, list: &ToolList, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let server = s
        .d()
        .ai_runtime
        .mcp_servers
        .into_iter()
        .find(|m| m.id == list.id);
    let name = server
        .as_ref()
        .map(|m| m.label.trim().to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| list.id.clone());
    let enabled = server.as_ref().and_then(|m| m.enabled_tools.clone());
    let all: Vec<String> = list.tools.iter().map(|(n, _)| n.clone()).collect();
    let checked = logic::active_tool_names(&all, enabled.as_deref());
    let body = if list.loading {
        status(p)
            .id("mcp-tools-status")
            .role(gpui_kit::Role::Status)
            .child(crate::ui::icon(icons::LOADER, crate::theme::size::ICON_SM))
            // A long connector name wraps instead of running past the modal.
            .child(
                div()
                    .min_w_0()
                    .child(format!("Connecting to {name}\u{2026}")),
            )
            .into_any_element()
    } else if let Some(e) = &list.error {
        div()
            .id("mcp-tools-error")
            .role(gpui_kit::Role::Alert)
            .flex()
            .flex_col()
            .items_start()
            .gap(space::S2)
            .child(
                div()
                    .text_size(text::MD)
                    .text_color(p.danger_text)
                    .child(e.clone()),
            )
            .child(
                Button::new("mcp-tools-retry", "Retry")
                    .ghost()
                    .sm()
                    .on_click(click(cx, |s, cx| load(s, cx))),
            )
            .into_any_element()
    } else if list.tools.is_empty() {
        status(p)
            .child("This connector exposes no tools.")
            .into_any_element()
    } else {
        let (a, b, c) = (list.id.clone(), list.id.clone(), list.id.clone());
        let every = all.clone();
        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space::S2)
            .child(
                div()
                    .text_size(text::BASE)
                    .text_color(p.text_muted)
                    .child(format!(
                        "{} tools \u{b7} {} active",
                        list.tools.len(),
                        checked.len()
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(space::S1)
                    .child(
                        Button::new("mcp-tools-all", "Select all")
                            .ghost()
                            .sm()
                            .on_click(click(cx, move |s, cx| {
                                curate(s, &a, Some(every.clone()), cx)
                            })),
                    )
                    .child(
                        Button::new("mcp-tools-none", "Select none")
                            .ghost()
                            .sm()
                            .on_click(click(cx, move |s, cx| curate(s, &b, Some(Vec::new()), cx))),
                    )
                    .child(
                        Button::new("mcp-tools-reset", "Reset to default")
                            .ghost()
                            .sm()
                            .on_click(click(cx, move |s, cx| curate(s, &c, None, cx))),
                    ),
            );
        let rows = list.tools.iter().enumerate().map(|(i, (tool, desc))| {
            let on_now = checked.contains(tool);
            let (id, name, every) = (list.id.clone(), tool.clone(), all.clone());
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(space::S3)
                .py(space::S2)
                .when(i > 0, |d| d.border_t_1().border_color(p.hairline))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(text::MD)
                                .text_color(p.text_strong)
                                .child(tool.clone()),
                        )
                        .children(desc.clone().map(|d| {
                            div()
                                .text_size(text::BASE)
                                .text_color(p.text_muted)
                                .child(d)
                        })),
                )
                .child(
                    Switch::new(
                        SharedString::from(format!("mcp-tool-{i}")),
                        tool.clone(),
                        on_now,
                    )
                    .on_toggle(on(cx, move |s, v: bool, cx| {
                        let current = s
                            .d()
                            .ai_runtime
                            .mcp_servers
                            .into_iter()
                            .find(|m| m.id == id)
                            .and_then(|m| m.enabled_tools);
                        let next = logic::toggle_tool(&every, current.as_deref(), &name, v);
                        curate(s, &id, Some(next), cx)
                    })),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .child(toolbar)
            .child(crate::ui::scroll::area(
                div()
                    .id("mcp-tools-list")
                    .flex()
                    .flex_col()
                    .max_h(px(380.))
                    .overflow_y_scroll()
                    .children(rows),
            ))
            .into_any_element()
    };
    let this = cx.entity().downgrade();
    Modal::new("mcp-tools", format!("{name} \u{b7} tools"))
        .eyebrow("MCP connector")
        .max_w(px(560.))
        .child(
            div()
                .track_focus(&list.focus)
                .px(space::S5)
                .pb(space::S5)
                .child(body),
        )
        .on_close(move |_, cx| {
            let _ = this.update(cx, |s, cx| {
                s.ui.mcp.tools = None;
                s.jobs.remove("mcp-tools");
                cx.notify();
            });
        })
        .into_any_element()
}
