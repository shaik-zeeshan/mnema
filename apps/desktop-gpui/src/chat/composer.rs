//! The one composer (`ChatComposer.svelte` + `ScopePanel.svelte`): large under
//! the greeting, docked under a thread. Enter sends, ⇧Enter newlines; Send
//! morphs to Stop while a turn streams. When Ask AI can't run, a notice with
//! the one helpful action replaces it.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Textarea;
use gpui_kit::prelude::*;
use gpui_kit::Focusable as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, radians, relative, svg, AnyElement, Context, Div, SharedString, Transformation, Window,
};

use super::scope::Scope;
use super::{format, Chat};
use crate::theme::{self, radius, size, space, text, Palette};
use crate::ui::date_input::DateInput;
use crate::ui::model_picker::{ModelPicker, Spec, Trigger};
use crate::ui::notice::notice;
use crate::ui::switch::Switch;
use crate::ui::{icon, mix, with_tip, Button, Tone};

/// The picker's spec for this chat (pin → Ask AI default → nothing).
pub fn picker_spec(chat: &Chat) -> Spec {
    let e = &chat.data.engine;
    Spec {
        providers: e.providers.clone(),
        pool: chat.pool.clone(),
        loading: chat.pool_loading,
        failures: chat.pool_failures.clone(),
        favorites: e.favorites.clone(),
        recents: e.recents.clone(),
        sentinel: Some(("Default".into(), e.default.clone())),
        sentinel_selected: chat.pin.is_none(),
        selected: chat.pin.clone().map(|(p, m)| (Some(p), m)),
        first_provider: e.default.as_ref().map(|(p, _)| p.clone()),
        foot: Some("applies to this chat only · Settings → AI".into()),
        can_star: true,
    }
}

fn picker(chat: &Chat, docked: bool, cx: &mut Context<Chat>) -> ModelPicker {
    let e = &chat.data.engine;
    let active = chat.pin.clone().or_else(|| e.default.clone());
    let spec = picker_spec(chat);
    let trigger = match &active {
        Some((provider, model)) => Trigger::Inline {
            glyph: Some(spec.letter(provider)),
            label: format::model_display_name(model),
            local: Some(spec.is_local(provider)),
            title: format!(
                "{}{} · {model}",
                if chat.pin.is_none() {
                    "Default · "
                } else {
                    ""
                },
                format::provider_label_by_id(&e.providers, provider)
            ),
        },
        None => Trigger::Inline {
            glyph: None,
            label: "Default".into(),
            local: None,
            title: "Default".into(),
        },
    };
    let this = cx.entity().downgrade();
    let open_this = this.clone();
    let star_this = this.clone();
    let picker = ModelPicker::new("chat-model", spec)
        .trigger(trigger)
        .up(docked)
        .on_open(move |_, cx| {
            let _ = open_this.update(cx, |c, cx| c.load_models(cx));
        })
        .on_select(move |engine, _, cx| {
            let _ = this.update(cx, |c, cx| c.select_model(engine, cx));
        })
        .on_favorite(move |engine, _, cx| {
            let _ = star_this.update(cx, |c, cx| c.toggle_favorite(engine, cx));
        });
    if chat.force_picker {
        picker.force_open()
    } else {
        picker
    }
}

/// `.ch-ctx`: Scope · Include · Goes to.
fn scope_panel(chat: &Chat, p: &Palette, cx: &mut Context<Chat>) -> Div {
    let today = crate::ui::clock::now(cx).date();
    let e = &chat.data.engine;
    let provider = chat
        .pin
        .clone()
        .or_else(|| e.default.clone())
        .map(|(p, _)| p);
    let config = provider
        .as_deref()
        .and_then(|id| e.providers.iter().find(|x| x.id == id));
    let local = format::provider_is_local(config);
    let name = provider
        .as_deref()
        .map(|id| format::provider_label_by_id(&e.providers, id))
        .unwrap_or_else(|| "your AI model".into());
    let via = format::provider_via(config);
    let via = if !via.is_empty() && !name.contains(&via) {
        via
    } else {
        String::new()
    };
    let unscoped: Vec<String> = [
        e.web_fetch.then(|| "web fetch".to_string()),
        (e.connectors > 0).then(|| {
            format!(
                "{} connector{}",
                e.connectors,
                if e.connectors == 1 { "" } else { "s" }
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    let foot: Vec<String> = [
        local.then(|| "text excerpts only · no frames or audio files".to_string()),
        (!unscoped.is_empty())
            .then(|| format!("+ {} — not limited by scope", unscoped.join(" · "))),
        (!local).then(|| format!("{name}’s data policy applies")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let goes = if local {
        let at = if via.is_empty() {
            String::new()
        } else {
            format!(" at {via}")
        };
        format!("Stays on this Mac. {name}{at}.")
    } else {
        let at = if via.is_empty() {
            String::new()
        } else if via.starts_with("your") {
            format!(" with {via}")
        } else {
            format!(" at {via}")
        };
        format!("Sent to {name}{at}.")
    };
    let body = if local {
        String::new()
    } else if chat.scope.about_you {
        " Only text: screen words, transcript lines, app and window names, page addresses, and Mnema’s notes about you. Never images or audio.".into()
    } else {
        " Only text: screen words, transcript lines, app and window names, and page addresses. Never images or audio.".into()
    };
    let (tone_fg, tone_bd, tone_bg) = if local {
        (
            p.accent,
            p.accent_border,
            mix(p.accent, p.surface_subtle, 0.05),
        )
    } else {
        (p.info, p.info_border, mix(p.info, p.surface_subtle, 0.06))
    };
    let dt = |label: &'static str, top: gpui_kit::Pixels| {
        div()
            .w(px(76.))
            .flex_none()
            .pt(top)
            .font_weight(theme::W_MEDIUM)
            .text_size(text::BASE)
            .line_height(relative(1.4))
            .text_color(p.text_subtle)
            .child(label)
    };
    let row = |label: &'static str, top, dd: AnyElement| {
        div()
            .flex()
            .gap(space::S4)
            .child(dt(label, top))
            .child(div().flex_1().min_w_0().pt(px(8.)).child(dd))
    };
    let about = chat.scope.about_you;
    div()
        .flex()
        .flex_col()
        .mx(space::S2)
        .pt(space::S2)
        .px(space::S2)
        .pb(space::S3)
        .border_t_1()
        .border_color(p.hairline)
        .child(row(
            "Scope",
            px(16.),
            scope_button(chat.scope, today, cx).into_any_element(),
        ))
        .child(row(
            "Include",
            px(16.),
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .pt(px(8.))
                .child(
                    Switch::new("chat-about-you", "About you", about).on_toggle({
                        let this = cx.entity().downgrade();
                        move |on, _, cx| {
                            let _ = this.update(cx, |c, cx| {
                                c.scope.about_you = on;
                                cx.notify();
                            });
                        }
                    }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::BASE)
                                .line_height(relative(1.4))
                                .text_color(p.text_strong)
                                .child("About you"),
                        )
                        .child(
                            div()
                                .text_size(text::SM)
                                .line_height(relative(1.45))
                                .text_color(p.text_subtle)
                                .child("Mnema’s notes about you (inferred from your activity)"),
                        ),
                )
                .into_any_element(),
        ))
        .child(row(
            "Goes to",
            px(16.),
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .mt(px(2.))
                .px(px(10.))
                .py(px(9.))
                .rounded(radius::MD)
                .bg(tone_bg)
                .border_1()
                .border_color(tone_bd)
                .text_size(text::SM)
                .line_height(relative(1.45))
                .text_color(p.text)
                .child(div().mt(px(2.)).text_color(tone_fg).child(icon(
                    if local {
                        "icons/laptop.svg"
                    } else {
                        "icons/cloud.svg"
                    },
                    px(15.),
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        // `<b>Sent to …</b> Only text: …` flows as one paragraph.
                        .child(super::inline_runs(&[
                            (goes.as_str(), Some((theme::W_SEMIBOLD, p.text_strong))),
                            (body.as_str(), None),
                        ]))
                        .when(!foot.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_color(p.text_subtle)
                                    .child(SharedString::from(foot.join(" · "))),
                            )
                        }),
                )
                .into_any_element(),
        ))
}

/// The scope trigger: the DateInput in range mode (All time + presets).
/// shortcut: docked under a thread the calendar still opens downward (no `up`
/// placement in DateInput); upgrade when the docked composer needs it.
fn scope_button(scope: Scope, today: time::Date, cx: &mut Context<Chat>) -> impl IntoElement {
    let span = scope.start.zip(scope.end);
    let this = cx.entity().downgrade();
    DateInput::new("chat-scope-range", today, today).span(span, move |next, _, cx| {
        let _ = this.update(cx, |c, cx| {
            c.scope.start = next.map(|(a, _)| a);
            c.scope.end = next.map(|(_, b)| b);
            cx.notify();
        });
    })
}

pub fn render(
    chat: &Chat,
    docked: bool,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let max_w = if docked {
        size::CHAT_COMPOSER_DOCKED_MAX_W
    } else {
        size::CHAT_THREAD_MAX_W
    };
    if chat.available() == Some(false) {
        let reason = chat
            .data
            .availability
            .as_ref()
            .and_then(|a| a.reason.clone())
            .unwrap_or_default();
        let (text, action, tone) = match reason.as_str() {
            "ask_ai_disabled" => (
                "Ask AI is off. Past chats stay readable here.",
                "Turn on in Settings →",
                Tone::Off,
            ),
            "ai_runtime_disabled" | "no_providers" => (
                "The reasoning engine is off. Chat answers over your history once it's enabled.",
                "Open settings",
                Tone::Off,
            ),
            r if r.contains("unreachable") => (
                "Can't reach the AI provider right now.",
                "Check again",
                Tone::Warn,
            ),
            _ => (
                "The reasoning engine needs attention in Settings.",
                "Open settings",
                Tone::Warn,
            ),
        };
        let glyph = if tone == Tone::Warn {
            IconName::TriangleAlert.path()
        } else {
            "icons/circle-off.svg".into()
        };
        return div()
            .w_full()
            .max_w(max_w)
            .mx_auto()
            .child(notice(
                tone,
                glyph,
                text,
                None,
                vec![Button::new("chat-off-action", action)
                    .ghost()
                    .sm()
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                    .into_any_element()],
                p,
            ))
            .into_any_element();
    }
    let streaming = chat.streaming;
    let focused = chat.composer.read(cx).focus_handle(cx).is_focused(window);
    let empty = chat.composer.read(cx).value().is_empty();
    let today = crate::ui::clock::now(cx).date();
    let send_to = chat
        .pin
        .clone()
        .or_else(|| chat.data.engine.default.clone())
        .map(|(id, _)| format::provider_label_by_id(&chat.data.engine.providers, &id));
    let send = if streaming {
        Button::icon_only("chat-send", crate::assets::STOP_SQUARE, "Stop")
            .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
            .into_any_element()
    } else {
        let b = if docked {
            Button::icon_only("chat-send", IconName::ArrowUp.path(), "Send")
        } else {
            Button::new(
                "chat-send",
                send_to.map_or("Send".into(), |s| format!("Send to {s}")),
            )
            .icon(IconName::ArrowUp.path())
            .kbd("↵")
        };
        b.primary()
            .disabled(chat.available().is_none())
            .on_click(cx.listener(|this, _, window, cx| this.send(None, window, cx)))
            .into_any_element()
    };
    let ctx_open = !docked || chat.ctx_open;
    div()
        .id(if docked {
            "chat-composer-docked"
        } else {
            "chat-composer"
        })
        .role(gpui_kit::Role::Form)
        .relative()
        .w_full()
        .max_w(max_w)
        .mx_auto()
        .rounded(radius::XL)
        .bg(p.surface_raised)
        .border_1()
        .border_color(if focused {
            p.border_hover
        } else {
            p.border_strong
        })
        .when(focused, |d| {
            d.shadow(vec![crate::ui::spread(
                crate::ui::alpha(p.fg, 0.03),
                px(4.),
            )])
        })
        .child(
            div()
                .relative()
                // 16 px (`--s-4`) less the kit textarea's own 10 px inset.
                .px(space::S4 - px(10.))
                .pt(if docked { space::S3 } else { space::S4 })
                .pb(if docked { space::S1 } else { space::S2 })
                .min_h(if docked { px(48.) } else { px(84.) })
                .text_size(if docked { text::MD } else { text::LG })
                .line_height(relative(1.5))
                .text_color(p.text_strong)
                .child(
                    Textarea::new(&chat.composer)
                        .aria_label("Message")
                        .appearance(false)
                        .px_0()
                        .py_0()
                        .text_size(if docked { text::MD } else { text::LG })
                        .disabled(streaming),
                )
                .when(!docked && empty, |d| {
                    // The placeholder, wrapping (`<textarea placeholder>` does);
                    // placed on the kit's text origin (10 px in, 4 px down).
                    d.child(
                        div()
                            .absolute()
                            .top(space::S4 + px(4.))
                            .left(space::S4)
                            .right(space::S4)
                            .text_color(p.text_muted)
                            .child(super::COMPOSER_PLACEHOLDER),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(space::S2)
                .py(space::S2)
                .pr(space::S2)
                .pl(space::S3)
                .child(picker(chat, docked, cx))
                .child(with_tip(
                    div()
                        .id("chat-ctx")
                        .test_support()
                        .role(gpui_kit::Role::Button)
                        .aria_expanded(ctx_open)
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .h(size::H_MD)
                        .px(px(12.))
                        .rounded(radius::SM)
                        .bg(p.surface_raised)
                        .border_1()
                        .border_color(p.border_strong)
                        .text_size(text::MD)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_muted)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.surface_hover))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(SharedString::from(chat.scope.summary(today))),
                        )
                        .child(
                            svg()
                                .path(IconName::ChevronDown.path())
                                .size(px(11.))
                                .text_color(p.text_muted)
                                .when(chat.ctx_open, |s| {
                                    s.with_transformation(Transformation::rotate(radians(
                                        std::f32::consts::PI,
                                    )))
                                }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ctx_open = !this.ctx_open;
                            cx.notify();
                        })),
                    "What the model may read",
                ))
                .child(div().ml_auto().child(send)),
        )
        .when(ctx_open, |d| d.child(scope_panel(chat, p, cx)))
        .into_any_element()
}
