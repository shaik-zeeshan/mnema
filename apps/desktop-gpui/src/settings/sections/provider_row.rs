//! One connected provider instance (`ProviderRow.svelte`): the state word,
//! Test again and Manage, expanding in place to Label, Base URL / Endpoint,
//! the API key and Remove provider; a ChatGPT row carries its sign-in block
//! (`ChatgptConnect.svelte`) open or closed.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use capture_types::{AiProviderConfig, AiProviderKind};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Div, Entity, SharedString, Subscription};
use serde_json::Value;

use super::providers::{cloud, local_default};
use crate::chat::format::{kind_label, provider_instance_label, url_host};
use crate::settings::row::{line, row, state_word};
use crate::settings::{click, icons, Reach, Settings};
use crate::theme::{radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::copy::CopyButton;
use crate::ui::input::TextInput;
use crate::ui::link::Link;
use crate::ui::model_picker::ProviderFailure;
use crate::ui::{Button, Tone};

/// `ChatgptConnect`'s "Copied" flash.
const CODE_COPIED_FOR: Duration = Duration::from_millis(1500);
const VERIFY_HOST: &str = "auth.openai.com/codex/device";

/// One row's text fields.
pub struct PvInputs {
    pub label: Entity<InputState>,
    pub base_url: Entity<InputState>,
    pub key: Entity<InputState>,
    /// Which key placeholder the field shows (saved / not).
    pub saved_placeholder: Option<bool>,
    pub _subs: Vec<Subscription>,
}

/// A ChatGPT device sign-in in progress.
#[derive(Clone, Debug, PartialEq)]
pub enum Login {
    Starting,
    Waiting { code: String, url: String },
}

/// The AI runtime's Test connection.
#[derive(Default)]
pub struct Test {
    pub running: bool,
    /// `AiRuntimeTestResult` JSON, or why the command failed.
    pub result: Option<Result<Value, String>>,
    pub raw_open: bool,
}

/// What the provider rows hold beyond the drafts (the `ai-runtime` store).
#[derive(Default)]
pub struct PvUi {
    pub inputs: HashMap<String, PvInputs>,
    /// Fields whose typed key a landed save / clear forgets (next render).
    pub clear_key_field: HashSet<String>,
    pub key_errors: HashMap<String, String>,
    /// The id whose key save / clear is in flight (one at a time).
    pub saving: Option<String>,
    /// A removal's keychain clear is in flight (Add provider waits).
    pub removing: bool,
    pub removal_error: Option<String>,
    pub login: HashMap<String, Login>,
    pub login_error: HashMap<String, String>,
    pub disconnecting: Option<String>,
    /// The model listing's per-provider failures.
    pub failures: Vec<ProviderFailure>,
    pub test: Test,
}

/// `providerRowState`: the state word and its tone.
pub(crate) fn row_state(
    pv: &AiProviderConfig,
    key_saved: bool,
    reason: &str,
    list_failed: bool,
    listed: bool,
) -> (&'static str, Option<Tone>) {
    let id = pv.id.to_lowercase();
    let reason = reason.to_lowercase();
    if reason == format!("needs_reconnect:{id}") {
        return ("needs reconnect", Some(Tone::Danger));
    }
    if reason == format!("provider_unreachable:{id}") {
        return ("unreachable", Some(Tone::Warn));
    }
    if pv.kind == AiProviderKind::OpenaiCompatible && pv.base_url.trim().is_empty() {
        return ("needs base URL", Some(Tone::Warn));
    }
    if pv.kind == AiProviderKind::Chatgpt {
        return if key_saved {
            ("connected", Some(Tone::Ok))
        } else {
            ("not connected", None)
        };
    }
    let is_cloud = cloud(pv.kind);
    if is_cloud && !key_saved {
        return ("no key", None);
    }
    match (list_failed, is_cloud, listed) {
        (true, _, _) => ("not reachable", Some(Tone::Warn)),
        (_, _, true) => ("reachable", Some(Tone::Ok)),
        (_, true, false) => ("key saved", Some(Tone::Ok)),
        _ => ("on this Mac", None),
    }
}

/// The failed verify's line under the row (`reachabilityFailureText`).
fn reach_line(pv: &AiProviderConfig, why: &str, p: &Palette) -> Div {
    let rejected = why.eq_ignore_ascii_case("authentication failed");
    let text = if !cloud(pv.kind) && why.eq_ignore_ascii_case("unreachable") {
        format!(
            "The local endpoint could not be reached. Start {}, then test again.",
            kind_label(pv.kind)
        )
    } else {
        let short = match why.to_lowercase().as_str() {
            "authentication failed" => "rejected this key".to_string(),
            "unreachable" => "couldn\u{2019}t reach it".to_string(),
            r => r.to_string(),
        };
        format!("Not reachable \u{2014} {short}.")
    };
    if rejected {
        line(Tone::Danger, icons::ALERT, text, p)
    } else {
        line(Tone::Warn, icons::CLOUD_OFF, text, p)
    }
}

fn nid(prefix: &str, id: &str) -> SharedString {
    SharedString::from(format!("{prefix}-{id}"))
}

pub fn render(
    s: &Settings,
    pv: &AiProviderConfig,
    reason: &str,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let key_saved = s.data.keys_saved.contains(&pv.id);
    let is_cloud = cloud(pv.kind);
    let chatgpt = pv.kind == AiProviderKind::Chatgpt;
    let key_kind = is_cloud && !chatgpt;
    let typed = s.typed_key(&pv.id, cx);
    let key_error = s.ui.pv.key_errors.get(&pv.id).cloned();
    let needs_base_url =
        pv.kind == AiProviderKind::OpenaiCompatible && pv.base_url.trim().is_empty();
    // Pinned open while something here still needs the user: a typed key only
    // reaches the vault on Save key, so collapsing would hide the warning.
    let pinned = !typed.is_empty() || key_error.is_some() || needs_base_url;
    let expanded = s.ui.provider_open.as_deref() == Some(pv.id.as_str()) || pinned;
    let host = url_host(if pv.base_url.is_empty() && !is_cloud {
        local_default(pv.kind)
    } else {
        &pv.base_url
    })
    .to_string();
    let desc = if expanded {
        key_kind.then(|| {
            if key_saved {
                "A key is saved \u{2014} paste a new one to replace it. It goes to the encrypted vault, never a config file.".to_string()
            } else {
                "Paste an API key. It goes to the encrypted vault, never a config file.".to_string()
            }
        })
    } else if chatgpt {
        None
    } else if key_kind {
        let ending = s
            .data
            .key_endings
            .get(&pv.id)
            .filter(|_| key_saved)
            .map(|e| format!("\u{2026}{e}"));
        let lead = if key_saved {
            "Key in the encrypted vault"
        } else {
            "No key yet"
        };
        let parts: Vec<String> = [Some(lead.to_string()), ending, Some(host.clone())]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        Some(parts.join(" \u{b7} "))
    } else {
        Some(format!("{host} \u{b7} no key, nothing leaves this Mac"))
    };
    // This row's own verify outranks the picker's model-list probe.
    let reach = s.ui.reach.get(&pv.id);
    let checking = reach == Some(&Reach::Checking);
    let verdict = reach.filter(|_| !checking);
    let list_failure = s.ui.pv.failures.iter().find(|f| f.provider == pv.id);
    let (list_failed, listed) = match verdict {
        Some(r) => (matches!(r, Reach::Failed(_)), *r == Reach::Reachable),
        None => (
            list_failure.is_some(),
            s.pool.iter().any(|m| m.provider == pv.id),
        ),
    };
    let (word, tone) = row_state(pv, key_saved, reason, list_failed, listed);
    let reason_here = {
        let l = reason.to_lowercase();
        let ours = ["needs_reconnect:", "provider_unreachable:"]
            .iter()
            .any(|pre| l.starts_with(pre) && reason[pre.len()..] == pv.id);
        ours.then(|| reason.to_string())
    };
    let below: Option<Div> = if let Some(r) = &reason_here {
        let needs = r.to_lowercase().starts_with("needs_");
        let text = crate::insights::model::reason_label(r, Some(&s.d()));
        Some(if needs {
            line(Tone::Danger, icons::ALERT, text, p)
        } else {
            line(Tone::Warn, icons::CLOUD_OFF, text, p)
        })
    } else if let (Some(Reach::Failed(why)), "not reachable") = (verdict, word) {
        Some(reach_line(pv, why, p))
    } else if let (Some(f), "not reachable") = (list_failure, word) {
        let text = if is_cloud {
            format!(
                "Couldn\u{2019}t list this provider\u{2019}s models \u{2014} {}.",
                f.reason
            )
        } else {
            format!(
                "The local endpoint could not be reached. Start {}, then retry.",
                kind_label(pv.kind)
            )
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(line(Tone::Warn, icons::CLOUD_OFF, text, p))
                .child(
                    Button::new(nid("pv-retry", &pv.id), "Retry")
                        .ghost()
                        .sm()
                        .disabled(s.pool_loading)
                        .on_click(click(cx, |s, cx| s.load_pool(cx))),
                ),
        )
    } else {
        None
    };
    let test_id = pv.id.clone();
    let open_id = pv.id.clone();
    let ctl = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .whitespace_nowrap()
        .child(state_word(word, tone, p))
        .when(!chatgpt && (key_saved || !is_cloud), |d| {
            d.child(
                Button::new(
                    nid("pv-test", &pv.id),
                    if checking {
                        "Testing\u{2026}"
                    } else {
                        "Test again"
                    },
                )
                .ghost()
                .disabled(checking)
                .busy(checking)
                .spinner(checking)
                .on_click(click(cx, move |s, cx| s.test_provider(test_id.clone(), cx))),
            )
        })
        .when(!pinned, |d| {
            d.child(
                Button::new(
                    nid("pv-manage", &pv.id),
                    if expanded { "Done" } else { "Manage" },
                )
                .on_click(click(cx, move |s, cx| {
                    s.ui.provider_open = if s.ui.provider_open.as_deref() == Some(open_id.as_str())
                    {
                        None
                    } else {
                        Some(open_id.clone())
                    };
                    cx.notify();
                })),
            )
        });
    let r = row(provider_instance_label(pv))
        .desc_opt(desc)
        .extra(chip(
            if is_cloud { "cloud" } else { "local" },
            ChipKind::Neutral,
            p,
        ))
        .when(below.is_some(), |r| r.below(below.unwrap()));
    // The fields and the ChatGPT block need the full width.
    let r = if expanded || chatgpt {
        let body = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(space::S2)
            .w_full()
            .min_w_0()
            .when(chatgpt, |d| {
                d.child(chatgpt_block(s, &pv.id, key_saved, p, cx))
            })
            .when(expanded, |d| {
                d.child(fields(s, pv, key_kind, key_saved, &typed, key_error, p, cx))
            });
        r.full().aside(ctl).control(body)
    } else {
        r.control(ctl)
    };
    r.build(p).into_any_element()
}

/// `.pr-fields`: a 76 px label column beside each field.
fn field_row(label: &'static str, field: impl IntoElement, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .w_full()
        .child(
            div()
                .w(px(76.))
                .flex_none()
                .text_size(text::SM)
                .font_weight(crate::theme::W_MEDIUM)
                .text_color(p.text_muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(field))
}

#[allow(clippy::too_many_arguments)]
fn fields(
    s: &Settings,
    pv: &AiProviderConfig,
    key_kind: bool,
    key_saved: bool,
    typed: &str,
    key_error: Option<String>,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let Some(inputs) = s.ui.pv.inputs.get(&pv.id) else {
        return div().into_any_element();
    };
    let saving_here = s.ui.pv.saving.as_deref() == Some(pv.id.as_str());
    let busy = s.ui.pv.saving.is_some();
    let needs_base_url =
        pv.kind == AiProviderKind::OpenaiCompatible && pv.base_url.trim().is_empty();
    let mut col = div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .w_full()
        .max_w(px(640.))
        .pt(space::S1)
        .child(field_row(
            "Label",
            TextInput::new(nid("pv-label", &pv.id), "Label", &inputs.label),
            p,
        ));
    if pv.kind == AiProviderKind::OpenaiCompatible {
        col = col.child(field_row(
            "Base URL",
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    TextInput::new(nid("pv-base-url", &pv.id), "Base URL", &inputs.base_url)
                        .num()
                        .invalid(needs_base_url),
                )
                .when(needs_base_url, |d| {
                    d.child(line(
                        Tone::Warn,
                        icons::INFO,
                        "OpenAI-compatible providers need a base URL.",
                        p,
                    ))
                }),
            p,
        ));
    } else if !cloud(pv.kind) {
        col = col.child(field_row(
            "Endpoint",
            TextInput::new(nid("pv-endpoint", &pv.id), "Endpoint", &inputs.base_url).num(),
            p,
        ));
    }
    if key_kind {
        let (save_id, clear_id) = (pv.id.clone(), pv.id.clone());
        let hint: Option<Div> = match key_error {
            Some(e) => Some(line(Tone::Danger, icons::ALERT, e, p)),
            None if !typed.is_empty() && !saving_here => Some(crate::ui::notice::inline(
                Tone::Warn,
                Some(crate::ui::icon(icons::INFO, crate::theme::size::ICON_SM).into_any_element()),
                vec![
                    "Unsaved API key \u{2014} click".into_any_element(),
                    crate::ui::notice::strong("Save key", p).into_any_element(),
                    "to store it in the vault.".into_any_element(),
                ],
                p,
            )),
            None => None,
        };
        col = col.child(field_row(
            "API key",
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .gap(space::S2)
                        .child(
                            div().flex_1().min_w_0().child(
                                TextInput::new(nid("pv-key", &pv.id), "API key", &inputs.key)
                                    .num()
                                    .invalid(s.ui.pv.key_errors.contains_key(&pv.id))
                                    .disabled(saving_here),
                            ),
                        )
                        .child(
                            Button::new(
                                nid("pv-save-key", &pv.id),
                                if saving_here {
                                    "Saving\u{2026}"
                                } else {
                                    "Save key"
                                },
                            )
                            .disabled(busy || typed.is_empty())
                            .busy(saving_here)
                            .spinner(saving_here)
                            .on_click(cx.listener(
                                move |s, _: &gpui_kit::ClickEvent, window, cx| {
                                    s.save_provider_key(save_id.clone(), window, cx)
                                },
                            )),
                        )
                        .child(
                            Button::new(nid("pv-clear-key", &pv.id), "Clear")
                                .ghost()
                                .disabled(busy || !key_saved)
                                .on_click(click(cx, move |s, cx| {
                                    s.clear_provider_key(clear_id.clone(), cx)
                                })),
                        ),
                )
                .children(hint),
            p,
        ));
    }
    let remove_id = pv.id.clone();
    col.child(
        div().flex().pl(px(76.) + space::S3).child(
            Button::new(nid("pv-remove", &pv.id), "Remove provider")
                .danger()
                .sm()
                .disabled(busy || s.ui.pv.removing)
                .on_click(click(cx, move |s, cx| {
                    s.remove_provider(remove_id.clone(), cx)
                })),
        ),
    )
    .into_any_element()
}

fn hint(t: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::XS)
        .line_height(gpui_kit::relative(1.45))
        .text_color(p.text_muted)
        .child(t.into())
}

/// `ChatgptConnect`: Connect / Reconnect / Disconnect, or the device code.
fn chatgpt_block(
    s: &Settings,
    id: &str,
    connected: bool,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let login = s.ui.pv.login.get(id);
    let error = s.ui.pv.login_error.get(id).cloned();
    let col = div().flex().flex_col().gap(space::S2).w_full();
    let begin = |el: SharedString, label: &'static str, cx: &mut Context<Settings>| {
        let id = id.to_string();
        Button::new(el, label).on_click(click(cx, move |s, cx| s.chatgpt_begin(id.clone(), cx)))
    };
    let col = if let Some(Login::Waiting { code, url }) = login {
        let copy_text = code.clone();
        let cancel_id = id.to_string();
        col.child(
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .gap(px(4.))
                .child(hint("Enter this code at", p))
                .child(Link::new(nid("pv-cg-page", id), VERIFY_HOST, url.clone()))
                .child(hint("\u{2014} waiting for approval\u{2026}", p)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(
                    div()
                        .id(nid("pv-cg-code", id))
                        .test_support()
                        .role(gpui_kit::Role::Button)
                        .aria_label("Click to copy")
                        .cursor_pointer()
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(radius::MD)
                        .border_1()
                        .border_dashed()
                        .border_color(p.border)
                        .font_family(FONT_MONO)
                        .text_size(text::LG)
                        .text_color(p.text)
                        .child(SharedString::from(code.clone()))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                copy_text.clone(),
                            ))
                        }),
                )
                .child(
                    CopyButton::icon(nid("pv-cg-copy", id), "Copy code", code.clone())
                        .tip("Copy code")
                        .revert(CODE_COPIED_FOR),
                ),
        )
        .child(
            div()
                .flex()
                .gap(space::S2)
                .child(begin(nid("pv-cg-restart", id), "Start over", cx).ghost())
                .child(
                    Button::new(nid("pv-cg-cancel", id), "Cancel")
                        .ghost()
                        .on_click(click(cx, move |s, cx| {
                            s.chatgpt_cancel(cancel_id.clone(), cx)
                        })),
                ),
        )
    } else {
        let starting = login == Some(&Login::Starting);
        let disconnecting = s.ui.pv.disconnecting.as_deref() == Some(id);
        let off_id = id.to_string();
        col.child(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(
                    begin(
                        nid("pv-cg-connect", id),
                        if starting {
                            "Contacting OpenAI\u{2026}"
                        } else if connected {
                            "Reconnect ChatGPT"
                        } else {
                            "Connect ChatGPT"
                        },
                        cx,
                    )
                    .disabled(starting || disconnecting)
                    .busy(starting),
                )
                .when(connected, |d| {
                    d.child(
                        Button::new(
                            nid("pv-cg-disconnect", id),
                            if disconnecting { "Disconnecting\u{2026}" } else { "Disconnect" },
                        )
                        .ghost()
                        .disabled(disconnecting || login.is_some())
                        .on_click(click(cx, move |s, cx| s.chatgpt_disconnect(off_id.clone(), cx))),
                    )
                }),
        )
        .child(hint(
            "Signs in with your own ChatGPT Plus/Pro account in the browser \u{2014} no API key. Usage counts against your plan's Codex limits. The sign-in token is stored only in the encrypted vault.",
            p,
        ))
    };
    col.children(error.map(|e| {
        div()
            .id(nid("pv-cg-error", id))
            .test_support()
            .role(gpui_kit::Role::Alert)
            .text_size(text::XS)
            .text_color(p.danger)
            .child(SharedString::from(e))
    }))
    .into_any_element()
}

/// [`crate::settings::fixture::PROVIDER_STATES`]: Ollama's Manage open; an
/// unsaved key typed into Anthropic's; a ChatGPT row just added; its device code.
#[cfg(feature = "fixtures")]
pub fn fixture_state(
    s: &mut Settings,
    which: &str,
    window: &mut gpui_kit::Window,
    cx: &mut Context<Settings>,
) {
    match which {
        "provider-manage-local" => s.ui.provider_open = Some("ollama".into()),
        "provider-manage-key" => {
            s.ui.provider_open = Some("anthropic".into());
            if let Some(i) = s.ui.pv.inputs.get("anthropic") {
                i.key.update(cx, |k, cx| k.set_value("skab", window, cx));
            }
        }
        "provider-chatgpt" | "provider-chatgpt-code" => {
            s.add_provider(AiProviderKind::Chatgpt, cx);
            if which == "provider-chatgpt-code" {
                use crate::settings::fixture::{CHATGPT_CODE, CHATGPT_URL};
                s.ui.provider_open = None;
                s.ui.pv.login.insert(
                    "chatgpt".into(),
                    Login::Waiting {
                        code: CHATGPT_CODE.into(),
                        url: CHATGPT_URL.into(),
                    },
                );
            }
        }
        _ => {}
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pv(kind: AiProviderKind, base_url: &str) -> AiProviderConfig {
        AiProviderConfig {
            id: "x".into(),
            kind,
            label: String::new(),
            base_url: base_url.into(),
        }
    }

    #[test]
    fn the_state_word_matches_provider_row_state() {
        let a = pv(AiProviderKind::Anthropic, "");
        assert_eq!(row_state(&a, false, "", false, false).0, "no key");
        assert_eq!(row_state(&a, true, "", false, false).0, "key saved");
        assert_eq!(row_state(&a, true, "", false, true).0, "reachable");
        assert_eq!(row_state(&a, true, "", true, false).0, "not reachable");
        assert_eq!(
            row_state(&a, true, "needs_reconnect:x", false, true).0,
            "needs reconnect"
        );
        assert_eq!(
            row_state(&a, true, "provider_unreachable:X", false, true).0,
            "unreachable"
        );
        let o = pv(AiProviderKind::Ollama, "");
        assert_eq!(row_state(&o, false, "", false, false).0, "on this Mac");
        assert_eq!(row_state(&o, false, "", true, false).0, "not reachable");
        let c = pv(AiProviderKind::OpenaiCompatible, " ");
        assert_eq!(row_state(&c, true, "", false, true).0, "needs base URL");
        let g = pv(AiProviderKind::Chatgpt, "");
        assert_eq!(row_state(&g, false, "", true, false).0, "not connected");
        assert_eq!(row_state(&g, true, "", false, false).0, "connected");
    }
}
