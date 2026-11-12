//! Intelligence › MCP connectors (`McpConnectors.svelte` + `mcp-connector-row.ts`,
//! the picker `McpConnectorPicker` / `McpConnectorCatalog`): one row per
//! connector with its auth state and tool caption, and the Add connector modal.
//! The modal's steps live beside this file: `catalog` (step 1), `preset` (a
//! preset's Connect / Edit body), `form` (Custom), `oauth` (the in-modal
//! Connect stages), `tools` (per-tool curation); the rules in `logic`.

mod catalog;
mod form;
mod logic;
mod oauth;
mod preset;
mod tools;

#[cfg(test)]
#[path = "mcp/flow_tests.rs"]
mod flow_tests;
#[cfg(test)]
#[path = "mcp/preset_tests.rs"]
mod preset_tests;

use std::cell::Cell;
use std::collections::HashMap;

use capture_types::{McpAuthMode, McpServerConfig, McpTransport};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Entity, SharedString, Subscription, Window};
use gpui_kit::{AnimationExt as _, TestSupportExt as _};

use crate::settings::row::{row, section, state_word};
use crate::settings::{click, commands, icons, live, on, Settings};
use crate::theme::{space, text, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::dialog::Modal;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::switch::Switch;
use crate::ui::{Button, Tone};
use logic::{Badge, PRESETS};

/// `NODE_URL`: where the "needs Node" hint sends people.
const NODE_URL: &str = "https://nodejs.org";

/// The MCP section's view state (inside `Settings::ui`).
#[derive(Default)]
pub struct McpUi {
    /// What the open picker modal shows (`ui.mcp_picker_open` opens it).
    pub step: Step,
    /// The tool-list modal, when open.
    pub tools: Option<tools::ToolList>,
    /// Discovered tool counts by connector id (from a successful listing).
    pub counts: HashMap<String, usize>,
    /// `mcp_check_node`: `None` = not probed, `Some(None)` = Node is missing.
    pub node: Option<Option<String>>,
    /// The probe is in flight or done (one per Settings visit, as Svelte).
    node_probed: Cell<bool>,
    /// The last `mcp_oauth_begin` failure per connector, shown on its row.
    pub oauth_errors: HashMap<String, String>,
    /// The catalog's search field, alive while the catalog shows.
    pub search: Option<(Entity<InputState>, Subscription)>,
    /// The connector just added: its row flashes once (`mcp-row--new`).
    pub flash: Option<String>,
}

/// `onPickerAdded`: flash the new row, then forget it once the wash has faded.
pub(super) fn flash(s: &mut Settings, id: &str, cx: &mut Context<Settings>) {
    s.ui.mcp.flash = Some(id.to_string());
    let id = id.to_string();
    cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(crate::theme::motion::FLASH)
            .await;
        let _ = this.update(cx, |s, cx| {
            if s.ui.mcp.flash.as_deref() == Some(id.as_str()) {
                s.ui.mcp.flash = None;
                cx.notify();
            }
        });
    })
    .detach();
}

#[derive(Default)]
pub enum Step {
    #[default]
    Catalog,
    Preset(preset::Connect),
    Form(form::Form),
    OAuth(oauth::Flow),
}

/// `rowTag`: http rows show their auth mode, stdio its transport.
fn tag(s: &McpServerConfig) -> &'static str {
    match (&s.transport, &s.auth_mode) {
        (McpTransport::Http, McpAuthMode::OAuth) => "OAuth",
        (McpTransport::Http, _) => "Bearer",
        (McpTransport::Stdio, _) => "stdio",
    }
}

fn oauth_state<'a>(s: &'a Settings, id: &str) -> Option<&'a str> {
    s.data
        .mcp_oauth
        .iter()
        .find(|o| o["id"].as_str() == Some(id))
        .and_then(|o| o["state"].as_str())
}

/// Close the picker modal; the next open starts on the catalog.
fn close_picker(s: &mut Settings, cx: &mut Context<Settings>) {
    s.ui.mcp_picker_open = false;
    s.ui.mcp.step = Step::Catalog;
    s.ui.mcp.search = None;
    cx.notify();
}

/// `backToCatalog` (Back, or Esc on an add step): not while connecting.
fn back_to_catalog(s: &mut Settings, _: &mut Window, cx: &mut Context<Settings>) {
    let connecting = match &s.ui.mcp.step {
        Step::Preset(c) => c.connecting,
        Step::Form(f) => f.connecting,
        _ => false,
    };
    if !connecting {
        s.ui.mcp.step = Step::Catalog;
        cx.notify();
    }
}

/// The open step is busy (a connect or save in flight): close stands down.
fn busy(step: &Step) -> bool {
    match step {
        Step::Catalog => false,
        Step::Preset(c) => c.connecting,
        Step::Form(f) => f.connecting,
        Step::OAuth(f) => f.busy,
    }
}

/// One Node probe per Settings visit, once a stdio connector shows
/// (`McpConnectors.svelte`'s effect); render can't mutate, so it latches a Cell.
fn probe_node(s: &Settings, cx: &mut Context<Settings>) {
    if s.ui.mcp.node_probed.replace(true) {
        return;
    }
    let backend = s.backend.clone();
    let job = cx.background_spawn(async move { backend.mcp_check_node() });
    cx.spawn(async move |this, cx| {
        let found = job.await;
        let _ = this.update(cx, |s, cx| {
            if let Ok(v) = found {
                s.ui.mcp.node = Some(v);
                cx.notify();
            }
        });
    })
    .detach();
}

fn connector_row(
    s: &Settings,
    server: &McpServerConfig,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let name = if server.label.trim().is_empty() {
        server.id.clone()
    } else {
        server.label.clone()
    };
    let oauth = server.auth_mode == McpAuthMode::OAuth && server.transport == McpTransport::Http;
    let secret = s.data.mcp_secrets.contains(&server.id);
    let view = logic::row_view(oauth, server.enabled, secret, oauth_state(s, &server.id));
    let stdio = server.transport == McpTransport::Stdio;
    let needs_node = stdio && s.ui.mcp.node == Some(None);
    let state: Option<(&str, Option<Tone>)> = match view.badge {
        Badge::Authorizing => None,
        Badge::Reconnect => Some(("needs reconnect", Some(Tone::Warn))),
        Badge::NotConnected => Some(("not connected", None)),
        _ => None,
    };
    let hint_line = div()
        .flex()
        .flex_wrap()
        .gap(px(4.))
        .text_size(text::BASE)
        .text_color(p.text_muted)
        .map(|d| {
            if needs_node {
                d.child("Runs on this Mac and needs Node, which wasn\u{2019}t found.")
                    .child(crate::ui::link::Link::new(
                        SharedString::from(format!("mcp-node-{}", server.id)),
                        "Install it from nodejs.org",
                        NODE_URL,
                    ))
            } else {
                d.child(
                    logic::sub_line(view.badge, server.enabled)
                        .unwrap_or_else(|| logic::hint(view.badge, stdio)),
                )
            }
        })
        .when(logic::shows_tools(view.badge), |d| {
            let caption = logic::tool_caption(
                s.ui.mcp.counts.get(&server.id).copied(),
                server.enabled_tools.as_deref(),
            );
            let id = server.id.clone();
            let enabled = server.enabled;
            d.child("\u{b7}").child(
                div()
                    .id(SharedString::from(format!("mcp-tools-{}", server.id)))
                    .role(gpui_kit::Role::Button)
                    .aria_label(if enabled {
                        "See and curate this connector's tools"
                    } else {
                        "Enable this connector to list its tools."
                    })
                    .text_color(if enabled { p.text } else { p.text_faint })
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|st| st.text_color(p.accent))
                            .on_click(commands::click_w(cx, move |s, window, cx| {
                                tools::open(s, &id, window, cx)
                            }))
                    })
                    .child(caption),
            )
        });
    let below = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(hint_line)
        .children(s.ui.mcp.oauth_errors.get(&server.id).map(|e| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(text::BASE)
                .text_color(p.danger_text)
                .child(crate::ui::icon(icons::ALERT, crate::theme::size::ICON_SM))
                .child(e.clone())
        }));
    let verb_button = |label: &'static str, reconnect: bool| {
        let id = server.id.clone();
        Button::new(
            SharedString::from(format!("mcp-{}-{}", label.to_lowercase(), server.id)),
            label,
        )
        .on_click(commands::click_w(cx, move |s, window, cx| {
            oauth::open_for(s, &id, reconnect, window, cx)
        }))
    };
    row(name.clone())
        .extra(chip(tag(server), ChipKind::Neutral, p))
        .when(needs_node, |r| {
            r.extra(chip("needs Node", ChipKind::Warn, p))
        })
        .when(state.is_some(), |r| {
            let (w, t) = state.unwrap_or_default();
            r.control(state_word(w, t, p))
        })
        .when(view.badge == Badge::Authorizing, |r| {
            // `.settings-state` with its small `.mx-spin`.
            r.control(
                div()
                    .id(SharedString::from(format!("mcp-authorizing-{}", server.id)))
                    .role(gpui_kit::Role::Status)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(crate::ui::progress::spinner(crate::theme::size::SPIN_SM))
                    .child(state_word("authorizing\u{2026}", None, p))
                    .test_support(),
            )
        })
        .when(view.actions.connect, |r| {
            r.control(verb_button("Connect", false))
        })
        .when(view.actions.reconnect, |r| {
            r.control(verb_button("Reconnect", true))
        })
        .when(view.actions.disconnect, |r| {
            let cid = server.id.clone();
            r.control(
                Button::new(
                    SharedString::from(format!("mcp-disconnect-{}", server.id)),
                    "Disconnect",
                )
                .ghost()
                .on_click(click(cx, move |s, cx| {
                    let id = cid.clone();
                    s.run_command("MCP connector", cx, move |core| {
                        live::mcp_oauth_disconnect(core, id)
                    })
                })),
            )
        })
        .when(view.actions.configure, |r| {
            let cid = server.id.clone();
            let preset = logic::preset_for(server).map(|p| p.id);
            r.control(
                Button::new(
                    SharedString::from(format!("mcp-configure-{}", server.id)),
                    "Configure",
                )
                .ghost()
                .on_click(commands::click_w(cx, move |s, window, cx| {
                    // `openConfigure`: a preset edits in its Connect body, a
                    // custom connector in its full form.
                    match preset.and_then(|id| PRESETS.iter().position(|p| p.id == id)) {
                        Some(i) => preset::open(s, i, Some(&cid), window, cx),
                        None => form::open(s, Some(&cid), window, cx),
                    }
                })),
            )
        })
        .control({
            let id = server.id.clone();
            Switch::new(
                SharedString::from(format!("mcp-on-{}", server.id)),
                format!("Enable {name}"),
                server.enabled,
            )
            .on_toggle(on(cx, move |s, v: bool, cx| {
                let id = id.clone();
                s.edit(
                    move |d| {
                        if let Some(m) = d.ai_runtime.mcp_servers.iter_mut().find(|m| m.id == id) {
                            m.enabled = v;
                        }
                    },
                    cx,
                )
            }))
        })
        .below(below)
        .dim_copy(view.dimmed)
        .build(p)
        .map(|row| {
            if s.ui.mcp.flash.as_deref() != Some(server.id.as_str()) {
                return row.into_any_element();
            }
            let wash = p.accent;
            row.with_animation(
                SharedString::from(format!("mcp-flash-{}", server.id)),
                gpui_kit::Animation::new(crate::theme::motion::FLASH)
                    .with_easing(crate::theme::motion::quart),
                move |el, t| el.bg(crate::ui::alpha(wash, 0.1 * (1. - t))),
            )
            .into_any_element()
        })
}

/// `removeMcpServer`: the confirm, then the draft row and its vault secret go.
pub(super) fn remove(server: &McpServerConfig, cx: &mut Context<Settings>) {
    let id = server.id.clone();
    let label = if server.label.trim().is_empty() {
        server.id.clone()
    } else {
        server.label.trim().to_string()
    };
    crate::settings::commands::confirm_as(
        cx,
        "Remove this connector?",
        &format!(
            "Removing \u{201c}{label}\u{201d} deletes its saved secret from this Mac right away and stops offering its tools to chat."
        ),
        "Remove & Delete Secret",
        "Keep Connector",
        move |s, cx| {
            let editing = match &s.ui.mcp.step {
                Step::Form(f) => f.edit.as_deref(),
                Step::Preset(c) => c.edit.as_deref(),
                _ => None,
            };
            if editing == Some(id.as_str()) {
                close_picker(s, cx);
            }
            s.data.mcp_secrets.remove(&id);
            let drop_id = id.clone();
            s.edit(
                move |d| d.ai_runtime.mcp_servers.retain(|m| m.id != drop_id),
                cx,
            );
            let backend = s.backend.clone();
            let vault_id = id.clone();
            cx.background_spawn(async move {
                if backend.mcp_clear_secret(&vault_id).is_err() {
                    log::warn!("settings: couldn't delete the secret of MCP connector {vault_id}");
                }
            })
            .detach();
        },
    );
}

fn open_picker(
    cx: &mut Context<Settings>,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static {
    click(cx, |s, cx| {
        s.ui.mcp_picker_open = true;
        s.ui.mcp.step = Step::Catalog;
        cx.notify();
    })
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let servers = s.d().ai_runtime.mcp_servers;
    if servers.iter().any(|m| m.transport == McpTransport::Stdio) {
        probe_node(s, cx);
    }
    let rows: Vec<AnyElement> = if servers.is_empty() {
        vec![div()
            .py(space::S3)
            .child(empty(
                EmptyLayout::Compact,
                Tone::Neutral,
                icons::PLUG,
                "No connectors yet",
                "Add one \u{2014} GitHub, Notion, a folder on this Mac \u{2014} then Connect or enable it.",
                vec![Button::new("mcp-add-empty", "Add connector\u{2026}").sm().icon(icons::PLUS).on_click(open_picker(cx)).into_any_element()],
                p,
            ))
            .into_any_element()]
    } else {
        let mut rows: Vec<AnyElement> =
            servers.iter().map(|m| connector_row(s, m, p, cx)).collect();
        rows.push(
            row("Add a connector")
                .desc("Secrets are stored encrypted on this Mac, never in settings; removing a connector deletes its secret.")
                .control(Button::new("mcp-add", "Add connector\u{2026}").icon(icons::PLUS).on_click(open_picker(cx)))
                .build(p)
                .into_any_element(),
        );
        rows
    };
    section(
        "MCP connectors",
        Some("Tools chat can use. A connector works only when it\u{2019}s enabled and authorized."),
        Vec::new(),
        false,
        rows,
        p,
    )
    .into_any_element()
}

/// Modals over the window: the tool list, else the Add / Edit / Connect picker.
/// The catalog's search field is made here, where the window is at hand.
pub fn picker(
    s: &mut Settings,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> Option<AnyElement> {
    if let Some(list) = &s.ui.mcp.tools {
        return Some(tools::modal(s, list, p, cx));
    }
    if !s.ui.mcp_picker_open {
        return None;
    }
    if matches!(s.ui.mcp.step, Step::Catalog) {
        catalog::ensure_search(s, window, cx);
    } else {
        s.ui.mcp.search = None;
    }
    let s = &*s;
    // `eyebrow` / `title` of the picker header.
    let (eyebrow, title, body): (&str, String, AnyElement) = match &s.ui.mcp.step {
        Step::Catalog => (
            "Add connector",
            "Pick a service".into(),
            catalog::render(s, p, cx),
        ),
        Step::Preset(c) => (
            if c.edit.is_some() {
                "Edit connector"
            } else {
                "Add connector"
            },
            preset::title(s, c),
            preset::render(s, c, p, cx),
        ),
        Step::Form(f) => (
            if f.edit.is_some() {
                "Edit connector"
            } else {
                "Add connector"
            },
            form::title(s, f),
            form::render(s, f, p, cx),
        ),
        Step::OAuth(f) => (
            match (&f.preset, f.reconnect) {
                (Some(_), _) => "Add connector",
                (None, true) => "Reconnect",
                (None, false) => "Connect",
            },
            oauth::title(s, f),
            oauth::render(s, f, p, cx),
        ),
    };
    let busy = busy(&s.ui.mcp.step);
    let this = cx.entity().downgrade();
    Some(
        Modal::new("mcp-picker", title)
            .eyebrow(eyebrow)
            .max_w(px(560.))
            .child(body)
            .on_close(move |_, cx| {
                let _ = this.update(cx, |s, cx| {
                    // `requestClose`: not while a connect or save is in flight.
                    if !busy {
                        close_picker(s, cx);
                    }
                });
            })
            .into_any_element(),
    )
}

/// Esc: close the tool list, else the picker; an add step (preset or Custom)
/// steps back to the catalog instead. Nothing while a connect or save is in
/// flight, as the close button. True when something was open.
pub(crate) fn escape(s: &mut Settings, cx: &mut Context<Settings>) -> bool {
    if s.ui.mcp.tools.take().is_some() {
        s.jobs.remove("mcp-tools");
        cx.notify();
        return true;
    }
    if !s.ui.mcp_picker_open {
        return false;
    }
    if busy(&s.ui.mcp.step) {
        return true;
    }
    let adding = match &s.ui.mcp.step {
        Step::Preset(c) => c.edit.is_none(),
        Step::Form(f) => f.edit.is_none(),
        _ => false,
    };
    if adding {
        s.ui.mcp.step = Step::Catalog;
        cx.notify();
    } else {
        close_picker(s, cx);
    }
    true
}

/// Goldens: open one of the MCP modal states over Settings.
#[cfg(feature = "fixtures")]
pub(crate) fn fixture_state(
    s: &mut Settings,
    state: &str,
    window: &mut Window,
    cx: &mut Context<Settings>,
) {
    match state {
        "form" => form::open(s, None, window, cx),
        "tools" => {
            let id = s.d().ai_runtime.mcp_servers.first().map(|m| m.id.clone());
            if let Some(id) = id {
                tools::open(s, &id, window, cx);
            }
        }
        "oauth" => {
            let i = PRESETS.iter().position(|p| p.oauth).unwrap_or(0);
            oauth::open_preset(s, i, window, cx);
        }
        "connect" => preset::open(s, 0, None, window, cx),
        _ => {}
    }
}
