//! The Custom connector form (`McpConnectorCustomForm` + the picker's
//! `submit` / `saveEdit`): name, transport, its command + args + env or URL +
//! authorization, and the one optional secret. Add verifies before finishing
//! (persist → secret → list tools) and rolls back on failure; edit saves the
//! fields, then re-verifies only when a new secret was pasted.

use std::sync::Arc;

use capture_types::RecordingSettings;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, AsyncApp, Context, Entity, SharedString, Subscription, Window,
};

use super::logic::Custom;
use super::{close_picker, Step};
use crate::settings::autosave::Unit;
use crate::settings::data::Backend;
use crate::settings::{click, commands, icons, Settings};
use crate::theme::{space, text, Palette};
use crate::ui::input::TextInput;
use crate::ui::segmented::Segmented;
use crate::ui::Button;

pub struct Form {
    /// The connector being edited (`None` = adding a new one).
    pub edit: Option<String>,
    pub stdio: bool,
    pub oauth: bool,
    pub name: Entity<InputState>,
    pub command: Entity<InputState>,
    pub url: Entity<InputState>,
    pub secret_env: Entity<InputState>,
    /// Masked; dropped with the form, never copied into a draft.
    pub secret: Entity<InputState>,
    pub args: Vec<Entity<InputState>>,
    pub env: Vec<(Entity<InputState>, Entity<InputState>)>,
    pub error: Option<String>,
    pub connecting: bool,
    _subs: Vec<Subscription>,
}

fn field(
    window: &mut Window,
    cx: &mut Context<Settings>,
    subs: &mut Vec<Subscription>,
    placeholder: &str,
    value: &str,
) -> Entity<InputState> {
    let (ph, v) = (placeholder.to_string(), value.to_string());
    let state = cx.new(|cx| InputState::new(window, cx).placeholder(ph).default_value(v));
    // The Add button's validity and the URL warning follow every keystroke;
    // in edit mode the keystroke also lands on the connector.
    subs.push(
        cx.subscribe_in(&state, window, |s, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change) {
                write_through(s, cx);
                cx.notify();
            }
        }),
    );
    state
}

/// Edit mode binds the live draft (Svelte passes it as the form's model): each
/// field edit lands on the connector at once and rides the ai_runtime autosave.
/// Only a pasted secret waits for Save.
fn write_through(s: &mut Settings, cx: &mut Context<Settings>) {
    let Some(f) = form_mut(s) else { return };
    let Some(id) = f.edit.clone() else { return };
    let model = f.model(cx);
    s.edit(
        move |d| {
            if let Some(m) = d.ai_runtime.mcp_servers.iter_mut().find(|m| m.id == id) {
                *m = model.to_config(id.clone(), m.enabled, m.enabled_tools.clone());
            }
        },
        cx,
    );
}

/// Open the form: blank (add, HTTP first as Svelte), or one custom connector (edit).
pub fn open(s: &mut Settings, edit: Option<&str>, window: &mut Window, cx: &mut Context<Settings>) {
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
    let m = server.as_ref().map(Custom::from_server).unwrap_or_default();
    let mut subs = Vec::new();
    let name = field(window, cx, &mut subs, "e.g. GitHub", &m.label);
    let command = field(window, cx, &mut subs, "npx", &m.command);
    let url = field(window, cx, &mut subs, "https://mcp.example.com/mcp", &m.url);
    let secret_env = field(window, cx, &mut subs, "e.g. GITHUB_TOKEN", &m.secret_env);
    let placeholder = if edit.is_some() {
        "Unchanged \u{2014} paste a new secret to replace it"
    } else {
        "Token, if the server needs one\u{2026}"
    };
    let secret = cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(placeholder)
    });
    let args = m
        .args
        .iter()
        .map(|a| field(window, cx, &mut subs, "argument", a))
        .collect();
    let env = m
        .env
        .iter()
        .map(|(n, v)| {
            (
                field(window, cx, &mut subs, "NAME", n),
                field(window, cx, &mut subs, "value", v),
            )
        })
        .collect();
    name.update(cx, |st, cx| st.focus(window, cx));
    s.ui.mcp.step = Step::Form(Form {
        edit: edit.map(str::to_string),
        stdio: m.stdio,
        oauth: m.oauth,
        name,
        command,
        url,
        secret_env,
        secret,
        args,
        env,
        error: None,
        connecting: false,
        _subs: subs,
    });
    s.ui.mcp_picker_open = true;
    cx.notify();
}

fn form_mut(s: &mut Settings) -> Option<&mut Form> {
    match &mut s.ui.mcp.step {
        Step::Form(f) => Some(f),
        _ => None,
    }
}

impl Form {
    /// The typed model (never the secret).
    pub fn model(&self, cx: &gpui_kit::App) -> Custom {
        let v = |e: &Entity<InputState>| e.read(cx).value().to_string();
        Custom {
            label: v(&self.name),
            stdio: self.stdio,
            oauth: self.oauth,
            command: v(&self.command),
            args: self.args.iter().map(v).collect(),
            env: self.env.iter().map(|(n, x)| (v(n), v(x))).collect(),
            url: v(&self.url),
            secret_env: v(&self.secret_env),
        }
    }
}

pub fn title(s: &Settings, f: &Form) -> String {
    match &f.edit {
        Some(id) => s
            .d()
            .ai_runtime
            .mcp_servers
            .iter()
            .find(|m| &m.id == id)
            .map(|m| m.label.trim().to_string())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| id.clone()),
        None => "Custom connector".into(),
    }
}

/// What a commit hands back: the settings now persisted (to adopt), and
/// either the verified tool count (`None` = verify skipped) or the reason.
type Committed = (Option<RecordingSettings>, Result<Option<usize>, String>);

/// Persist `next` (`flushAiRuntimeSave`: listing reads the persisted config),
/// store the secret, list the tools. The writes run on the background executor,
/// the listing (an npx cold boot takes seconds) on tokio. An add that fails
/// after persisting rolls back: its secret is deleted and `before` is saved again.
#[allow(clippy::too_many_arguments)]
pub(super) async fn commit(
    cx: &AsyncApp,
    backend: Arc<dyn Backend>,
    before: RecordingSettings,
    next: RecordingSettings,
    id: String,
    secret: Option<String>,
    verify: bool,
    rollback: bool,
) -> Committed {
    let (b, sid, s) = (backend.clone(), id.clone(), secret.clone());
    let stored = cx
        .background_spawn(async move {
            let saved = b.save_domain(Unit::AiRuntime, &next)?;
            let checked = s.map_or(Ok(()), |s| b.mcp_set_secret(&sid, &s));
            Ok::<_, String>((saved, checked.map(|_| None)))
        })
        .await;
    let (saved, mut checked) = match stored {
        Ok(stored) => stored,
        Err(e) => return (None, Err(e)),
    };
    if verify && checked.is_ok() {
        checked = backend
            .mcp_tools(&id)
            .spawn(cx)
            .await
            .map(|t| Some(t.len()));
    }
    match checked {
        Ok(n) => (Some(saved), Ok(n)),
        Err(e) if rollback => {
            log::warn!("settings: MCP connector {id} failed to verify; rolled back");
            let had_secret = secret.is_some();
            let restored = cx
                .background_spawn(async move {
                    if had_secret {
                        let _ = backend.mcp_clear_secret(&id);
                    }
                    backend.save_domain(Unit::AiRuntime, &before).ok()
                })
                .await;
            (restored, Err(e))
        }
        Err(e) => {
            log::warn!("settings: MCP connector {id} failed to verify");
            (Some(saved), Err(e))
        }
    }
}

/// Add connector / Save (`submit` / `saveEdit`).
fn submit(s: &mut Settings, window: &mut Window, cx: &mut Context<Settings>) {
    let Some(before) = s.draft.clone() else {
        return;
    };
    let Some(f) = form_mut(s) else { return };
    let model = f.model(cx);
    if f.connecting || model.missing_target() {
        return;
    }
    let secret = (!model.oauth_http())
        .then(|| commands::take_input(&f.secret, window, cx))
        .filter(|v| !v.is_empty());
    let mut next = before.clone();
    let servers = &mut next.ai_runtime.mcp_servers;
    let (id, verify, rollback) = match f.edit.clone() {
        None => {
            let ids: Vec<String> = servers.iter().map(|m| m.id.clone()).collect();
            let new = model.build(&ids);
            let id = new.id.clone();
            // Prepended: the new row lands at the top (Svelte).
            servers.insert(0, new);
            (id, !model.oauth_http(), true)
        }
        Some(id) => {
            let Some(m) = servers.iter_mut().find(|m| m.id == id) else {
                return;
            };
            *m = model.to_config(id.clone(), m.enabled, m.enabled_tools.clone());
            let enabled = m.enabled;
            if secret.is_none() {
                // Field edits ride the ai_runtime autosave; nothing to verify.
                let edited = m.clone();
                s.edit(
                    move |d| {
                        if let Some(m) = d.ai_runtime.mcp_servers.iter_mut().find(|m| m.id == id) {
                            *m = edited;
                        }
                    },
                    cx,
                );
                close_picker(s, cx);
                return;
            }
            // A disabled connector (or stdio while Node is missing) can't list tools.
            let node_missing = model.stdio && s.ui.mcp.node == Some(None);
            (id, enabled && !node_missing, false)
        }
    };
    let Some(f) = form_mut(s) else { return };
    f.connecting = true;
    f.error = None;
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
                    if let Some(f) = form_mut(s) {
                        f.connecting = false;
                        f.error = Some(e);
                    }
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

/// `.field-label`: the kit's `.mx-label` (500, subtle), not an eyebrow.
pub(super) fn label(t: &'static str, p: &Palette) -> gpui_kit::Div {
    div()
        .text_size(text::BASE)
        .font_weight(crate::theme::W_MEDIUM)
        .text_color(p.text_subtle)
        .child(t)
}

/// `.group-hint`: 11 px, muted.
pub(super) fn hint(t: impl Into<SharedString>, p: &Palette) -> gpui_kit::Div {
    div()
        .text_size(text::SM)
        .line_height(gpui_kit::relative(1.5))
        .text_color(p.text_muted)
        .child(t.into())
}

pub(super) fn stack(children: Vec<AnyElement>) -> gpui_kit::Div {
    div().flex().flex_col().gap(px(6.)).children(children)
}

/// One `.inline-row`: the inputs, then its Remove.
fn inline_row(
    inputs: Vec<AnyElement>,
    remove_id: SharedString,
    remove: impl Fn(&mut Settings, &mut Context<Settings>) + 'static,
    cx: &mut Context<Settings>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .children(
            inputs
                .into_iter()
                .map(|i| div().flex_1().min_w_0().child(i)),
        )
        .child(
            Button::new(remove_id, "Remove")
                .sm()
                .on_click(click(cx, remove)),
        )
        .into_any_element()
}

pub fn render(s: &Settings, f: &Form, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let m = f.model(cx);
    let input = |id: &str, aria: String, st: &Entity<InputState>| {
        TextInput::new(SharedString::from(id.to_string()), aria, st).into_any_element()
    };
    let mut body: Vec<AnyElement> = vec![
        stack(vec![
            label("Name", p).into_any_element(),
            input("mcp-form-name", "Name".into(), &f.name),
        ])
        .into_any_element(),
        stack(vec![
            label("Transport", p).into_any_element(),
            Segmented::new(
                "mcp-form-transport",
                "Transport for the connector",
                ["stdio (local process)", "HTTP (remote)"],
                if f.stdio { 0 } else { 1 },
            )
            .on_change(crate::settings::on(cx, |s, ix: usize, cx| {
                if let Some(f) = form_mut(s) {
                    f.stdio = ix == 0;
                }
                write_through(s, cx);
                cx.notify();
            }))
            .into_any_element(),
        ])
        .into_any_element(),
    ];
    if f.stdio {
        body.push(
            stack(vec![
                label("Command", p).into_any_element(),
                input("mcp-form-command", "Command".into(), &f.command),
            ])
            .into_any_element(),
        );
        let mut args = vec![label("Arguments", p).into_any_element()];
        for (i, a) in f.args.iter().enumerate() {
            args.push(inline_row(
                vec![input(
                    &format!("mcp-form-arg-{i}"),
                    format!("Argument {}", i + 1),
                    a,
                )],
                format!("mcp-form-arg-rm-{i}").into(),
                move |s, cx| {
                    if let Some(f) = form_mut(s) {
                        if i < f.args.len() {
                            f.args.remove(i);
                        }
                    }
                    write_through(s, cx);
                    cx.notify();
                },
                cx,
            ));
        }
        args.push(
            Button::new("mcp-form-arg-add", "+ Argument")
                .sm()
                .on_click(commands::click_w(cx, |s, window, cx| {
                    let mut subs = Vec::new();
                    let e = field(window, cx, &mut subs, "argument", "");
                    e.update(cx, |st, cx| st.focus(window, cx));
                    if let Some(f) = form_mut(s) {
                        f.args.push(e);
                        f._subs.extend(subs);
                    }
                    cx.notify();
                }))
                .into_any_element(),
        );
        body.push(stack(args).into_any_element());
        let mut env = vec![
            label("Environment variables", p).into_any_element(),
            hint("Non-secret values only. The one secret below is stored encrypted and delivered separately.", p)
                .into_any_element(),
        ];
        for (i, (n, v)) in f.env.iter().enumerate() {
            env.push(inline_row(
                vec![
                    input(
                        &format!("mcp-form-env-name-{i}"),
                        format!("Env var {} name", i + 1),
                        n,
                    ),
                    input(
                        &format!("mcp-form-env-value-{i}"),
                        format!("Env var {} value", i + 1),
                        v,
                    ),
                ],
                format!("mcp-form-env-rm-{i}").into(),
                move |s, cx| {
                    if let Some(f) = form_mut(s) {
                        if i < f.env.len() {
                            f.env.remove(i);
                        }
                    }
                    write_through(s, cx);
                    cx.notify();
                },
                cx,
            ));
        }
        env.push(
            Button::new("mcp-form-env-add", "+ Variable")
                .sm()
                .on_click(commands::click_w(cx, |s, window, cx| {
                    let mut subs = Vec::new();
                    let n = field(window, cx, &mut subs, "NAME", "");
                    let v = field(window, cx, &mut subs, "value", "");
                    n.update(cx, |st, cx| st.focus(window, cx));
                    if let Some(f) = form_mut(s) {
                        f.env.push((n, v));
                        f._subs.extend(subs);
                    }
                    cx.notify();
                }))
                .into_any_element(),
        );
        body.push(stack(env).into_any_element());
        body.push(
            stack(vec![
                label("Secret delivered as env var", p).into_any_element(),
                input(
                    "mcp-form-secret-env",
                    "Secret delivered as env var".into(),
                    &f.secret_env,
                ),
                hint(
                    "The secret below is injected into the child process under this env var name.",
                    p,
                )
                .into_any_element(),
            ])
            .into_any_element(),
        );
    } else {
        let no_url = m.url.trim().is_empty();
        body.push(
            stack(vec![
                label("URL", p).into_any_element(),
                TextInput::new("mcp-form-url", "URL", &f.url)
                    .invalid(no_url)
                    .into_any_element(),
            ])
            .when(no_url, |d| {
                d.child(
                    div()
                        .text_size(text::BASE)
                        .text_color(p.warn)
                        .child("An HTTP connector needs a URL."),
                )
            })
            .into_any_element(),
        );
        body.push(
            stack(vec![
                label("Authorization", p).into_any_element(),
                Segmented::new(
                    "mcp-form-auth",
                    "Authorization for the connector",
                    ["Bearer secret", "OAuth"],
                    if f.oauth { 1 } else { 0 },
                )
                .on_change(crate::settings::on(cx, |s, ix: usize, cx| {
                    if let Some(f) = form_mut(s) {
                        f.oauth = ix == 1;
                    }
                    write_through(s, cx);
                    cx.notify();
                }))
                .into_any_element(),
                hint(
                    if f.oauth {
                        "Sign in through your browser when you add this connector \u{2014} nothing is pasted here. Only the returned token is kept, stored encrypted on this Mac."
                    } else {
                        "The secret below is sent as an Authorization: Bearer header."
                    },
                    p,
                )
                .into_any_element(),
            ])
            .into_any_element(),
        );
    }
    if !m.oauth_http() {
        let saved = f
            .edit
            .as_ref()
            .is_some_and(|id| s.data.mcp_secrets.contains(id));
        body.push(
            stack(vec![
                label("Secret (optional)", p).into_any_element(),
                TextInput::new("mcp-form-secret", "Secret (optional)", &f.secret)
                    .invalid(f.error.is_some())
                    .disabled(f.connecting)
                    .into_any_element(),
                if saved {
                    // `.saved-badge`: the check + "secret saved", never the value.
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(text::BASE)
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
                },
            ])
            .into_any_element(),
        );
    }
    if let Some(e) = &f.error {
        body.push(
            div()
                .id("mcp-form-error")
                .role(gpui_kit::Role::Alert)
                .text_size(text::BASE)
                .text_color(p.danger_text)
                .child(e.clone())
                .into_any_element(),
        );
    }
    let edit = f.edit.clone();
    let primary = if f.connecting {
        "Connecting\u{2026}"
    } else if edit.is_some() {
        "Save"
    } else {
        "Add connector"
    };
    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .pt(space::S3)
        .child(match edit {
            Some(id) => Button::new("mcp-form-remove", "Remove")
                .sm()
                .danger()
                .disabled(f.connecting)
                .on_click(click(cx, move |s, cx| {
                    if let Some(m) = s.d().ai_runtime.mcp_servers.iter().find(|m| m.id == id) {
                        super::remove(m, cx);
                    }
                })),
            None => Button::new("mcp-form-back", "Back")
                .sm()
                .disabled(f.connecting)
                .on_click(commands::click_w(cx, super::back_to_catalog)),
        })
        .child(
            Button::new("mcp-form-submit", primary)
                .busy(f.connecting)
                .disabled(f.connecting || m.missing_target())
                .on_click(commands::click_w(cx, submit)),
        );
    // The footer stays put while a long stdio form scrolls.
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .id("mcp-form")
                .flex()
                .flex_col()
                .gap(space::S4)
                .px(space::S5)
                .pb(space::S5)
                .max_h(px(480.))
                .overflow_y_scroll()
                .children(body),
        )
        .child(
            footer
                .px(space::S5)
                .pb(space::S5)
                .border_t_1()
                .border_color(p.hairline),
        )
        .into_any_element()
}
