//! The list column (`ChatList.svelte`): New chat, search, date-grouped history
//! with hover actions and inline rename, and the "Default model" footer.

use crate::ui::input::FieldText as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::Focusable as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, SharedString, Window};

use super::{format, Chat, History, Rename};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{icon, icon_button, kicker, mix, skeleton, with_tip, Button, Tone};

/// `.mx-search`; `focused` = its input has focus (`:focus-within`).
pub fn search_box(input: AnyElement, focused: bool, p: &Palette) -> Div {
    search_states(
        div()
            .flex()
            .items_center()
            .gap(space::S2)
            .h(size::H_MD)
            .pl(space::S3)
            .pr(space::S2)
            .bg(p.surface_subtle)
            .border_1()
            .rounded(radius::MD),
        focused,
        p,
    )
    .child(icon(IconName::Search.path(), px(13.)))
    .child(
        div()
            .flex_1()
            .min_w_0()
            .text_size(text::MD)
            .text_color(p.text_strong)
            .child(input),
    )
}

/// `.mx-search` borders and icon colour: `border` at rest, `border_strong` on
/// hover; with focus inside, `accent_border` + the 3 px glow ring + an accent icon.
pub fn search_states(el: Div, focused: bool, p: &Palette) -> Div {
    if focused {
        el.border_color(p.accent_border)
            .shadow(vec![crate::ui::ring(p.accent_glow)])
            .text_color(p.accent)
    } else {
        el.border_color(p.border)
            .text_color(p.text_subtle)
            .hover(|s| s.border_color(p.border_strong))
    }
}

pub fn render(
    chat: &Chat,
    narrow: bool,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Chat>,
) -> impl IntoElement {
    let query = chat.search.read(cx).value().trim().to_lowercase();
    let now = crate::ui::clock::now(cx);
    let count = match &chat.data.history {
        History::Rows(rows) if query.is_empty() => Some(rows.len()),
        _ => None,
    };
    let body: AnyElement = match &chat.data.history {
        History::Loading => div()
            .flex()
            .flex_col()
            .children((0..7).map(|i| {
                div()
                    .p(px(10.))
                    .when(i > 0, |d| d.border_t_1().border_color(p.hairline))
                    .child(skeleton::lines(2, p))
            }))
            .into_any_element(),
        History::Error => empty(
            EmptyLayout::Compact,
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "Couldn’t load your chats",
            "They’re still saved on this Mac.",
            vec![Button::new("chat-list-retry", "Retry")
                .sm()
                .icon("icons/rotate-ccw.svg")
                .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                .into_any_element()],
            p,
        )
        .into_any_element(),
        History::Rows(rows) => {
            // Rows that already answer this search (`search_conversations` also
            // matches answers) show as they are; otherwise filter locally meanwhile.
            let answered = chat.data.query.to_lowercase() == query;
            let shown: Vec<_> = rows
                .iter()
                .filter(|r| {
                    query.is_empty()
                        || answered
                        || r.title.to_lowercase().contains(&query)
                        || r.preview.to_lowercase().contains(&query)
                })
                .cloned()
                .collect();
            if shown.is_empty() && !query.is_empty() {
                empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    IconName::Search.path(),
                    format!("Nothing matches “{query}”"),
                    "Searched titles, questions and answers.",
                    vec![Button::new("chat-clear-search", "Clear search")
                        .ghost()
                        .sm()
                        .on_click(cx.listener(|this, _, window, cx| {
                            // `clearSearch`: set_value emits no Change, so re-list now.
                            this.search_task = None;
                            this.search.update(cx, |s, cx| {
                                s.set_value("", window, cx);
                                s.focus(window, cx);
                            });
                            this.reload(cx);
                        }))
                        .into_any_element()],
                    p,
                )
                .px(px(8.))
                .into_any_element()
            } else if shown.is_empty() {
                empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    "icons/message-circle.svg",
                    "No chats yet",
                    "Questions you ask here or in Quick Recall are kept here.",
                    Vec::new(),
                    p,
                )
                .px(px(8.))
                .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .children(format::group_history(&shown, now).into_iter().map(
                        |(label, items)| {
                            div()
                                .flex()
                                .flex_col()
                                .child(group_head(&label, p))
                                .children(items.iter().map(|c| row(chat, c, now, p, cx)))
                        },
                    ))
                    .into_any_element()
            }
        }
    };
    let default = chat.data.engine.default.clone();
    div()
        .id("chat-list")
        .role(gpui_kit::Role::Navigation)
        .aria_label("Chats")
        .flex()
        .flex_col()
        .flex_none()
        .w(if narrow {
            size::CHAT_LIST_W_NARROW
        } else {
            size::CHAT_LIST_W
        })
        .h_full()
        .bg(mix(p.surface, p.bg, 0.82))
        .border_r_1()
        .border_color(p.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .pt(space::S4)
                .pr(space::S3)
                .pb(space::S2)
                .pl(space::S4)
                .child(kicker("Chats", p))
                .children(count.map(|n| {
                    div()
                        .ml_auto()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(n.to_string())
                })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .px(space::S3)
                .pb(space::S3)
                .border_b_1()
                .border_color(p.hairline)
                .child(
                    Button::new("chat-new", "New chat")
                        .icon(IconName::Plus.path())
                        .kbd("⌘J")
                        .block()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.new_chat(None, false, window, cx)
                        })),
                )
                .child(search_box(
                    Input::new(&chat.search)
                        .aria_label("Search chats")
                        .appearance(false)
                        .px_0()
                        .field_h(size::H_MD - px(2.))
                        .field_text(text::MD)
                        .into_any_element(),
                    chat.search.focus_handle(cx).is_focused(window),
                    p,
                )),
        )
        .child(crate::ui::scroll::area(
            div()
                .id("chat-history")
                .role(gpui_kit::Role::List)
                .aria_label("Conversation history")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .pt(space::S1)
                .px(space::S2)
                .pb(space::S4)
                .child(body),
        ))
        .children(default.map(|(_, model)| {
            let tip = format!("Change the default in Settings → AI — {model}");
            with_tip(
                div()
                    .id("chat-default-model")
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .py(space::S2)
                    .px(space::S4)
                    .border_t_1()
                    .border_color(p.hairline)
                    .cursor_pointer()
                    .text_size(text::SM)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text_subtle)
                    .child("Default")
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_muted)
                            .child(SharedString::from(model)),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_color(p.text_faint)
                            .child(icon(IconName::ArrowRight.path(), px(12.))),
                    ),
                tip,
            )
            .on_click(|_, _, cx| {
                cx.defer(|cx| crate::window::open_settings(Some("intelligence"), cx))
            })
        }))
}

fn group_head(label: &str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(space::S4)
        .mx(px(10.))
        .mb(px(4.))
        .font_weight(theme::W_MEDIUM)
        .text_size(text::SM)
        .line_height(relative(1.))
        .text_color(p.text_subtle)
        .when(label == "Pinned", |d| {
            d.child(
                div()
                    .text_color(p.text_faint)
                    .child(icon("icons/pin.svg", px(10.))),
            )
        })
        .child(SharedString::from(label.to_string()))
}

fn row(
    chat: &Chat,
    c: &capture_types::ConversationSummary,
    now: time::OffsetDateTime,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let id = c.conversation_id.clone();
    let current = chat.active.as_deref() == Some(id.as_str());
    let title = [&c.title, &c.preview]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "Untitled chat".into());
    let renaming = chat.renaming == Some(Rename::Row(id.clone()));
    let key = gpui_kit::SharedString::from(format!("chat-row-{id}"));
    let group = format!("chat-row-g-{id}");
    let acts_bg = mix(p.fg, p.surface, if current { 0.08 } else { 0.04 });
    let act = |name: &'static str, label: &'static str, path: SharedString, pressed: bool| {
        with_tip(
            icon_button(
                SharedString::from(format!("chat-{name}-{id}")),
                label,
                size::H_SM,
                pressed,
                p,
            )
            .child(icon(path, px(14.))),
            label,
        )
    };
    let (pin_id, ren_id, del_id) = (id.clone(), id.clone(), id.clone());
    let title_el = if renaming {
        div()
            .id("chat-rename-box")
            // `onRenameKeydown`: Esc cancels (the field lets it through).
            .capture_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.cancel_rename(cx);
                }
            }))
            // A click in the field must not open the row under it.
            .on_click(|_, _, cx| cx.stop_propagation())
            .h(px(22.))
            .px(px(6.))
            .ml(px(-6.))
            .border_1()
            .border_color(p.accent_border)
            .rounded(radius::XS)
            .bg(p.surface_subtle)
            .shadow(vec![crate::ui::ring(p.accent_glow)])
            .text_color(p.text_strong)
            .font_weight(theme::W_MEDIUM)
            .text_size(text::MD)
            .child(
                Input::new(&chat.rename_input)
                    .aria_label("Rename chat")
                    .appearance(false)
                    .px_0()
                    .field_h(px(20.))
                    .field_text(text::MD),
            )
            .into_any_element()
    } else {
        with_tip(
            div()
                .id(SharedString::from(format!("chat-row-title-{id}")))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_weight(theme::W_MEDIUM)
                .text_size(text::MD)
                .line_height(relative(1.35))
                .child(SharedString::from(title.clone())),
            title,
        )
        .into_any_element()
    };
    div()
        .id(key)
        .test_support()
        .role(gpui_kit::Role::Link)
        .group(SharedString::from(group.clone()))
        .relative()
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(7.))
        .pr(px(8.))
        .pl(px(10.))
        .rounded(radius::MD)
        .cursor_pointer()
        .text_color(if current { p.text_strong } else { p.text })
        .when(current, |d| d.bg(p.wash_strong))
        .when(!current, |d| d.hover(|s| s.bg(p.wash)))
        .child(title_el)
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .line_height(relative(1.3))
                .text_color(p.text_subtle)
                .child(SharedString::from(format!(
                    "{} · {} turn{}",
                    format::chat_when(c.updated_at_ms, now),
                    c.turn_count,
                    if c.turn_count == 1 { "" } else { "s" }
                ))),
        )
        .when(!renaming, |d| {
            d.child(
                div()
                    .absolute()
                    .right(px(4.))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .pl(px(18.))
                    .invisible()
                    .group_hover(SharedString::from(group), |s| s.visible())
                    .child(
                        div()
                            .flex()
                            .rounded(radius::SM)
                            .bg(acts_bg)
                            .child(
                                act(
                                    "pin",
                                    if c.pinned { "Unpin" } else { "Pin" },
                                    "icons/pin.svg".into(),
                                    c.pinned,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_pin(pin_id.clone(), cx)
                                    },
                                )),
                            )
                            .child(
                                act("rename", "Rename", "icons/pencil.svg".into(), false).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.start_rename(Rename::Row(ren_id.clone()), window, cx)
                                    }),
                                ),
                            )
                            .child(
                                act("delete", "Delete", "icons/trash.svg".into(), false).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.delete(del_id.clone(), window, cx)
                                    }),
                                ),
                            ),
                    ),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| this.open(id.clone(), cx)))
        .into_any_element()
}
