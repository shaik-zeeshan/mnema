//! One turn (`ChatTurn.svelte`): the question bubble (radii lg lg xs lg), the
//! steps disclosure (live dot + label while working, "N steps · 3.4s" after),
//! the "Thought process" box (its tail kept in view while it streams), the
//! answer blocks, sources, and the actions row. Pure render of the TurnView.

use capture_types::{AnswerBlock, ToolActivityEntry};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::{
    div, px, radians, relative, svg, Animation, AnyElement, Context, Div, ScrollHandle,
    SharedString, Transformation,
};

use super::{blocks, format, prose, sources, Chat, ChatTurn};
use crate::theme::{self, motion, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::notice::{inline, notice};
use crate::ui::progress::spinner;
use crate::ui::{icon, icon_button, nid, with_tip, Button, Tone};

fn chevron(open: bool, p: &Palette) -> impl IntoElement {
    svg()
        .path(IconName::ChevronDown.path())
        .flex_none()
        .size(px(11.))
        .text_color(p.text_subtle)
        .when(open, |s| {
            s.with_transformation(Transformation::rotate(radians(std::f32::consts::PI)))
        })
}

/// A `details > summary` row.
fn summary(id: SharedString, p: &Palette) -> gpui_kit::Stateful<Div> {
    let hover = p.text_strong;
    div()
        .id(id)
        .role(gpui_kit::Role::Button)
        .flex()
        .items_center()
        .gap(px(6.))
        .py(px(4.))
        .cursor_pointer()
        .font_weight(theme::W_MEDIUM)
        .text_size(text::BASE)
        .line_height(relative(1.))
        .text_color(p.text_subtle)
        .hover(move |s| s.text_color(hover))
}

fn app_chip(a: &ToolActivityEntry, p: &Palette) -> Option<Div> {
    let app = a.app.clone()?;
    let first = app
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    // shortcut: app icons need the S8 image cache; the letter fallback stands in.
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_color(p.text_subtle)
            .child("·")
            .child(
                div()
                    .size(px(13.))
                    .rounded(px(3.))
                    .bg(p.surface_hover)
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(theme::W_SEMIBOLD)
                    .text_size(px(8.))
                    .child(SharedString::from(first)),
            )
            .child(SharedString::from(app)),
    )
}

fn steps_list(t: &ChatTurn, live: bool, p: &Palette) -> Div {
    let n = t.view.tool_activities.len();
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .mt(px(6.))
        .py(px(2.))
        .pl(px(12.))
        .border_l_1()
        .border_color(p.border)
        .children(t.view.tool_activities.iter().enumerate().map(|(i, a)| {
            let running = live && t.view.live_activity.is_some() && i + 1 == n;
            div()
                .id(nid(&format!("chat-step-{}", t.view.turn_index), i as u64))
                .flex()
                .items_center()
                .gap(space::S2)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::BASE)
                .line_height(relative(1.4))
                .text_color(if running { p.text_strong } else { p.text_muted })
                .child(
                    div()
                        .flex_none()
                        .text_color(if running { p.accent } else { p.text_subtle })
                        .map(|d| {
                            if running {
                                d.child(spinner(px(13.)))
                            } else {
                                d.child(icon(IconName::Check.path(), px(13.)))
                            }
                        }),
                )
                .child(SharedString::from(a.label.clone()))
                .children(app_chip(a, p))
                // Rows rise in (mx-rise 220 ms expo).
                .with_animation(
                    nid(&format!("chat-step-rise-{}", t.view.turn_index), i as u64),
                    Animation::new(motion::MED).with_easing(motion::expo),
                    |el, k| el.opacity(k).mt(px(6.) * (1. - k)),
                )
        }))
}

fn steps(chat: &Chat, t: &ChatTurn, p: &Palette, cx: &mut Context<Chat>) -> Option<Div> {
    let live = t.live();
    let line = chat.steps(t);
    if !live && line.is_none() {
        return None;
    }
    let ix = t.view.turn_index;
    let open = live || chat.steps_open.contains(&ix);
    let head = if live {
        let label = t
            .view
            .live_activity
            .as_ref()
            .map(|a| a.label.clone())
            .unwrap_or_else(|| {
                if t.view.phase == "streaming" {
                    "Writing".into()
                } else {
                    "Thinking".into()
                }
            });
        summary(SharedString::from(format!("chat-steps-{ix}")), p).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_color(p.text_muted)
                .child(
                    // `box-shadow: 0 0 0 3px glow` as a 12 px glow disc (GPUI
                    // shadows don't follow the radius).
                    div()
                        .size(px(12.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .mx(px(-3.))
                        .rounded_full()
                        .bg(p.accent_glow)
                        .child(div().size(px(6.)).rounded_full().bg(p.accent)),
                )
                .child(SharedString::from(format!("{label}…"))),
        )
    } else {
        summary(SharedString::from(format!("chat-steps-{ix}")), p)
            .aria_expanded(open)
            .child(icon(IconName::Check.path(), px(12.)))
            .children(line.map(SharedString::from))
            .child(chevron(open, p))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.steps_open.remove(&ix) {
                    this.steps_open.insert(ix);
                }
                cx.notify();
            }))
    };
    Some(
        div()
            .mb(space::S4)
            .child(head)
            .when(open && !t.view.tool_activities.is_empty(), |d| {
                d.child(steps_list(t, live, p))
            }),
    )
}

fn reasoning(chat: &Chat, t: &ChatTurn, p: &Palette, cx: &mut Context<Chat>) -> Option<Div> {
    let r = t
        .view
        .reasoning
        .as_deref()
        .filter(|r| !r.trim().is_empty())?;
    let ix = t.view.turn_index;
    let live = t.live() && t.view.blocks.is_empty();
    let open = live || chat.reason_open.contains(&ix);
    let handle = ScrollHandle::new();
    if live {
        // Keep the streaming tail in view.
        handle.scroll_to_bottom();
    }
    Some(
        div()
            .mb(space::S3)
            .child(
                summary(SharedString::from(format!("chat-reason-{ix}")), p)
                    .aria_expanded(open)
                    .child(icon("icons/sparkle.svg", px(12.)))
                    .child("Thought process")
                    .child(chevron(open, p))
                    .when(!live, |el| {
                        el.on_click(cx.listener(move |this, _, _, cx| {
                            if !this.reason_open.remove(&ix) {
                                this.reason_open.insert(ix);
                            }
                            cx.notify();
                        }))
                    }),
            )
            .when(open, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("chat-reason-box-{ix}")))
                        .mt(px(6.))
                        .pl(px(12.))
                        .border_l_1()
                        .border_color(p.border)
                        .max_h(px(12. * 12. * 1.55))
                        .overflow_y_scroll()
                        .track_scroll(&handle)
                        .text_size(text::BASE)
                        .line_height(relative(1.55))
                        .text_color(p.text_subtle)
                        .child(SharedString::from(r.to_string())),
                )
            }),
    )
}

fn error_notice(
    chat: &Chat,
    t: &ChatTurn,
    trailing: bool,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> Div {
    let copy = error_copy(t.view.error_message.as_deref(), t.error_kind.as_deref());
    let acts = if trailing {
        let disabled = chat.streaming || chat.available() != Some(true);
        vec![match copy.action {
            Action::Settings => Button::new("chat-err-settings", "Open settings")
                .sm()
                // shortcut: Settings navigation is the shell's (S13).
                .on_click(|_, _, _| log::debug!("chat: error -> settings (S13)"))
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
    notice(
        copy.tone,
        copy.icon,
        copy.title,
        Some(copy.text.into()),
        acts,
        p,
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Retry,
    Settings,
    NewChat,
}

struct ErrorCopy {
    tone: Tone,
    icon: SharedString,
    title: &'static str,
    text: String,
    action: Action,
}

/// `isReasonCode`: `snake_case`, optionally `:<id>`, no spaces.
pub fn is_reason_code(m: &str) -> bool {
    let (head, tail) = m.split_once(':').map_or((m, None), |(h, t)| (h, Some(t)));
    !head.is_empty()
        && head.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        && tail.is_none_or(|t| !t.is_empty() && !t.chars().any(char::is_whitespace))
}

/// `turnErrorCopy` + ChatErrorTurn's head: plain copy and the ONE action; a raw
/// resolve code never reaches the screen.
/// shortcut: reason codes map to generic copy (no provider names, no ChatGPT
/// "Sign in again" button) until S13 brings `aiRuntimeReasonLabelFor` across.
fn error_copy(message: Option<&str>, kind: Option<&str>) -> ErrorCopy {
    let raw = message.unwrap_or("").trim();
    let code = is_reason_code(raw);
    let unreachable = kind == Some("unreachable") || raw.contains("unreachable");
    let alert: SharedString = IconName::TriangleAlert.path();
    if code {
        if unreachable {
            return ErrorCopy {
                tone: Tone::Warn,
                icon: "icons/cloud-off.svg".into(),
                title: "Can’t reach the AI provider right now",
                text: "Can't reach the AI provider right now — your sign-in is fine.".into(),
                action: Action::Retry,
            };
        }
        return ErrorCopy {
            tone: Tone::Danger,
            icon: alert,
            title: "The AI settings need a look",
            text: "The reasoning engine needs attention in Settings.".into(),
            action: Action::Settings,
        };
    }
    let text = if raw.is_empty() {
        "The engine couldn't answer.".to_string()
    } else {
        raw.to_string()
    };
    let (tone, title, action) = match kind {
        _ if unreachable => (
            Tone::Warn,
            "Can’t reach the AI provider right now",
            Action::Retry,
        ),
        Some("quota") => (
            Tone::Warn,
            "The provider is limiting requests",
            Action::Retry,
        ),
        Some("context_too_long") => (
            Tone::Warn,
            "This chat is too long for the model",
            Action::NewChat,
        ),
        Some("settings") | Some("auth") => (
            Tone::Danger,
            "The AI settings need a look",
            Action::Settings,
        ),
        _ => (Tone::Danger, "Couldn’t answer this one", Action::Retry),
    };
    ErrorCopy {
        tone,
        icon: if unreachable {
            "icons/cloud-off.svg".into()
        } else {
            alert
        },
        title,
        text,
        action,
    }
}

fn answer(t: &ChatTurn, p: &Palette) -> Div {
    let n = t.view.blocks.len();
    div()
        .flex()
        .flex_col()
        .children(
            t.view
                .blocks
                .iter()
                .enumerate()
                .map(|(bi, b)| -> AnyElement {
                    match b {
                        AnswerBlock::Prose { markdown } => {
                            let streaming = t.view.phase != "done" && bi + 1 == n;
                            // `.answer-prose` has no outer margin; blocks carry theirs.
                            div()
                                .child(prose::prose(
                                    nid(&format!("chat-prose-{}", t.view.turn_index), bi as u64),
                                    markdown,
                                    streaming,
                                    p,
                                ))
                                .into_any_element()
                        }
                        AnswerBlock::Bars { title, items } => blocks::panel(
                            title.as_deref(),
                            None,
                            blocks::bars(items, p).into_any_element(),
                            p,
                        )
                        .into_any_element(),
                        AnswerBlock::Dossier { items } => blocks::panel(
                            Some("Relevant to this"),
                            Some("from User Context"),
                            blocks::dossier(items, p).into_any_element(),
                            p,
                        )
                        .into_any_element(),
                        AnswerBlock::Timeline { title, items } => blocks::panel(
                            None,
                            None,
                            blocks::timeline(title.as_deref(), items, p).into_any_element(),
                            p,
                        )
                        .into_any_element(),
                    }
                }),
        )
}

fn actions(chat: &Chat, t: &ChatTurn, trailing: bool, p: &Palette, cx: &mut Context<Chat>) -> Div {
    let ix = t.view.turn_index;
    let copied = chat.copied == Some(ix);
    let has_text = !t.plain_text().is_empty();
    let can_regen = trailing && !chat.streaming && chat.available() == Some(true);
    let providers = &chat.data.engine.providers;
    let model = t.view.model.as_deref().map(format::model_display_name);
    let where_ = t.view.provider.as_deref().and_then(|id| {
        providers.iter().find(|x| x.id == id).map(|x| {
            if format::provider_is_local(Some(x)) {
                "local"
            } else {
                "cloud"
            }
        })
    });
    let window = t
        .view
        .model
        .as_deref()
        .and_then(format::context_window_for_model);
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .mt(space::S3)
        .when(has_text, |d| {
            d.child(
                with_tip(
                    icon_button(
                        SharedString::from(format!("chat-copy-{ix}")),
                        "Copy answer",
                        size::H_SM,
                        false,
                        p,
                    )
                    .child(icon(
                        if copied {
                            IconName::Check.path()
                        } else {
                            IconName::Copy.path()
                        },
                        px(15.),
                    )),
                    if copied { "Copied" } else { "Copy" },
                )
                .on_click(cx.listener(move |this, _, _, cx| this.copy_answer(ix, cx))),
            )
            .when(copied, |d| {
                d.child(
                    div().mx(px(6.)).child(
                        inline(
                            Tone::Ok,
                            None,
                            vec![SharedString::from("Copied").into_any_element()],
                            p,
                        )
                        .text_size(text::SM),
                    ),
                )
            })
        })
        .when(can_regen, |d| {
            d.child(
                with_tip(
                    icon_button("chat-regenerate", "Regenerate", size::H_SM, false, p)
                        .child(icon("icons/rotate-cw.svg", px(15.))),
                    "Regenerate",
                )
                .on_click(cx.listener(|this, _, window, cx| this.regenerate(window, cx))),
            )
        })
        .children(model.map(|m| {
            div()
                .ml(space::S2)
                .flex()
                .font_weight(theme::W_MEDIUM)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(div().font_family(FONT_MONO).child(SharedString::from(m)))
                .children(where_.map(|w| SharedString::from(format!(" · {w}"))))
        }))
        .children(t.view.context_tokens.map(|tokens| {
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap(px(6.))
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .map(|d| match window {
                    Some(w) => d
                        .child(
                            div()
                                .relative()
                                .w(px(40.))
                                .h(px(4.))
                                .rounded(px(2.))
                                .bg(crate::ui::alpha(p.fg, 0.08))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .h_full()
                                        .w(relative((tokens as f32 / w as f32).min(1.)))
                                        .bg(p.chart_grey_4),
                                ),
                        )
                        .child(SharedString::from(format!(
                            "{} / {}",
                            format::format_token_count(tokens),
                            format::format_token_count(w)
                        ))),
                    None => d.child(SharedString::from(format!(
                        "{} tokens",
                        format::format_token_count(tokens)
                    ))),
                })
        }))
}

pub fn render(
    chat: &Chat,
    t: &ChatTurn,
    trailing: bool,
    ruled: bool,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let now = crate::ui::clock::now(cx);
    let asked = t.at_ms.map(|ms| format::weekday_clock(ms, now));
    let ix = t.view.turn_index;
    let question = div()
        .ml_auto()
        .mb(px(6.))
        .max_w(relative(0.78))
        .px(px(14.))
        .py(px(10.))
        .rounded_tl(radius::LG)
        .rounded_tr(radius::LG)
        .rounded_br(radius::XS)
        .rounded_bl(radius::LG)
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .text_size(text::MD)
        .line_height(relative(1.5))
        .text_color(p.text_strong)
        .child(SharedString::from(t.view.question.clone()));
    let meta = asked.map(|a| {
        div()
            .mb(space::S4)
            .text_right()
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .line_height(relative(1.))
            .text_color(p.text_subtle)
            .child(SharedString::from(a))
    });
    let body: Vec<AnyElement> = if t.view.phase == "error" {
        let mut v = Vec::new();
        if let Some(s) = steps(chat, t, p, cx) {
            v.push(s.into_any_element());
        }
        v.push(error_notice(chat, t, trailing, p, cx).into_any_element());
        v
    } else {
        let mut v = Vec::new();
        if let Some(s) = steps(chat, t, p, cx) {
            v.push(s.into_any_element());
        }
        v.push(
            div()
                .flex()
                .flex_col()
                .children(reasoning(chat, t, p, cx))
                .child(answer(t, p))
                .into_any_element(),
        );
        if t.view.phase == "done" {
            let srcs = super::model::sources(&t.view.sources);
            if !srcs.is_empty() {
                v.push(sources::row(ix, &srcs, p, cx).into_any_element());
            }
            v.push(actions(chat, t, trailing, p, cx).into_any_element());
        }
        v
    };
    div()
        .id(SharedString::from(format!("chat-turn-{ix}")))
        .role(gpui_kit::Role::Article)
        .flex()
        .flex_col()
        .when(ruled, |d| {
            d.mt(space::S6)
                .pt(space::S6)
                .border_t_1()
                .border_dashed()
                .border_color(p.hairline)
        })
        .child(question)
        .children(meta)
        .children(body)
        .into_any_element()
}
