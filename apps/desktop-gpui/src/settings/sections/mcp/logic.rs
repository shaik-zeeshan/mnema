//! The MCP connector rules, pure: `mcp-presets.ts`, `newMcpServerId`
//! (`ai-providers.ts`), `mcp-tool-curation.ts`, `mcp-oauth-stage.ts` and
//! `mcp-connector-row.ts`. Tests mirror their `.test.ts` cases (`logic_tests.rs`).

use capture_types::{McpAuthMode, McpEnvVar, McpServerConfig, McpTransport};

/// `MCP_PRESETS`: id, label, tagline, step-2 lede, hosted url or local command + args, OAuth.
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub tagline: &'static str,
    pub lede: &'static str,
    pub url: Option<&'static str>,
    pub command: Option<(&'static str, &'static [&'static str])>,
    pub oauth: bool,
    /// `helpUrl`: where to create the token the secret field asks for.
    pub help: Option<&'static str>,
    /// `secretLabel`: the ONE token field's label (`None` = no secret).
    pub secret: Option<&'static str>,
    /// The tile's brand mark (an asset path).
    pub icon: &'static str,
}

pub const PRESETS: [Preset; 5] = [
    Preset {
        id: "github",
        label: "GitHub",
        tagline: "Issues, pull requests, code search.",
        lede: "Chat can search code, read issues, and open pull requests in your repos.",
        url: Some("https://api.githubcopilot.com/mcp/"),
        command: None,
        oauth: false,
        help: Some("https://github.com/settings/personal-access-tokens"),
        secret: Some("Personal access token"),
        icon: crate::assets::BRAND_GITHUB,
    },
    Preset {
        id: "linear",
        label: "Linear",
        tagline: "Issues, projects, cycles.",
        lede: "Chat can look up and update issues, projects, and cycles.",
        url: Some("https://mcp.linear.app/mcp"),
        command: None,
        oauth: false,
        help: Some("https://linear.app/settings/account/security"),
        secret: Some("API key"),
        icon: crate::assets::BRAND_LINEAR,
    },
    Preset {
        id: "stripe",
        label: "Stripe",
        tagline: "Customers, payments, invoices.",
        lede: "Chat can look up customers, payments, and invoices.",
        url: Some("https://mcp.stripe.com"),
        command: None,
        oauth: false,
        help: Some("https://dashboard.stripe.com/apikeys"),
        secret: Some("Restricted API key"),
        icon: crate::assets::BRAND_STRIPE,
    },
    Preset {
        id: "notion",
        label: "Notion",
        tagline: "Pages and databases \u{2014} sign in with your browser.",
        lede: "Chat can read and edit your pages and databases.",
        url: Some("https://mcp.notion.com/mcp"),
        command: None,
        oauth: true,
        help: None,
        secret: None,
        icon: crate::assets::BRAND_NOTION,
    },
    Preset {
        id: "filesystem",
        label: "Filesystem",
        tagline: "Read and write files in folders you choose.",
        lede: "Chat can read and write files in the folders you pick.",
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
        help: None,
        secret: None,
        icon: "icons/folder.svg",
    },
];

impl Preset {
    /// `kind === "hosted"`: an HTTP endpoint (else a local command).
    pub fn hosted(&self) -> bool {
        self.url.is_some()
    }
}

/// The catalog search (`matchesQuery`): label + tagline, case-blind.
pub fn matches(p: &Preset, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || format!("{} {}", p.label, p.tagline)
            .to_lowercase()
            .contains(&q)
}

/// "✓ added": a connector already carries this preset's slug or a suffixed one.
pub fn added(p: &Preset, servers: &[McpServerConfig]) -> bool {
    let slug = new_server_id(p.label, &[]);
    servers.iter().any(|m| {
        m.id == slug
            || m.id
                .strip_prefix(&slug)
                .and_then(|r| r.strip_prefix('-'))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The Advanced panel's text (`PresetAdvancedInput` minus Node).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Advanced {
    pub name: String,
    pub url: String,
    pub command: String,
    pub args: String,
}

/// `presetOverrides`: only the fields the user changed land on `m` (a local
/// preset added while Node is missing lands switched off).
pub fn apply_overrides(p: &Preset, adv: &Advanced, node_missing: bool, m: &mut McpServerConfig) {
    if !p.hosted() && node_missing {
        m.enabled = false;
    }
    let name = adv.name.trim();
    if !name.is_empty() && name != p.label {
        m.label = name.to_string();
    }
    if p.hosted() {
        let url = adv.url.trim();
        if !url.is_empty() && Some(url) != p.url {
            m.url = Some(url.to_string());
        }
    } else {
        let (cmd, args) = p.command.unwrap_or_default();
        let command = adv.command.trim();
        if !command.is_empty() && command != cmd {
            m.command = Some(command.to_string());
        }
        if adv.args.trim() != args.join(" ") {
            m.args = adv.args.split_whitespace().map(str::to_string).collect();
        }
    }
}

/// `syncAdvToDraft`: edit mode writes the Advanced text straight onto the connector.
pub fn write_advanced(adv: &Advanced, m: &mut McpServerConfig) {
    m.label = adv.name.clone();
    if m.transport == McpTransport::Http {
        m.url = Some(adv.url.trim().to_string());
    } else {
        m.command = Some(adv.command.trim().to_string());
        m.args = adv.args.split_whitespace().map(str::to_string).collect();
    }
}

/// `presetForServer`: the id is the preset id or a slugger variant
/// (`github`, `github-2`…) AND the transport matches. `None` = a custom connector.
pub fn preset_for(server: &McpServerConfig) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| {
        let transport = if p.url.is_some() {
            McpTransport::Http
        } else {
            McpTransport::Stdio
        };
        let suffixed = server
            .id
            .strip_prefix(p.id)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        server.transport == transport && (server.id == p.id || suffixed)
    })
}

/// `newMcpServerId`: a `[a-z0-9-]` slug of the label (`connector` when it has
/// none), suffixed on collision. The charset is load-bearing (`mcp__<id>__<tool>`).
pub fn new_server_id(label: &str, existing: &[String]) -> String {
    let mut base = String::new();
    for c in label.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "connector" } else { base };
    let taken = |id: &str| existing.iter().any(|e| e == id);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or_default()
}

/// `presetDisplayLabel`: the preset label, or the first free `Label (n)`.
pub fn preset_display_label(label: &str, existing: &[String]) -> String {
    let taken = |l: &str| existing.iter().any(|e| e.trim() == l);
    if !taken(label) {
        return label.to_string();
    }
    (2..)
        .map(|n| format!("{label} ({n})"))
        .find(|l| !taken(l))
        .unwrap_or_default()
}

/// `presetToDraft`: a fresh id and display label; `enabled` is the caller's.
pub fn from_preset(
    preset: &Preset,
    existing: &[McpServerConfig],
    enabled: bool,
) -> McpServerConfig {
    let ids: Vec<String> = existing.iter().map(|m| m.id.clone()).collect();
    let labels: Vec<String> = existing.iter().map(|m| m.label.clone()).collect();
    McpServerConfig {
        id: new_server_id(preset.label, &ids),
        label: preset_display_label(preset.label, &labels),
        enabled,
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

/// The Custom form's model (`McpConnectorCustomForm`), as typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Custom {
    pub label: String,
    pub stdio: bool,
    pub oauth: bool,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub url: String,
    pub secret_env: String,
}

impl Custom {
    /// An existing connector, for edit mode.
    pub fn from_server(s: &McpServerConfig) -> Self {
        Self {
            label: s.label.clone(),
            stdio: s.transport == McpTransport::Stdio,
            oauth: s.auth_mode == McpAuthMode::OAuth,
            command: s.command.clone().unwrap_or_default(),
            args: s.args.clone(),
            env: s
                .env
                .iter()
                .map(|e| (e.name.clone(), e.value.clone()))
                .collect(),
            url: s.url.clone().unwrap_or_default(),
            secret_env: s.secret_env_name.clone().unwrap_or_default(),
        }
    }

    /// `addDisabled` for Custom: http needs a URL, stdio a command.
    pub fn missing_target(&self) -> bool {
        if self.stdio {
            self.command.trim().is_empty()
        } else {
            self.url.trim().is_empty()
        }
    }

    /// OAuth http: no pasted secret, authorized in the browser instead.
    pub fn oauth_http(&self) -> bool {
        !self.stdio && self.oauth
    }

    /// `buildCustomMcpDraft`: only the chosen transport's fields survive; blank
    /// args and env rows drop. `id`, `enabled` and the curation are the caller's.
    pub fn to_config(
        &self,
        id: String,
        enabled: bool,
        enabled_tools: Option<Vec<String>>,
    ) -> McpServerConfig {
        let some = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let stdio = self.stdio;
        McpServerConfig {
            id,
            label: self.label.trim().to_string(),
            enabled,
            transport: if stdio {
                McpTransport::Stdio
            } else {
                McpTransport::Http
            },
            auth_mode: if !stdio && self.oauth {
                McpAuthMode::OAuth
            } else {
                McpAuthMode::Bearer
            },
            command: if stdio { some(&self.command) } else { None },
            args: if stdio {
                self.args
                    .iter()
                    .filter(|a| !a.trim().is_empty())
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
            env: if stdio {
                self.env
                    .iter()
                    .filter(|(n, _)| !n.trim().is_empty())
                    .map(|(name, value)| McpEnvVar {
                        name: name.clone(),
                        value: value.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            url: if stdio { None } else { some(&self.url) },
            secret_env_name: if stdio { some(&self.secret_env) } else { None },
            enabled_tools,
        }
    }

    /// A brand-new connector: slugged id off the trimmed label, switched on.
    pub fn build(&self, existing_ids: &[String]) -> McpServerConfig {
        self.to_config(new_server_id(self.label.trim(), existing_ids), true, None)
    }
}

/// `MCP_DEFAULT_TOOL_CAP`: lockstep with app-core's `ask_ai::mcp` (private there).
pub const DEFAULT_TOOL_CAP: usize = 32;

/// `activeToolNames`: `None` = the first [`DEFAULT_TOOL_CAP`] in server order;
/// a list = exactly those that still exist (new tools stay off: the drift rule).
pub fn active_tool_names(all: &[String], enabled: Option<&[String]>) -> Vec<String> {
    match enabled {
        None => all.iter().take(DEFAULT_TOOL_CAP).cloned().collect(),
        Some(list) => all.iter().filter(|n| list.contains(n)).cloned().collect(),
    }
}

/// `toggleTool`: the first toggle turns an uncurated connector curated; answers
/// the new list in server order, drifted names dropped (may be empty).
pub fn toggle_tool(
    all: &[String],
    enabled: Option<&[String]>,
    name: &str,
    on: bool,
) -> Vec<String> {
    let mut chosen = active_tool_names(all, enabled);
    chosen.retain(|n| n != name);
    if on {
        chosen.push(name.to_string());
    }
    all.iter().filter(|n| chosen.contains(n)).cloned().collect()
}

/// `activeToolCount` for "N tools · M active".
pub fn active_tool_count(tools: usize, enabled: Option<&[String]>) -> usize {
    enabled.map_or(tools.min(DEFAULT_TOOL_CAP), <[String]>::len)
}

/// `McpOAuthStage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Authorizing,
    Authorized,
    Denied,
}

/// `deriveMcpOAuthStage`: the live state plus this session's attempt flags.
pub fn oauth_stage(
    state: Option<&str>,
    attempted: bool,
    has_error: bool,
    saw_authorizing: bool,
) -> Stage {
    match state {
        Some("authorized") => Stage::Authorized,
        Some("authorizing") => Stage::Authorizing,
        _ if !attempted => Stage::Idle,
        _ if has_error || saw_authorizing => Stage::Denied,
        // Begin is still in flight: the optimistic wait.
        _ => Stage::Authorizing,
    }
}

/// `McpRowBadge`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Secret,
    Authorized,
    AuthorizedMuted,
    NotConnected,
    Authorizing,
    Reconnect,
    None,
}

/// `McpRowView`: the badge, the extra buttons (the switch is always there), dimming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Actions {
    pub connect: bool,
    pub reconnect: bool,
    pub disconnect: bool,
    pub configure: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowView {
    pub badge: Badge,
    pub actions: Actions,
    pub dimmed: bool,
}

/// `deriveMcpConnectorRow`. `oauth` = the draft's auth mode is OAuth.
pub fn row_view(
    oauth: bool,
    enabled: bool,
    has_secret: bool,
    oauth_state: Option<&str>,
) -> RowView {
    let none = Actions::default();
    let configure = Actions {
        configure: true,
        ..none
    };
    if !oauth {
        return RowView {
            badge: if has_secret {
                Badge::Secret
            } else {
                Badge::None
            },
            actions: configure,
            dimmed: !enabled,
        };
    }
    let (badge, actions, dimmed) = match oauth_state.unwrap_or("none") {
        "authorized" if enabled => (
            Badge::Authorized,
            Actions {
                disconnect: true,
                configure: true,
                ..none
            },
            false,
        ),
        "authorized" => (Badge::AuthorizedMuted, configure, true),
        "authorizing" => (Badge::Authorizing, none, false),
        "reconnect" => (
            Badge::Reconnect,
            Actions {
                reconnect: true,
                disconnect: true,
                ..none
            },
            false,
        ),
        _ => (
            Badge::NotConnected,
            Actions {
                connect: true,
                ..none
            },
            false,
        ),
    };
    RowView {
        badge,
        actions,
        dimmed,
    }
}

/// `subLine`: the OAuth states that carry words instead of a tool count.
pub fn sub_line(badge: Badge, enabled: bool) -> Option<&'static str> {
    Some(match badge {
        Badge::AuthorizedMuted => "authorized \u{b7} token kept \u{2014} turn on to use it again",
        Badge::NotConnected if enabled => {
            "Enabled, but not authorized yet \u{2014} approve in your browser to start using it."
        }
        Badge::NotConnected => {
            "Not authorized yet \u{2014} turn it on and Connect to start using it."
        }
        Badge::Authorizing => {
            "Waiting for your browser\u{2026} approve access in the tab that opened."
        }
        Badge::Reconnect => {
            "Token expired \u{2014} chat reports it as unavailable until you reconnect."
        }
        _ => return None,
    })
}

/// `hintFor`: the description line when no state sub-line applies.
pub fn hint(badge: Badge, stdio: bool) -> &'static str {
    match badge {
        Badge::Secret => "Secret stored encrypted",
        Badge::Authorized => "Authorized",
        _ if stdio => "Runs on this Mac",
        _ => "No secret saved",
    }
}

/// `showToolCount`: a usable-ish row offers its tool list.
pub fn shows_tools(badge: Badge) -> bool {
    matches!(badge, Badge::Secret | Badge::None | Badge::Authorized)
}

/// The row's tool caption: "N tools · M active", "M active", or "see tools".
pub fn tool_caption(count: Option<usize>, enabled: Option<&[String]>) -> String {
    match (count, enabled) {
        (Some(n), e) => format!("{n} tools \u{b7} {} active", active_tool_count(n, e)),
        (None, Some(list)) => format!("{} active", list.len()),
        (None, None) => "see tools".into(),
    }
}

#[cfg(test)]
#[path = "logic_tests.rs"]
mod tests;
