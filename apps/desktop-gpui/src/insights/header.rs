//! The Insights bar (`.in-bar`): 56 px, solid surface, hairline under it; four
//! tabs (16 px medium, counts beside) with a 2 px ink under the active one on
//! the hairline; the active tab's controls on the right. And the one engine
//! notice over the view (`notice` in `+page.svelte`): reason-keyed copy.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Div, Role, SharedString, Window};

use super::model::{engine_state, Engine, Tab};
use super::Insights;
use crate::theme::{self, space, text, Palette};
use crate::ui::notice::notice as notice_box;
use crate::ui::{Button, Tone};

pub const BAR_H: f32 = 56.;

pub fn bar(this: &Insights, p: &Palette, window: &mut Window, cx: &mut Context<Insights>) -> Div {
    let count = |t: Tab| match t {
        Tab::Subjects => this.status.ctx.as_ref().map(|c| c.subject_count),
        Tab::Context => this.status.authored.map(|n| n as i64),
        _ => None,
    };
    let tabs = Tab::ALL.into_iter().map(|t| {
        let on = this.tab == t;
        div()
            .id(SharedString::from(format!(
                "insights-tab-{}",
                t.label().to_lowercase()
            )))
            .test_support()
            .role(Role::Tab)
            .relative()
            .flex()
            .items_center()
            .gap(px(7.))
            .h_full()
            .cursor_pointer()
            .text_size(text::LG)
            .font_weight(theme::W_MEDIUM)
            .text_color(if on { p.text_strong } else { p.text_subtle })
            .hover(|s| s.text_color(p.text))
            .child(t.label())
            .children(count(t).map(|n| {
                div()
                    .text_size(text::XS)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(if on { p.text_subtle } else { p.text_faint })
                    .child(n.to_string())
            }))
            .when(on, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(-1.))
                        .h(px(2.))
                        .rounded(px(2.))
                        .bg(p.text_strong),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_tab(t, cx)))
    });
    div()
        .flex_none()
        .h(px(BAR_H))
        .border_b_1()
        .border_color(p.hairline)
        .bg(p.bg)
        .child(
            div()
                .w_full()
                .max_w(px(1280.))
                .mx_auto()
                .px(space::S7)
                .h_full()
                .flex()
                .items_center()
                .gap(space::S3)
                .child(
                    div()
                        .id("insights-tabs")
                        .role(Role::TabList)
                        .flex()
                        .h_full()
                        .gap(space::S5)
                        .children(tabs),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S3)
                        .children(controls(this, p, window, cx)),
                ),
        )
}

fn controls(
    this: &Insights,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> Vec<AnyElement> {
    match this.tab {
        Tab::Overview => super::overview::controls(this, p, window, cx),
        Tab::Journal => super::journal::controls(this, p, cx),
        Tab::Subjects => super::subjects::controls(this, p, cx),
        _ => Vec::new(),
    }
}

/// The engine notice (pitch / off / unreachable / fix / derivation-off).
pub fn notice(this: &Insights, p: &Palette, cx: &mut Context<Insights>) -> Option<AnyElement> {
    let engine = engine_state(&this.status);
    let tab = this.tab;
    let per = |o: &'static str, j: &'static str, s: &'static str, c: &'static str| -> String {
        match tab {
            Tab::Overview => o,
            Tab::Journal => j,
            Tab::Subjects => s,
            Tab::Context => c,
        }
        .into()
    };
    let derivation_off = engine == Engine::On
        && this.status.ctx.as_ref().and_then(|c| c.reason.as_deref())
            == Some("user_context_disabled");
    // shortcut: "Set up AI" / "Turn on in Settings →" / "Reconnect" open Settings
    // through the shell once S7 routes Settings deep links (the button is inert).
    let (tone, icon, title, body, action, ghost): (Tone, SharedString, String, String, &str, bool) = match &engine {
        Engine::Pitch if tab != Tab::Journal => (
            Tone::Neutral,
            IconName::Sparkle.path(),
            "AI isn't set up yet".into(),
            per(
                "Connect a provider — local (Ollama, Llamafile) or your own API key — to get the read, categories and focus.",
                "",
                "Subjects are views your AI provider forms from your activity.",
                "You can write context now — it's saved and used once AI is set up.",
            ),
            "Set up AI",
            false,
        ),
        Engine::Off => (
            Tone::Off,
            IconName::CircleOff.path(),
            "AI features are off".into(),
            per(
                "You're seeing capture-only numbers. The read, categories and focus come back when AI features are on.",
                "Nothing new is written up while they're off. Days already written stay as they are.",
                "Subjects stop updating while AI features are off. Nothing here is lost.",
                "What you write is saved, and steers the dossier again once AI features are back on.",
            ),
            "Turn on in Settings →",
            true,
        ),
        Engine::Unreachable(text) => (
            Tone::Warn,
            IconName::CloudOff.path(),
            text.split(" — ").next().unwrap_or(text).to_string(),
            "New write-ups resume on their own once it's reachable. Everything already here stays.".into(),
            "Try again",
            false,
        ),
        Engine::Fix(text, id) => (
            Tone::Danger,
            IconName::TriangleAlert.path(),
            match id {
                Some(id) => format!(
                    "{} needs you to sign in again",
                    crate::chat::format::provider_label_by_id(
                        this.status.settings.as_ref().map_or(&[][..], |s| &s.ai_runtime.providers),
                        id
                    )
                ),
                None => text.clone(),
            },
            if id.is_some() {
                "OpenAI rejected the saved sign-in, so new write-ups are paused. Everything already here stays."
            } else {
                "New write-ups are paused. Everything already here stays."
            }
            .into(),
            if id.is_some() { "Reconnect" } else { "Open settings" },
            false,
        ),
        _ if derivation_off => (
            Tone::Off,
            IconName::CircleOff.path(),
            "Continuous derivation is off".into(),
            per(
                "No new activities, read or focus while it's off — the numbers below are capture-only.",
                "New capture isn't written up while it's off. Days already written stay as they are.",
                "No new evidence reaches your subjects while it's off. Confidence keeps fading on its own.",
                "What you write is saved, and steers the dossier once derivation is back on.",
            ),
            "Turn on in Settings →",
            true,
        ),
        _ => return None,
    };
    let retry = matches!(engine, Engine::Unreachable(_));
    let mut btn = Button::new("insights-notice-act", action.to_string()).sm();
    if ghost {
        btn = btn.ghost();
    }
    if retry {
        btn = btn
            .icon(IconName::RotateCcw.path())
            .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));
    }
    Some(
        div()
            .id("insights-notice")
            .role(Role::Status)
            .mb(space::S5)
            .child(notice_box(
                tone,
                icon,
                title,
                Some(body.into()),
                vec![btn.into_any_element()],
                p,
            ))
            .into_any_element(),
    )
}
