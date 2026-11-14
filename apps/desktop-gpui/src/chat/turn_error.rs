//! A failed turn (`ChatErrorTurn.svelte`): one notice with a title, one plain
//! sentence and the ONE action that can help. `error_copy` is `turnErrorCopy`
//! plus the notice head, over Insights' `reason_copy`, so a raw resolve code
//! never reaches the screen. A rejected ChatGPT login offers "Sign in again"
//! inline (`ChatgptConnect`'s device-code flow); an unreachable provider never
//! asks for a sign-in (ADR 0058).

use capture_types::RecordingSettings;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, ClipboardItem, Context, Div, SharedString};
use serde_json::Value;
use std::time::Duration;

use super::{Chat, ChatTurn};
use crate::insights::model::{reason_copy, Engine};
use crate::theme::{self, size, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::link::Link;
use crate::ui::notice::notice;
use crate::ui::Tracking as _;
use crate::ui::{icon, icon_button, with_tip, Button, Tone};

const VERIFY_HOST: &str = "auth.openai.com/codex/device";
/// `ChatgptConnect.svelte`: the copy button reads "Copied" for 1.5 s.
pub const CODE_COPIED_FOR: Duration = Duration::from_millis(1500);

impl Chat {
    /// The device code on the clipboard, and "Copied" on its button (either click).
    pub fn copy_code(&mut self, code: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(code));
        self.code_copied = true;
        self.code_copied_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CODE_COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                this.code_copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Retry,
    Settings,
    NewChat,
    /// Sign in to this ChatGPT provider instance again; Retry can't help first.
    Reconnect(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorCopy {
    pub tone: Tone,
    /// The cloud-off icon (offline never reads as a sign-in problem).
    pub unreachable: bool,
    pub title: &'static str,
    pub text: String,
    pub action: Action,
}

/// The inline ChatGPT sign-in, per provider instance.
#[derive(Clone, Debug, PartialEq)]
pub enum Login {
    Starting,
    Waiting { code: String, url: String },
    Failed(String),
}

/// `isReasonCode`: `snake_case`, optionally `:<id>`, no spaces.
pub fn is_reason_code(m: &str) -> bool {
    let (head, tail) = m.split_once(':').map_or((m, None), |(h, t)| (h, Some(t)));
    !head.is_empty()
        && head.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        && tail.is_none_or(|t| !t.is_empty() && !t.chars().any(char::is_whitespace))
}

/// `turnErrorCopy` + ChatErrorTurn's head.
pub fn error_copy(
    message: Option<&str>,
    kind: Option<&str>,
    settings: Option<&RecordingSettings>,
) -> ErrorCopy {
    let raw = message.unwrap_or("").trim();
    let (text, action) = if is_reason_code(raw) {
        match reason_copy(raw, settings) {
            Engine::Unreachable(t) => (t, Action::Retry),
            Engine::Fix(_, Some(id)) => {
                let providers = settings.map_or(&[][..], |s| &s.ai_runtime.providers[..]);
                let name = super::format::provider_label_by_id(providers, &id);
                (
                    format!("{name} signed you out. Sign in again to keep chatting — retrying won't help until then."),
                    Action::Reconnect(id),
                )
            }
            Engine::Fix(t, None) => (t, Action::Settings),
            // `reason_copy` answers only the two above.
            _ => ("The engine couldn't answer.".into(), Action::Settings),
        }
    } else {
        let text = if raw.is_empty() {
            "The engine couldn't answer."
        } else {
            raw
        };
        let action = match kind {
            Some("settings" | "auth") => Action::Settings,
            Some("context_too_long") => Action::NewChat,
            _ => Action::Retry,
        };
        (text.to_string(), action)
    };
    let unreachable = kind == Some("unreachable") || raw.contains("unreachable");
    let (tone, title) = match (&action, kind) {
        (Action::Reconnect(_), _) => (Tone::Danger, "Sign in again to keep chatting"),
        _ if unreachable => (Tone::Warn, "Can’t reach the AI provider right now"),
        (_, Some("quota")) => (Tone::Warn, "The provider is limiting requests"),
        (_, Some("context_too_long")) => (Tone::Warn, "This chat is too long for the model"),
        (Action::Settings, _) => (Tone::Danger, "The AI settings need a look"),
        _ => (Tone::Danger, "Couldn’t answer this one"),
    };
    ErrorCopy {
        tone,
        unreachable,
        title,
        text,
        action,
    }
}

impl Chat {
    /// `beginLogin`: ask for a device code, open the verify page, show the code.
    pub fn sign_in(&mut self, id: String, cx: &mut Context<Self>) {
        self.login = Some((id.clone(), Login::Starting));
        let b = self.sources.clone();
        let bid = id.clone();
        let task = cx.background_spawn(async move { b.chatgpt_begin(&bid) });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |this, cx| {
                if !matches!(&this.login, Some((l, Login::Starting)) if *l == id) {
                    return;
                }
                let state = match got {
                    Ok((code, url)) => {
                        crate::ui::link::open(&url, cx);
                        Login::Waiting { code, url }
                    }
                    Err(e) => Login::Failed(e),
                };
                this.login = Some((id, state));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// `cancelLogin`: an abandoned poll would keep hitting OpenAI's rate limit.
    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.login.take() {
            let b = self.sources.clone();
            cx.background_spawn(async move { b.chatgpt_cancel(&id) })
                .detach();
        }
        cx.notify();
    }

    /// `chatgpt_login_update`: the poll's one outcome; the composer comes back
    /// through a fresh availability read either way.
    pub fn on_chatgpt_update(&mut self, v: &Value, cx: &mut Context<Self>) {
        let mine = |(id, _): &(String, Login)| v["providerId"].as_str() == Some(id.as_str());
        if let Some((id, _)) = self.login.take_if(|l| mine(l)) {
            if v["connected"].as_bool() != Some(true) {
                let why = v["error"].as_str().unwrap_or("Sign-in failed.");
                self.login = Some((id, Login::Failed(why.into())));
            }
        }
        self.reload(cx);
        cx.notify();
    }
}

/// `hint`'s one line, for `baseline` rows.
const HINT: Line = Line {
    family: FONT_UI,
    size: text::SM,
    lh: 1.45,
};

fn hint(s: &'static str, p: &Palette) -> Div {
    div()
        .text_size(text::SM)
        .line_height(gpui_kit::relative(1.45))
        .text_color(p.text_muted)
        .child(s)
}

/// `ChatgptConnect` with "Sign in again", for provider instance `id`.
fn sign_in(chat: &Chat, id: &str, p: &Palette, cx: &mut Context<Chat>) -> AnyElement {
    let state = chat.login.as_ref().filter(|(l, _)| l == id).map(|(_, s)| s);
    let col = div().flex().flex_col().gap(px(6.)).max_w(px(280.));
    let begin = |el: &'static str, label: &'static str, id: &str, cx: &mut Context<Chat>| {
        let id = id.to_string();
        Button::new(el, label)
            .sm()
            .on_click(cx.listener(move |this, _, _, cx| this.sign_in(id.clone(), cx)))
    };
    if let Some(Login::Waiting { code, url }) = state {
        let copy = |c: &String| {
            let c = c.clone();
            cx.listener(move |this, _: &gpui_kit::ClickEvent, _, cx| this.copy_code(c.clone(), cx))
        };
        let copied = chat.code_copied;
        let label = if copied { "Copied" } else { "Copy code" };
        return col
            // `.chatgpt-connect__hint`: one paragraph, so the link takes the
            // hint's size and line height and every first line shares a baseline.
            .child(
                baseline::row_top(vec![
                    (HINT, hint("Enter this code at", p)),
                    (
                        HINT,
                        div().child(Link::new("chat-signin-page", VERIFY_HOST, url.clone())),
                    ),
                    (HINT, hint("— waiting for approval…", p)),
                ])
                .flex_wrap()
                .gap(px(4.)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("chat-signin-code")
                            .test_support()
                            .role(gpui_kit::Role::Button)
                            .aria_label("Copy code")
                            .cursor_pointer()
                            .font_family(FONT_MONO)
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::LG)
                            .tracking(0.12)
                            .text_color(p.text_strong)
                            .child(SharedString::from(code.clone()))
                            .on_click(copy(code)),
                    )
                    .child(
                        with_tip(
                            icon_button("chat-signin-copy", label, size::H_SM, false, p).child(
                                icon(
                                    if copied {
                                        IconName::Check.path()
                                    } else {
                                        IconName::Copy.path()
                                    },
                                    px(14.),
                                ),
                            ),
                            label,
                        )
                        .on_click(copy(code)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(begin("chat-signin-restart", "Start over", id, cx).ghost())
                    .child(
                        Button::new("chat-signin-cancel", "Cancel")
                            .sm()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_sign_in(cx))),
                    ),
            )
            .into_any_element();
    }
    let starting = state == Some(&Login::Starting);
    // `.chatgpt-connect__actions` is a row: the button keeps its own width.
    col.child(
        div().flex().child(
            begin(
                "chat-signin",
                if starting {
                    "Contacting OpenAI…"
                } else {
                    "Sign in again"
                },
                id,
                cx,
            )
            .busy(starting)
            .disabled(starting),
        ),
    )
    .child(hint(
        "Signs in with your own ChatGPT Plus/Pro account in the browser — no API key. Usage counts against your plan's Codex limits. The sign-in token is stored only in the encrypted vault.",
        p,
    ))
    .children(match state {
        Some(Login::Failed(e)) => Some(
            div()
                .text_size(text::SM)
                .text_color(p.danger)
                .child(SharedString::from(e.clone())),
        ),
        _ => None,
    })
    .into_any_element()
}

pub fn error_notice(
    chat: &Chat,
    t: &ChatTurn,
    trailing: bool,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> Div {
    let copy = error_copy(
        t.view.error_message.as_deref(),
        t.error_kind.as_deref(),
        chat.data.settings.as_ref(),
    );
    // Only the trailing turn acts: a mid-thread action would collide turn indexes.
    let acts = if trailing {
        let disabled = chat.streaming || chat.available() != Some(true);
        vec![match &copy.action {
            Action::Reconnect(id) => sign_in(chat, id, p, cx),
            Action::Settings => Button::new("chat-err-settings", "Open settings")
                .sm()
                .on_click(|_, _, cx| {
                    cx.defer(|cx| crate::window::open_settings(Some("intelligence"), cx))
                })
                .into_any_element(),
            Action::NewChat => Button::new("chat-err-new", "New chat")
                .sm()
                .on_click(cx.listener(|this, _, window, cx| this.new_chat(None, false, window, cx)))
                .into_any_element(),
            Action::Retry => Button::new("chat-err-retry", "Retry")
                .sm()
                .icon("icons/rotate-ccw.svg")
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.regenerate(window, cx)))
                .into_any_element(),
        }]
    } else {
        Vec::new()
    };
    let icon_path: SharedString = if copy.unreachable {
        "icons/cloud-off.svg".into()
    } else {
        IconName::TriangleAlert.path()
    };
    notice(
        copy.tone,
        icon_path,
        copy.title,
        Some(copy.text.into()),
        acts,
        p,
    )
}

#[cfg(test)]
#[path = "turn_error_tests.rs"]
mod tests;
