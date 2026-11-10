//! The in-modal OAuth Connect (`McpOAuthConnectPanel` + `McpOAuthConnect`):
//! idle → waiting for the browser → connected, or "didn't go through". Two
//! doors: a row's Connect / Reconnect, or the catalog's OAuth preset (added and
//! persisted once, on Connect). app-core's `mcp_oauth_begin` opens the browser;
//! the stage follows its live state (`mcp_oauth_statuses`, refreshed on
//! `McpAuthorizationChanged`) plus this flow's attempt flags.

use capture_types::RecordingSettings;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, FocusHandle, Window};
use serde_json::Value;

use super::logic::{self, Stage, PRESETS};
use super::{close_picker, oauth_state, Step};
use crate::settings::autosave::Unit;
use crate::settings::{click, icons, Settings};
use crate::theme::{space, text, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::Button;

pub struct Flow {
    /// The connector whose live state drives the stage (the row's, or the
    /// preset's once added). `None` before a preset is added.
    pub id: Option<String>,
    /// The catalog preset (the add-on-Connect door).
    pub preset: Option<usize>,
    pub reconnect: bool,
    pub attempted: bool,
    /// We saw `authorizing` since the last attempt: a later fall-back is a denial.
    pub saw_authorizing: bool,
    /// `mcp_oauth_begin` failed: the browser never opened.
    pub has_error: bool,
    /// Adding the preset failed (nothing persisted).
    pub submit_error: Option<String>,
    /// The add + begin is in flight.
    pub busy: bool,
    /// The stage's one primary action (Connect / Cancel / Done / Try again).
    pub primary: FocusHandle,
}

fn start(s: &mut Settings, flow: Flow, window: &mut Window, cx: &mut Context<Settings>) {
    flow.primary.focus(window, cx);
    s.ui.mcp.step = Step::OAuth(flow);
    s.ui.mcp_open = None;
    s.ui.mcp_picker_open = true;
    cx.notify();
}

/// A row's Connect / Reconnect.
pub fn open_for(
    s: &mut Settings,
    id: &str,
    reconnect: bool,
    window: &mut Window,
    cx: &mut Context<Settings>,
) {
    let flow = Flow {
        id: Some(id.to_string()),
        preset: None,
        reconnect,
        attempted: false,
        saw_authorizing: false,
        has_error: false,
        submit_error: None,
        busy: false,
        primary: cx.focus_handle(),
    };
    start(s, flow, window, cx);
}

/// The catalog's OAuth preset: nothing is added until Connect.
pub fn open_preset(
    s: &mut Settings,
    preset: usize,
    window: &mut Window,
    cx: &mut Context<Settings>,
) {
    let flow = Flow {
        id: None,
        preset: Some(preset),
        reconnect: false,
        attempted: false,
        saw_authorizing: false,
        has_error: false,
        submit_error: None,
        busy: false,
        primary: cx.focus_handle(),
    };
    start(s, flow, window, cx);
}

fn flow_mut(s: &mut Settings) -> Option<&mut Flow> {
    match &mut s.ui.mcp.step {
        Step::OAuth(f) => Some(f),
        _ => None,
    }
}

/// The service name: the row's label (or id), else the preset's.
fn label(s: &Settings, f: &Flow) -> String {
    let server = f.id.as_ref().and_then(|id| {
        s.d()
            .ai_runtime
            .mcp_servers
            .into_iter()
            .find(|m| &m.id == id)
    });
    match (server, f.preset) {
        (Some(m), _) if !m.label.trim().is_empty() => m.label.trim().to_string(),
        (Some(m), _) => m.id,
        (None, Some(i)) => PRESETS[i].label.to_string(),
        (None, None) => String::new(),
    }
}

pub fn title(s: &Settings, f: &Flow) -> String {
    format!("Connect {}", label(s, f))
}

pub fn stage(s: &Settings, f: &Flow) -> Stage {
    let state = f.id.as_deref().and_then(|id| oauth_state(s, id));
    logic::oauth_stage(state, f.attempted, f.has_error, f.saw_authorizing)
}

/// What one Connect did, off the main thread.
struct Began {
    saved: Option<RecordingSettings>,
    add_error: Option<String>,
    begin: Result<(), String>,
    statuses: Option<Vec<Value>>,
}

/// Connect / Reconnect (`oauthConnectStart`).
fn connect(s: &mut Settings, cx: &mut Context<Settings>) {
    let Some(before) = s.draft.clone() else {
        return;
    };
    let Some(f) = flow_mut(s) else { return };
    if f.busy {
        return;
    }
    f.attempted = true;
    f.saw_authorizing = false;
    f.has_error = false;
    f.submit_error = None;
    f.busy = true;
    // The preset door adds the connector once, switched on (`presetToDraft`).
    let (id, next) = match (&f.id, f.preset) {
        (Some(id), _) => (id.clone(), None),
        (None, Some(i)) => {
            let mut next = before.clone();
            let m = logic::from_preset(&PRESETS[i], &next.ai_runtime.mcp_servers, true);
            let id = m.id.clone();
            next.ai_runtime.mcp_servers.insert(0, m);
            (id, Some(next))
        }
        (None, None) => return,
    };
    let backend = s.backend.clone();
    let job_id = id.clone();
    let job = cx.background_spawn(async move {
        let mut saved = None;
        if let Some(next) = next {
            // `flushAiRuntimeSave`: begin resolves the connector from saved settings.
            match backend.save_domain(Unit::AiRuntime, &next) {
                Ok(s) => saved = Some(s),
                Err(e) => {
                    return Began {
                        saved: None,
                        add_error: Some(e),
                        begin: Ok(()),
                        statuses: None,
                    }
                }
            }
        }
        let begin = backend.mcp_oauth_begin(&job_id);
        Began {
            saved,
            add_error: None,
            begin,
            statuses: backend.mcp_oauth_statuses(),
        }
    });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        let _ = this.update(cx, |s, cx| {
            if let Some(saved) = r.saved {
                let ai = saved.ai_runtime;
                s.adopt(move |d| d.ai_runtime = ai.clone());
            }
            if let Some(st) = r.statuses {
                s.data.mcp_oauth = st;
            }
            match &r.begin {
                Ok(()) => s.ui.mcp.oauth_errors.remove(&id),
                Err(e) => {
                    log::warn!("settings: MCP connector {id} couldn't start its sign-in");
                    s.ui.mcp.oauth_errors.insert(id.clone(), e.clone())
                }
            };
            let authorizing = oauth_state(s, &id) == Some("authorizing");
            if let Some(f) = flow_mut(s) {
                f.busy = false;
                if let Some(e) = r.add_error {
                    // Nothing was added: back to the resting Connect.
                    f.submit_error = Some(e);
                    f.attempted = false;
                } else {
                    f.id = Some(id.clone());
                    f.has_error = r.begin.is_err();
                    f.saw_authorizing |= authorizing;
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

fn line(t: impl Into<gpui_kit::SharedString>, p: &Palette) -> gpui_kit::Div {
    div()
        .max_w(px(360.))
        .text_size(text::SM)
        .text_color(p.text_subtle)
        .text_center()
        .child(t.into())
}

fn headline(t: impl Into<gpui_kit::SharedString>, p: &Palette) -> gpui_kit::Div {
    div()
        .text_size(text::MD)
        .font_weight(crate::theme::W_SEMIBOLD)
        .text_color(p.text_strong)
        .child(t.into())
}

/// The round stage glyph (`.oauth__glyph` / `__check` / `__x`).
fn glyph(
    path: &'static str,
    fg: gpui_kit::Hsla,
    bg: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
) -> gpui_kit::Div {
    div()
        .size(px(46.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .child(crate::ui::icon(path, px(22.)))
}

pub fn render(s: &Settings, f: &Flow, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let name = label(s, f);
    let server = f.id.as_ref().and_then(|id| {
        s.d()
            .ai_runtime
            .mcp_servers
            .into_iter()
            .find(|m| &m.id == id)
    });
    // The row door names the connector; the preset door shows its lede.
    let lede = match (f.preset, &server) {
        (Some(i), _) => PRESETS[i].lede.to_string(),
        (None, Some(_)) => {
            format!("Chat can use {name}'s tools once you approve access in your browser.")
        }
        (None, None) => String::new(),
    };
    let url = server
        .as_ref()
        .and_then(|m| m.url.clone())
        .or_else(|| f.preset.and_then(|i| PRESETS[i].url.map(str::to_string)))
        .unwrap_or_default();
    let path = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_string();
    let verb = if f.reconnect { "Reconnect" } else { "Connect" };
    let who = if name.trim().is_empty() {
        "access".to_string()
    } else {
        name.clone()
    };
    let primary =
        |id: &'static str, label: String| Button::new(id, label).primary().focus_handle(&f.primary);
    let stage = match stage(s, f) {
        Stage::Idle => div()
            .child(
                primary(
                    "mcp-oauth-connect",
                    if name.is_empty() {
                        format!("{verb} with OAuth")
                    } else {
                        format!("{verb} {name}")
                    },
                )
                .busy(f.busy)
                .on_click(click(cx, |s, cx| connect(s, cx))),
            )
            .child(line(
                "Opens your browser to approve. Nothing is pasted here \u{2014} only the returned token is kept, stored encrypted on this Mac.",
                p,
            )),
        Stage::Authorizing => div()
            .child(glyph(icons::EXTERNAL, p.accent, p.accent_bg, p.accent_border))
            .child(headline(format!("Approve {who} in your browser"), p))
            .child(line(
                "We opened your browser \u{2014} approve access there, then you'll come right back.",
                p,
            ))
            .child(
                div()
                    .id("mcp-oauth-waiting")
                    .role(gpui_kit::Role::Status)
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .text_size(text::SM)
                    .font_weight(crate::theme::W_SEMIBOLD)
                    .text_color(p.text_muted)
                    .child(div().size(px(8.)).rounded_full().bg(p.accent))
                    .child("WAITING\u{2026}"),
            )
            // The pending sign-in lapses on its own (~5 min, ADR 0051): Cancel
            // only dismisses, as Svelte's.
            .child(
                Button::new("mcp-oauth-cancel", "Cancel")
                    .ghost()
                    .sm()
                    .focus_handle(&f.primary)
                    .on_click(click(cx, |s, cx| close_picker(s, cx))),
            ),
        Stage::Authorized => div()
            .child(glyph(icons::CHECK, p.accent, p.accent_bg, p.accent_border))
            .child(headline("Connected", p))
            .child(
                primary("mcp-oauth-done", "Done".into())
                    .on_click(click(cx, |s, cx| {
                        close_picker(s, cx);
                        s.refresh(cx);
                    })),
            ),
        Stage::Denied => div()
            .child(glyph(icons::ALERT, p.danger_text, p.danger_bg, p.danger_border))
            .child(headline("Approval didn't go through", p))
            .child(line(
                "You cancelled or denied access in the browser. Nothing was stored.",
                p,
            ))
            .child(
                primary("mcp-oauth-retry", "Try again".into())
                    .on_click(click(cx, |s, cx| {
                        if let Some(f) = flow_mut(s) {
                            f.attempted = false;
                            f.saw_authorizing = false;
                        }
                        cx.notify();
                    })),
            ),
    };
    let stage = stage
        .flex()
        .flex_col()
        .items_center()
        .gap(space::S3)
        .pt(px(26.))
        .pb(space::S3)
        .px(space::S2);
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .p(space::S5)
        .when(!lede.is_empty(), |d| {
            d.child(
                div()
                    .text_size(text::BASE)
                    .text_color(p.text_muted)
                    .child(lede),
            )
        })
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(chip("HTTP", ChipKind::Neutral, p))
                .when(!path.is_empty(), |d| {
                    d.child(chip(path, ChipKind::Neutral, p))
                })
                .child(chip("OAuth", ChipKind::Accent, p)),
        )
        .child(stage)
        .children(f.submit_error.clone().map(|e| {
            div()
                .id("mcp-oauth-error")
                .role(gpui_kit::Role::Alert)
                .text_size(text::BASE)
                .text_color(p.danger_text)
                .child(e)
        }))
        .into_any_element()
}
