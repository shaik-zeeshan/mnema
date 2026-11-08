//! Intelligence › MCP connectors (`McpConnectors.svelte` + `mcp-connector-row.ts`,
//! the picker `McpConnectorPicker` / `McpConnectorCatalog`, `mcp-presets.ts`):
//! one row per connector with its auth state, and the Add connector catalog.

use capture_types::{McpAuthMode, McpServerConfig, McpTransport};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use crate::settings::row::{row, section, state_word};
use crate::settings::{click, icons, on, Settings};
use crate::theme::{self, radius, space, text, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::dialog::Modal;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::switch::Switch;
use crate::ui::{Button, Tone};

/// `MCP_PRESETS`: id, label, tagline, hosted url or local command + args, OAuth.
struct Preset {
    id: &'static str,
    label: &'static str,
    tagline: &'static str,
    url: Option<&'static str>,
    command: Option<(&'static str, &'static [&'static str])>,
    oauth: bool,
}

const PRESETS: [Preset; 5] = [
    Preset {
        id: "github",
        label: "GitHub",
        tagline: "Issues, pull requests, code search.",
        url: Some("https://api.githubcopilot.com/mcp/"),
        command: None,
        oauth: false,
    },
    Preset {
        id: "linear",
        label: "Linear",
        tagline: "Issues, projects, cycles.",
        url: Some("https://mcp.linear.app/mcp"),
        command: None,
        oauth: false,
    },
    Preset {
        id: "stripe",
        label: "Stripe",
        tagline: "Customers, payments, invoices.",
        url: Some("https://mcp.stripe.com"),
        command: None,
        oauth: false,
    },
    Preset {
        id: "notion",
        label: "Notion",
        tagline: "Pages and databases \u{2014} sign in with your browser.",
        url: Some("https://mcp.notion.com/mcp"),
        command: None,
        oauth: true,
    },
    Preset {
        id: "filesystem",
        label: "Filesystem",
        tagline: "Read and write files in folders you choose.",
        url: None,
        command: Some((
            "npx",
            &[
                "-y",
                "@modelcontextprotocol/server-filesystem",
                "~/Documents",
            ],
        )),
        oauth: false,
    },
];

/// `rowTag`: http rows show their auth mode, stdio its transport.
fn tag(s: &McpServerConfig) -> &'static str {
    match (&s.transport, &s.auth_mode) {
        (McpTransport::Http, McpAuthMode::OAuth) => "OAuth",
        (McpTransport::Http, _) => "Bearer",
        (McpTransport::Stdio, _) => "stdio",
    }
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
    // shortcut: OAuth states (mcp_oauth_statuses) and the browser connect flow are
    // src-tauri until S3; an OAuth connector reads "not connected" until then.
    let (state, hint): (Option<(&str, Option<Tone>)>, String) = if oauth {
        let sub = if server.enabled {
            "Enabled, but not authorized yet \u{2014} approve in your browser to start using it."
        } else {
            "Not authorized yet \u{2014} turn it on and Connect to start using it."
        };
        (Some(("not connected", None)), sub.into())
    } else if secret {
        (None, "Secret stored encrypted".into())
    } else if server.transport == McpTransport::Stdio {
        (None, "Runs on this Mac".into())
    } else {
        (None, "No secret saved".into())
    };
    let count = server
        .enabled_tools
        .as_ref()
        .map_or("see tools".to_string(), |t| format!("{} active", t.len()));
    let id = server.id.clone();
    let below = div()
        .flex()
        .gap(px(4.))
        .text_size(text::BASE)
        .text_color(p.text_muted)
        .child(hint)
        // shortcut: the tool-list modal (mcp_list_server_tools) is src-tauri until S3.
        .when(!oauth, |d| {
            d.child("\u{b7}").child(
                div()
                    .text_color(if server.enabled { p.text } else { p.text_faint })
                    .child(count),
            )
        });
    row(name.clone())
        .extra(chip(tag(server), ChipKind::Neutral, p))
        .when(state.is_some(), |r| {
            let (w, t) = state.unwrap();
            r.control(state_word(w, t, p))
        })
        .when(oauth, |r| {
            r.control(Button::new(
                SharedString::from(format!("mcp-connect-{}", server.id)),
                "Connect",
            ))
        })
        .when(!oauth, |r| {
            r.control(
                Button::new(
                    SharedString::from(format!("mcp-configure-{}", server.id)),
                    "Configure",
                )
                .ghost(),
            )
        })
        .control(
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
            })),
        )
        .below(below)
        .build(p)
        .when(!server.enabled, |d| d.opacity(theme::DISABLED_OPACITY))
        .into_any_element()
}

fn open_picker(
    cx: &mut Context<Settings>,
) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static {
    click(cx, |s, cx| {
        s.ui.mcp_picker_open = true;
        cx.notify();
    })
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let servers = s.d().ai_runtime.mcp_servers;
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

/// The Add connector catalog: one tile per preset; a pick adds it switched off.
pub fn picker(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Option<AnyElement> {
    if !s.ui.mcp_picker_open {
        return None;
    }
    let tiles = PRESETS.iter().enumerate().map(|(i, preset)| {
        div()
            .id(SharedString::from(format!("mcp-preset-{}", preset.id)))
            .role(gpui_kit::Role::Button)
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(space::S3)
            .rounded(radius::LG)
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .cursor_pointer()
            .hover(|st| st.bg(p.wash_strong))
            .child(
                div()
                    .text_size(text::MD)
                    .font_weight(theme::W_SEMIBOLD)
                    .text_color(p.text_strong)
                    .child(preset.label),
            )
            .child(
                div()
                    .text_size(text::BASE)
                    .text_color(p.text_muted)
                    .child(preset.tagline),
            )
            .on_click(click(cx, move |s, cx| {
                s.ui.mcp_picker_open = false;
                s.edit(
                    |d| {
                        d.ai_runtime
                            .mcp_servers
                            .push(from_preset(&PRESETS[i], &d.ai_runtime.mcp_servers))
                    },
                    cx,
                );
            }))
    });
    let this = cx.entity().downgrade();
    Some(
        Modal::new("mcp-picker", "Add a connector")
            .lead(div().text_color(p.text_muted).child(
                "Pick a service. You can Connect or add its secret after it\u{2019}s added.",
            ))
            // shortcut: the custom connector form (McpConnectorCustomForm) and the in-modal OAuth
            // stages are not built yet; presets only.
            .child(div().grid().grid_cols(2).gap(space::S2).children(tiles))
            .on_close(move |_, cx| {
                let _ = this.update(cx, |s, cx| {
                    s.ui.mcp_picker_open = false;
                    cx.notify();
                });
            })
            .into_any_element(),
    )
}

/// `connectorFromPreset`: a fresh id (`github`, `github-2`…), switched off.
fn from_preset(preset: &Preset, existing: &[McpServerConfig]) -> McpServerConfig {
    let id = if existing.iter().any(|m| m.id == preset.id) {
        (2..)
            .map(|n| format!("{}-{n}", preset.id))
            .find(|id| !existing.iter().any(|m| &m.id == id))
            .unwrap_or_default()
    } else {
        preset.id.to_string()
    };
    McpServerConfig {
        id,
        label: preset.label.into(),
        enabled: false,
        transport: if preset.url.is_some() {
            McpTransport::Http
        } else {
            McpTransport::Stdio
        },
        auth_mode: if preset.oauth {
            McpAuthMode::OAuth
        } else {
            McpAuthMode::Bearer
        },
        command: preset.command.map(|(c, _)| c.to_string()),
        args: preset.command.map_or(Vec::new(), |(_, a)| {
            a.iter().map(|s| s.to_string()).collect()
        }),
        env: Vec::new(),
        url: preset.url.map(str::to_string),
        secret_env_name: None,
        enabled_tools: None,
    }
}
