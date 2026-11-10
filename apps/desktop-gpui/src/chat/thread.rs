//! The conversation pane (`Chat.svelte` markup): the 48 px header (title,
//! turns · started, pin / rename / delete), the transcript (max 760) or the
//! new-chat greeting, and the docked composer with "Jump to latest".

use crate::ui::input::FieldText as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, SharedString, Window};

use super::{composer, format, turn, Chat, Pane, Rename};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{icon, icon_button, kicker, skeleton, with_tip, Button, Tone};

/// "Try asking": text, icon, tint, scope preset, mono hint (`ChatEmpty.svelte`).
const SUGGESTIONS: [(&str, &str, Tint, &str, &str); 6] = [
    (
        "What did I work on yesterday?",
        "icons/monitor.svg",
        Tint::Screen,
        "Yesterday",
        "yday",
    ),
    (
        "Summarize my calls this week",
        "icons/mic.svg",
        Tint::Mic,
        "This week",
        "week",
    ),
    (
        "Find the PR link I saw Tuesday",
        "icons/search.svg",
        Tint::Screen,
        "This week",
        "week",
    ),
    (
        "What was I reading before lunch?",
        "icons/monitor.svg",
        Tint::Screen,
        "Today",
        "today",
    ),
    (
        "Who did I talk to most this week?",
        "icons/volume-2.svg",
        Tint::SysAudio,
        "This week",
        "week",
    ),
    (
        "Draft my standup from today",
        "icons/user.svg",
        Tint::Info,
        "Today",
        "today",
    ),
];

#[derive(Clone, Copy)]
enum Tint {
    Screen,
    Mic,
    SysAudio,
    Info,
}

fn header(chat: &Chat, p: &Palette, cx: &mut Context<Chat>) -> Div {
    let now = crate::ui::clock::now(cx);
    let opening = matches!(chat.pane, Pane::Loading | Pane::LoadError(_));
    let row = chat.active.as_deref().and_then(|id| chat.row(id));
    let count = if opening {
        row.map_or(0, |r| r.turn_count as usize)
    } else {
        chat.turns.len()
    };
    let started = chat.created_at_ms.or(row.map(|r| r.created_at_ms));
    let pinned = chat.head_pinned();
    let title: AnyElement = if chat.renaming == Some(Rename::Header) {
        div()
            .flex_initial()
            .w(px(420.))
            .min_w_0()
            .h(px(30.))
            .px(px(6.))
            .ml(px(-7.))
            .border_1()
            .border_color(p.accent_border)
            .rounded(radius::XS)
            .bg(p.surface_subtle)
            .shadow(vec![crate::ui::ring(p.accent_glow)])
            .font_weight(theme::W_SEMIBOLD)
            .text_size(text::LG)
            .text_color(p.text_strong)
            .child(
                Input::new(&chat.rename_input)
                    .aria_label("Rename chat")
                    .appearance(false)
                    .px_0()
                    .field_h(px(28.))
                    .field_text(text::LG),
            )
            .into_any_element()
    } else {
        div()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .font_weight(theme::W_SEMIBOLD)
            .text_size(text::LG)
            .line_height(relative(1.2))
            .text_color(p.text_strong)
            .child(SharedString::from(chat.display_title()))
            .into_any_element()
    };
    let meta = (count > 0).then(|| {
        let started = started
            .map(|ms| format!(" · started {}", format::weekday_clock(ms, now)))
            .unwrap_or_default();
        div()
            .flex_none()
            .whitespace_nowrap()
            .font_weight(theme::W_MEDIUM)
            .text_size(text::XS)
            .text_color(p.text_subtle)
            .child(SharedString::from(format!(
                "{count} turn{}{started}",
                if count == 1 { "" } else { "s" }
            )))
    });
    let act =
        |id: &'static str, label: &'static str, tip: &'static str, path: SharedString, on: bool| {
            with_tip(
                icon_button(id, label, size::H_SM, on, p).child(icon(path, px(15.))),
                tip,
            )
        };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(space::S3)
        .h(size::CHAT_HEAD_H)
        .pl(space::S6)
        .pr(space::S4)
        .border_b_1()
        .border_color(p.hairline)
        .bg(crate::ui::mix(p.bg, gpui_kit::transparent_black(), 0.7))
        .child(title)
        .children(meta)
        .child(
            div()
                .ml_auto()
                .flex()
                .gap(px(2.))
                .child(
                    act(
                        "chat-head-pin",
                        if pinned { "Unpin chat" } else { "Pin chat" },
                        if pinned { "Unpin" } else { "Pin" },
                        "icons/pin.svg".into(),
                        pinned,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.active.clone() {
                            this.toggle_pin(id, cx)
                        }
                    })),
                )
                .child(
                    act(
                        "chat-head-rename",
                        "Rename chat",
                        "Rename",
                        "icons/pencil.svg".into(),
                        false,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_rename(Rename::Header, window, cx)
                    })),
                )
                .child(
                    act(
                        "chat-head-delete",
                        "Delete chat",
                        "Delete",
                        "icons/trash.svg".into(),
                        false,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(id) = this.active.clone() {
                            this.delete(id, window, cx)
                        }
                    })),
                ),
        )
}

fn greeting(chat: &Chat, p: &Palette, window: &mut Window, cx: &mut Context<Chat>) -> Div {
    let hour = crate::ui::clock::now(cx).hour();
    let suggest = chat.available() == Some(true);
    let tint = |t: Tint| match t {
        Tint::Screen => p.source_screen,
        Tint::Mic => p.source_mic,
        Tint::SysAudio => p.source_sysaudio,
        Tint::Info => p.info,
    };
    // `clamp(32px, 9vh, 96px)` top padding.
    let top = (window.viewport_size().height * 0.09).clamp(px(32.), px(96.));
    let hello = format!("{} ", format::greeting(hour));
    const ASK: &str = "What do you want to remember?";
    // shortcut: balances against the column's max width (or the window, if
    // narrower), not the laid-out width; a sidebar-squeezed column wraps naturally.
    let avail = size::CHAT_THREAD_MAX_W.min(window.viewport_size().width) - space::S6 * 2.;
    let title_w = crate::ui::balanced_width(
        &[(&hello, theme::W_SEMIBOLD), (ASK, theme::W_MEDIUM)],
        theme::FONT_UI,
        text::DISPLAY,
        avail,
        window,
    );
    div()
        .w_full()
        .max_w(size::CHAT_THREAD_MAX_W)
        .mx_auto()
        .pt(top)
        .px(space::S6)
        .pb(space::S6)
        .child(
            div()
                .mb(space::S5)
                .child(kicker("New chat", p))
                .child(
                    div()
                        .my(space::S3)
                        .max_w(title_w)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_size(text::DISPLAY)
                        .line_height(relative(1.04))
                        .text_color(p.text_strong)
                        .child(super::inline_runs(&[
                            (&hello, None),
                            (ASK, Some((theme::W_MEDIUM, p.text_muted))),
                        ])),
                )
                .child(
                    div()
                        .text_size(text::PROSE)
                        .line_height(relative(1.6))
                        .text_color(p.text_muted)
                        .child("Answers come from what Mnema captured on this Mac. The model only reads inside the scope below, and you choose where it runs."),
                ),
        )
        .child(composer::render(chat, false, p, window, cx))
        .when(suggest, |d| {
            d.child(
                div()
                    .mt(space::S5)
                    .child(
                        div()
                            .mb(space::S2)
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::BASE)
                            .text_color(p.text_subtle)
                            .child("Try asking"),
                    )
                    .child(div().flex().flex_wrap().gap(space::S2).children(
                        SUGGESTIONS.iter().enumerate().map(|(i, (label, path, t, preset, hint))| {
                            let (label, preset) = (*label, *preset);
                            let c = tint(*t);
                            div()
                                .id(crate::ui::nid("chat-suggest", i as u64))
                                .test_support()
                                .role(gpui_kit::Role::Button)
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .h(size::H_SM)
                                .px(px(9.))
                                .rounded(radius::BTN_SM)
                                .bg(p.surface_raised)
                                .border_1()
                                .border_color(p.border_strong)
                                .text_size(text::BASE)
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text_strong)
                                .cursor_pointer()
                                .hover(|s| s.bg(p.surface_hover))
                                .child(div().text_color(c).child(icon(*path, px(13.))))
                                .child(label)
                                .child(
                                    div()
                                        .font_family(FONT_MONO)
                                        .text_size(text::XS)
                                        .font_weight(theme::W_REGULAR)
                                        .text_color(p.text_subtle)
                                        .child(*hint),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.pick_suggestion(label, preset, window, cx)
                                }))
                        }),
                    )),
            )
        })
}

fn loading(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .ml_auto()
                .mb(space::S5)
                .w(relative(0.46))
                .h(px(40.))
                .child(skeleton::skel(p).size_full().rounded(radius::LG)),
        )
        .child(div().w(relative(0.3)).mb(px(16.)).child(skeleton::title(p)))
        .child(skeleton::lines(4, p))
        .child(div().mt(px(24.)).child(skeleton::chart(5, p)))
}

pub fn render(
    chat: &Chat,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Chat>,
) -> impl IntoElement {
    let new = chat.is_new() && chat.pane != Pane::Missing;
    let thread_body: Option<AnyElement> = (!new).then(|| {
        let inner: AnyElement = match &chat.pane {
            Pane::Missing => empty(
                EmptyLayout::Center,
                Tone::Neutral,
                "icons/trash.svg",
                "This chat was deleted",
                "It isn’t on this Mac anymore.",
                vec![Button::new("chat-missing-new", "New chat")
                    .sm()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.new_chat(None, false, window, cx)),
                    )
                    .into_any_element()],
                p,
            )
            .mt(px(80.))
            .into_any_element(),
            Pane::Loading => loading(p).into_any_element(),
            Pane::LoadError(e) => empty(
                EmptyLayout::Center,
                Tone::Danger,
                IconName::TriangleAlert.path(),
                "Couldn’t open this chat",
                format!("It’s still saved on this Mac. {e}"),
                vec![Button::new("chat-open-retry", "Try again")
                    .sm()
                    .icon("icons/rotate-ccw.svg")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.active.clone() {
                            this.turns.clear();
                            this.open(id, cx)
                        }
                    }))
                    .into_any_element()],
                p,
            )
            .mt(px(80.))
            .into_any_element(),
            _ => {
                let last = chat.turns.len().saturating_sub(1);
                div()
                    .flex()
                    .flex_col()
                    .children(
                        chat.turns
                            .iter()
                            .enumerate()
                            .map(|(i, t)| turn::render(chat, t, i == last, i > 0, p, cx)),
                    )
                    .into_any_element()
            }
        };
        div()
            .w_full()
            .max_w(size::CHAT_THREAD_MAX_W)
            .mx_auto()
            .pt(space::S6)
            .px(space::S6)
            .pb(space::S5)
            .child(inner)
            .into_any_element()
    });
    let docked = !new && matches!(chat.pane, Pane::Thread);
    let at_bottom = chat_at_bottom(chat);
    div()
        .id("chat-main")
        .role(gpui_kit::Role::Region)
        .aria_label("Conversation")
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .when(!new, |d| d.child(header(chat, p, cx)))
        .child(
            div()
                .id("chat-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&chat.scroll)
                .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
                .map(|d| match thread_body {
                    Some(body) => d.child(body),
                    None => d.child(greeting(chat, p, window, cx)),
                })
                .child(crate::ui::scroll::bar(&chat.scroll)),
        )
        .when(docked, |d| {
            d.child(
                div()
                    .relative()
                    .flex_none()
                    .px(space::S6)
                    .pb(space::S4)
                    .bg(p.bg)
                    .when(!at_bottom, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_full()
                                .mb(px(8.))
                                .left_0()
                                .right_0()
                                .flex()
                                .justify_center()
                                .child(
                                    Button::new("chat-jump", "Jump to latest")
                                        .sm()
                                        .icon(IconName::ArrowDown.path())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.scroll.scroll_to_bottom();
                                            cx.notify()
                                        })),
                                ),
                        )
                    })
                    .child(composer::render(chat, true, p, window, cx)),
            )
        })
}

fn chat_at_bottom(chat: &Chat) -> bool {
    let (off, max) = (chat.scroll.offset().y, chat.scroll.max_offset().y);
    f32::from(max + off) <= 40.
}
