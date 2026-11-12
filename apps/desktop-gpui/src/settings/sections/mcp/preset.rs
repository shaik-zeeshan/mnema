//! Step 2 of the picker for a bearer or local preset (`McpConnectorPicker`'s
//! `connect` body + `McpAdvancedFields`): the lede, transport + path chips, the
//! Node line for a local preset, the ONE token field, the collapsed Advanced
//! overrides, and Back / Add connector. Add verifies before finishing (persist →
//! secret → list tools) and rolls back on failure. Edit mode (a preset row's
//! Configure) writes Advanced edits straight onto the connector, offers Remove,
//! and re-verifies on Save only when a new token was pasted.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Entity, Subscription, Window};

use super::form::{commit, hint, label, stack};
use super::logic::{self, Advanced, PRESETS};
use super::{close_picker, probe_node, Step, NODE_URL};
use crate::settings::{click, commands, icons, Settings};
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::input::TextInput;
use crate::ui::link::Link;
use crate::ui::Button;

pub struct Connect {
    pub preset: usize,
    /// The connector being edited (`None` = adding the preset).
    pub edit: Option<String>,
    /// Masked; dropped with the step, never copied into a draft.
    pub token: Entity<InputState>,
    pub adv_open: bool,
    pub name: Entity<InputState>,
    pub url: Entity<InputState>,
    pub command: Entity<InputState>,
    pub args: Entity<InputState>,
    pub error: Option<String>,
    pub connecting: bool,
    _subs: Vec<Subscription>,
}

fn connect_mut(s: &mut Settings) -> Option<&mut Connect> {
    match &mut s.ui.mcp.step {
        Step::Preset(c) => Some(c),
        _ => None,
    }
}

impl Connect {
    pub fn advanced(&self, cx: &gpui_kit::App) -> Advanced {
        let v = |e: &Entity<InputState>| e.read(cx).value().to_string();
        Advanced {
            name: v(&self.name),
            url: v(&self.url),
            command: v(&self.command),
            args: v(&self.args),
        }
    }
}

/// `syncAdvToDraft`: in edit mode an Advanced edit lands on the connector at once
/// (the ai_runtime autosave persists it), so Save without a token just closes.
fn write_through(s: &mut Settings, cx: &mut Context<Settings>) {
    let Some(c) = connect_mut(s) else { return };
    let Some(id) = c.edit.clone() else { return };
    let adv = c.advanced(cx);
    s.edit(
        move |d| {
            if let Some(m) = d.ai_runtime.mcp_servers.iter_mut().find(|m| m.id == id) {
                logic::write_advanced(&adv, m);
            }
        },
        cx,
    );
}

/// `selectPreset` (add) / `startEdit` (an existing preset connector).
pub fn open(
    s: &mut Settings,
    i: usize,
    edit: Option<&str>,
    window: &mut Window,
    cx: &mut Context<Settings>,
) {
    let p = &PRESETS[i];
    let server = edit.and_then(|id| {
        s.d()
            .ai_runtime
            .mcp_servers
            .into_iter()
            .find(|m| m.id == id)
    });
    if edit.is_some() && server.is_none() {
        return; // stale id
    }
    let (cmd, args) = p.command.unwrap_or_default();
    let seed = match &server {
        Some(m) => Advanced {
            name: m.label.clone(),
            url: m.url.clone().unwrap_or_default(),
            command: m.command.clone().unwrap_or_default(),
            args: m.args.join(" "),
        },
        None => Advanced {
            name: p.label.into(),
            url: p.url.unwrap_or_default().into(),
            command: cmd.into(),
            args: args.join(" "),
        },
    };
    let mut subs = Vec::new();
    let mut field = |value: &str, placeholder: &str, adv: bool| {
        let (v, ph) = (value.to_string(), placeholder.to_string());
        // Only the token field is blank-seeded, and it is the masked one.
        let st = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(!adv)
                .placeholder(ph)
                .default_value(v)
        });
        subs.push(
            cx.subscribe_in(&st, window, move |s, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    if adv {
                        write_through(s, cx);
                    }
                    cx.notify();
                }
            }),
        );
        st
    };
    let name = field(&seed.name, "", true);
    let url = field(&seed.url, "", true);
    let command = field(&seed.command, "", true);
    let args = field(&seed.args, "space-separated", true);
    let secret = p.secret.unwrap_or_default().to_lowercase();
    let placeholder = if edit.is_some() {
        format!("Unchanged \u{2014} paste a new {secret} to replace it")
    } else {
        format!("Paste your {secret}\u{2026}")
    };
    let token = field("", &placeholder, false);
    if p.secret.is_some() {
        token.update(cx, |st, cx| st.focus(window, cx));
    }
    s.ui.mcp.search = None;
    s.ui.mcp.step = Step::Preset(Connect {
        preset: i,
        edit: edit.map(str::to_string),
        token,
        adv_open: false,
        name,
        url,
        command,
        args,
        error: None,
        connecting: false,
        _subs: subs,
    });
    s.ui.mcp_picker_open = true;
    if !p.hosted() {
        probe_node(s, cx);
    }
    cx.notify();
}

pub fn title(s: &Settings, c: &Connect) -> String {
    match &c.edit {
        Some(id) => s
            .d()
            .ai_runtime
            .mcp_servers
            .iter()
            .find(|m| &m.id == id)
            .map(|m| m.label.trim().to_string())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| id.clone()),
        None => format!("Connect {}", PRESETS[c.preset].label),
    }
}

/// `addDisabled`: a preset with a secret needs one pasted before it's added.
fn add_disabled(c: &Connect, cx: &gpui_kit::App) -> bool {
    c.connecting
        || (c.edit.is_none()
            && PRESETS[c.preset].secret.is_some()
            && c.token.read(cx).value().trim().is_empty())
}

/// Add connector (`submit`) / Save (`saveEdit`).
fn submit(s: &mut Settings, window: &mut Window, cx: &mut Context<Settings>) {
    let Some(before) = s.draft.clone() else {
        return;
    };
    let node_missing = s.ui.mcp.node == Some(None);
    let Some(c) = connect_mut(s) else { return };
    if add_disabled(c, cx) {
        return;
    }
    let p = &PRESETS[c.preset];
    let secret = Some(commands::take_input(&c.token, window, cx)).filter(|v| !v.is_empty());
    let mut next = before.clone();
    let (id, verify, rollback) = match c.edit.clone() {
        None => {
            let mut m = logic::from_preset(p, &next.ai_runtime.mcp_servers, true);
            logic::apply_overrides(p, &c.advanced(cx), node_missing, &mut m);
            let id = m.id.clone();
            // Prepended: the new row lands at the top (Svelte).
            next.ai_runtime.mcp_servers.insert(0, m);
            // A local preset added while Node is missing can't list tools: it lands off.
            (id, p.hosted() || !node_missing, true)
        }
        Some(id) => {
            if secret.is_none() {
                // The Advanced edits already rode the autosave.
                close_picker(s, cx);
                return;
            }
            let Some(m) = next.ai_runtime.mcp_servers.iter().find(|m| m.id == id) else {
                return;
            };
            // A disabled connector (or stdio while Node is missing) can't list tools.
            let verify = m.enabled && (p.hosted() || !node_missing);
            (id, verify, false)
        }
    };
    c.connecting = true;
    c.error = None;
    let backend = s.backend.clone();
    let has_secret = secret.is_some();
    // Detached on purpose: a rollback must finish even if Settings goes away.
    cx.spawn(async move |this, cx| {
        let (saved, outcome) = commit(
            cx,
            backend,
            before,
            next,
            id.clone(),
            secret,
            verify,
            rollback,
        )
        .await;
        let _ = this.update(cx, |s, cx| {
            if let Some(saved) = saved {
                let ai = saved.ai_runtime;
                s.adopt(move |d| d.ai_runtime = ai.clone());
            }
            match outcome {
                Ok(count) => {
                    if has_secret {
                        s.data.mcp_secrets.insert(id.clone());
                    }
                    if let Some(n) = count {
                        s.ui.mcp.counts.insert(id.clone(), n);
                    }
                    log::info!("settings: MCP connector {id} saved");
                    close_picker(s, cx);
                    if rollback {
                        super::flash(s, &id, cx);
                    }
                    s.refresh(cx);
                }
                Err(e) => {
                    if rollback {
                        s.data.mcp_secrets.remove(&id);
                    }
                    if let Some(c) = connect_mut(s) {
                        c.connecting = false;
                        c.error = Some(e);
                    }
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

fn error_line(id: &'static str, e: &str, p: &Palette) -> AnyElement {
    div()
        .id(id)
        .role(gpui_kit::Role::Alert)
        .text_size(text::SM)
        .text_color(p.danger_text)
        .child(e.to_string())
        .into_any_element()
}

/// The Node line under a local preset's chips.
fn node_line(s: &Settings, p: &Palette) -> AnyElement {
    let base = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(4.))
        .text_size(text::SM);
    match &s.ui.mcp.node {
        None => base
            .text_color(p.text_muted)
            .child("Checking for Node\u{2026}"),
        Some(None) => base
            .text_color(p.warn)
            .font_weight(theme::W_SEMIBOLD)
            .child("Runs locally and needs Node, which wasn\u{2019}t found.")
            .child(Link::new(
                "mcp-connect-node",
                "Install it from nodejs.org",
                NODE_URL,
            )),
        Some(Some(v)) => base
            .text_color(p.accent)
            .child("Runs locally via Node \u{2014} found")
            .child(div().font_family(FONT_MONO).child(format!("node {v}")))
            .child(crate::ui::icon(icons::CHECK, crate::theme::size::ICON_XS)),
    }
    .into_any_element()
}

/// `McpAdvancedFields`: the collapsed name + URL, or name + command + args.
fn advanced(c: &Connect, hosted: bool, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let field = |id: &'static str, l: &'static str, st: &Entity<InputState>| {
        stack(vec![
            label(l, p).into_any_element(),
            TextInput::new(id, l, st).into_any_element(),
        ])
        .into_any_element()
    };
    let body: Vec<AnyElement> = if !c.adv_open {
        Vec::new()
    } else if hosted {
        vec![
            field("mcp-adv-name", "Name", &c.name),
            field("mcp-adv-url", "URL", &c.url),
        ]
    } else {
        vec![
            field("mcp-adv-name", "Name", &c.name),
            field("mcp-adv-command", "Command", &c.command),
            field("mcp-adv-args", "Arguments", &c.args),
        ]
    };
    let open = c.adv_open;
    div()
        .flex()
        .flex_col()
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .overflow_hidden()
        .child(
            div()
                .id("mcp-adv-toggle")
                .role(gpui_kit::Role::Button)
                .aria_label("Advanced")
                .aria_expanded(open)
                .flex()
                .items_center()
                .justify_between()
                .py(px(10.))
                .px(space::S3)
                .text_size(text::XS)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_muted)
                .cursor_pointer()
                .hover(|st| st.text_color(p.text_strong))
                .on_click(click(cx, |s, cx| {
                    if let Some(c) = connect_mut(s) {
                        c.adv_open = !c.adv_open;
                    }
                    cx.notify();
                }))
                .child("ADVANCED")
                .child(crate::ui::icon(
                    if open {
                        "icons/chevron-down.svg"
                    } else {
                        "icons/chevron-right.svg"
                    },
                    crate::theme::size::ICON_XS,
                ))
                .test_support(),
        )
        .when(open, |d| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S3)
                    .pt(px(2.))
                    .px(space::S3)
                    .pb(px(14.))
                    .border_t_1()
                    .border_color(p.border)
                    .children(body),
            )
        })
        .into_any_element()
}

pub fn render(s: &Settings, c: &Connect, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let pr = &PRESETS[c.preset];
    let adv = c.advanced(cx);
    let path = if pr.hosted() {
        let u = adv.url.trim();
        u.strip_prefix("https://")
            .or_else(|| u.strip_prefix("http://"))
            .unwrap_or(u)
            .to_string()
    } else {
        format!("{} {}", adv.command.trim(), adv.args.trim())
            .trim()
            .to_string()
    };
    let mut body: Vec<AnyElement> = vec![
        div()
            .text_size(text::SM)
            .line_height(gpui_kit::relative(1.5))
            .text_color(p.text_muted)
            .child(pr.lede)
            .into_any_element(),
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .child(chip(
                if pr.hosted() { "HTTP" } else { "STDIO" },
                ChipKind::Neutral,
                p,
            ))
            .when(!path.is_empty(), |d| {
                d.child(chip(path, ChipKind::Neutral, p))
            })
            .into_any_element(),
    ];
    if !pr.hosted() {
        body.push(node_line(s, p));
    }
    if let Some(secret) = pr.secret {
        let saved = c
            .edit
            .as_ref()
            .is_some_and(|id| s.data.mcp_secrets.contains(id));
        body.push(
            stack(vec![
                label(secret, p).into_any_element(),
                TextInput::new("mcp-connect-token", secret, &c.token)
                    .invalid(c.error.is_some())
                    .disabled(c.connecting)
                    .into_any_element(),
            ])
            .children(
                c.error
                    .as_deref()
                    .map(|e| error_line("mcp-connect-error", e, p)),
            )
            .children(
                pr.help
                    .map(|url| Link::new("mcp-connect-help", "Create one", url).quiet()),
            )
            .into_any_element(),
        );
        body.push(if saved {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(text::SM)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.accent)
                .child(crate::ui::icon(icons::CHECK, crate::theme::size::ICON_XS))
                .child("secret saved")
                .into_any_element()
        } else {
            hint(
                "Stored encrypted on this Mac \u{2014} never in Mnema's settings.",
                p,
            )
            .into_any_element()
        });
    }
    body.push(advanced(c, pr.hosted(), p, cx));
    if let (Some(e), None) = (&c.error, pr.secret) {
        body.push(error_line("mcp-connect-error", e, p));
    }
    let primary = if c.connecting {
        "Connecting\u{2026}"
    } else if c.edit.is_some() {
        "Save"
    } else {
        "Add connector"
    };
    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(space::S3)
        .pt(space::S1)
        .child(match c.edit.clone() {
            Some(id) => Button::new("mcp-connect-remove", "Remove")
                .sm()
                .danger()
                .disabled(c.connecting)
                .on_click(click(cx, move |s, cx| {
                    if let Some(m) = s.d().ai_runtime.mcp_servers.iter().find(|m| m.id == id) {
                        super::remove(m, cx);
                    }
                })),
            None => Button::new("mcp-connect-back", "Back")
                .sm()
                .disabled(c.connecting)
                .on_click(commands::click_w(cx, super::back_to_catalog)),
        })
        .child(
            Button::new("mcp-connect-submit", primary)
                .busy(c.connecting)
                .disabled(add_disabled(c, cx))
                .on_click(commands::click_w(cx, submit)),
        );
    div()
        .id("mcp-connect")
        .flex()
        .flex_col()
        .gap(px(14.))
        .px(space::S5)
        .pb(space::S5)
        .children(body)
        .child(footer)
        .into_any_element()
}
