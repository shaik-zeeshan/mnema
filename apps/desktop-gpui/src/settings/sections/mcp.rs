//! Intelligence › MCP connectors (`McpConnectors.svelte` + `mcp-connector-row.ts`,
//! the picker `McpConnectorPicker` / `McpConnectorCatalog`): one row per
//! connector with its auth state and tool caption, and the Add connector modal.
//! The modal's steps live beside this file: `form` (Custom), `oauth` (the
//! in-modal Connect stages), `tools` (per-tool curation); the rules in `logic`.

mod form;
mod logic;
mod oauth;
mod tools;

#[cfg(test)]
#[path = "mcp/flow_tests.rs"]
mod flow_tests;

use std::cell::Cell;
use std::collections::HashMap;

use capture_types::{McpAuthMode, McpServerConfig, McpTransport};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString, Window};

use crate::settings::row::{row, section, state_word};
use crate::settings::{click, commands, icons, live, on, Settings};
use crate::theme::{self, space, text, Palette};
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
}

#[derive(Default)]
pub enum Step {
    #[default]
    Catalog,
    Form(form::Form),
    OAuth(oauth::Flow),
}

/// The preset a connector was added from: its "Create one" link.
fn help_url(server: &McpServerConfig) -> Option<&'static str> {
    logic::preset_for(server).and_then(|p| p.help)
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
    cx.notify();
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
        Badge::Authorizing => Some(("authorizing\u{2026}", None)),
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
            let custom = logic::preset_for(server).is_none();
            r.control(
                Button::new(
                    SharedString::from(format!("mcp-configure-{}", server.id)),
                    "Configure",
                )
                .ghost()
                .on_click(commands::click_w(cx, move |s, window, cx| {
                    if custom {
                        // A custom connector edits in its full form (edit mode).
                        form::open(s, Some(&cid), window, cx);
                    } else {
                        s.ui.mcp_open =
                            (s.ui.mcp_open.as_deref() != Some(cid.as_str())).then(|| cid.clone());
                        cx.notify();
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
        .when(s.ui.mcp_open.as_deref() == Some(server.id.as_str()), |r| {
            r.below(configure(s, server, oauth, secret, cx))
        })
        .build(p)
        .when(view.dimmed, |d| d.opacity(theme::DISABLED_OPACITY))
        .into_any_element()
}

/// A preset connector's inline Configure: its secret (stored in the vault,
/// never in settings) and Remove.
// shortcut: presets edit inline (secret + Remove), not in Svelte's step-2 body with
// the Advanced name/URL/command overrides; port `McpAdvancedFields` if presets need editing.
fn configure(
    s: &Settings,
    server: &McpServerConfig,
    oauth: bool,
    secret: bool,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let (save_id, clear_id) = (server.id.clone(), server.id.clone());
    let rm = server.clone();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(space::S2)
        .w_full()
        .pt(space::S2)
        .when(!oauth, |d| {
            d.child(
                div()
                    .flex_1()
                    .min_w(px(200.))
                    .child(crate::ui::input::TextInput::new(
                        "mcp-secret",
                        "Connector secret",
                        &s.inputs.mcp_secret,
                    )),
            )
            .children(help_url(server).map(|url| {
                // `.field-help`: small and muted; the kit arrow stands for its "→".
                crate::ui::link::Link::new("mcp-secret-help", "Create one", url).quiet()
            }))
            .child(
                Button::new("mcp-secret-save", "Save secret")
                    .sm()
                    .on_click(commands::click_w(cx, move |s, window, cx| {
                        let input = s.inputs.mcp_secret.clone();
                        let value = commands::take_input(&input, window, cx);
                        let id = save_id.clone();
                        if !value.is_empty() {
                            s.run_command("MCP connector", cx, move |core| {
                                live::mcp_set_secret(core, &id, &value)
                            });
                        }
                    })),
            )
            .when(secret, |d| {
                d.child(
                    Button::new("mcp-secret-clear", "Clear secret")
                        .sm()
                        .ghost()
                        .on_click(click(cx, move |s, cx| {
                            let id = clear_id.clone();
                            s.run_command("MCP connector", cx, move |core| {
                                live::mcp_clear_secret(core, &id)
                            })
                        })),
                )
            })
        })
        .child(
            Button::new("mcp-remove", "Remove")
                .sm()
                .ghost_danger()
                .on_click(click(cx, move |_, cx| remove(&rm, cx))),
        )
        .into_any_element()
}

/// `removeMcpServer`: the confirm, then the draft row and its vault secret go.
pub(super) fn remove(server: &McpServerConfig, cx: &mut Context<Settings>) {
    let id = server.id.clone();
    let label = if server.label.trim().is_empty() {
        server.id.clone()
    } else {
        server.label.trim().to_string()
    };
    crate::settings::confirm(
        cx,
        "Remove this connector?",
        &format!(
            "Removing \u{201c}{label}\u{201d} deletes its saved secret from this Mac right away and stops offering its tools to chat."
        ),
        "Remove & Delete Secret",
        move |s, cx| {
            s.ui.mcp_open = None;
            if matches!(&s.ui.mcp.step, Step::Form(f) if f.edit.as_deref() == Some(id.as_str())) {
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

/// A catalog tile (McpConnectorCatalog `.tile`).
fn tile(
    id: &str,
    label: &'static str,
    tagline: &'static str,
    p: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(SharedString::from(format!("mcp-preset-{id}")))
        .role(gpui_kit::Role::Button)
        .aria_label(label)
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(space::S3)
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .cursor_pointer()
        .hover(|st| st.bg(p.surface_hover).border_color(p.border_hover))
        .child(
            div()
                .text_size(text::MD)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(label),
        )
        .child(
            div()
                .text_size(text::BASE)
                .text_color(p.text_muted)
                .child(tagline),
        )
}

/// The catalog: one tile per preset, then Custom. A bearer or local preset adds
/// at once, switched off; the OAuth one opens its Connect stages.
fn catalog(p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let tiles = PRESETS.iter().enumerate().map(|(i, preset)| {
        tile(preset.id, preset.label, preset.tagline, p).on_click(commands::click_w(
            cx,
            move |s, window, cx| {
                if PRESETS[i].oauth {
                    oauth::open_preset(s, i, window, cx);
                    return;
                }
                close_picker(s, cx);
                s.edit(
                    |d| {
                        let m = logic::from_preset(&PRESETS[i], &d.ai_runtime.mcp_servers, false);
                        d.ai_runtime.mcp_servers.push(m)
                    },
                    cx,
                );
            },
        ))
    });
    let custom = tile(
        "custom",
        "Custom",
        "Any other MCP server \u{2014} fill in every field yourself.",
        p,
    )
    .border_dashed()
    .on_click(commands::click_w(cx, |s, window, cx| {
        form::open(s, None, window, cx)
    }));
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .p(space::S5)
        .child(
            div().text_color(p.text_muted).child(
                "Pick a service. You can Connect or add its secret after it\u{2019}s added.",
            ),
        )
        .child(div().grid().grid_cols(2).gap(space::S2).children(tiles))
        .child(custom)
        .into_any_element()
}

/// Modals over the window: the tool list, else the Add / Edit / Connect picker.
pub fn picker(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Option<AnyElement> {
    if let Some(list) = &s.ui.mcp.tools {
        return Some(tools::modal(s, list, p, cx));
    }
    if !s.ui.mcp_picker_open {
        return None;
    }
    let (title, body, busy): (String, AnyElement, bool) = match &s.ui.mcp.step {
        Step::Catalog => ("Add a connector".into(), catalog(p, cx), false),
        Step::Form(f) => (form::title(s, f), form::render(s, f, p, cx), f.connecting),
        Step::OAuth(f) => (oauth::title(s, f), oauth::render(s, f, p, cx), f.busy),
    };
    let this = cx.entity().downgrade();
    Some(
        Modal::new("mcp-picker", title)
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
        _ => {}
    }
}
