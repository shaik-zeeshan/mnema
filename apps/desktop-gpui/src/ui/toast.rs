//! `.mx-toast(s)`: the transient result of the user's own action, bottom-centre
//! above the status bar. ok = done (+ optional Undo, with its drain bar) ·
//! danger = failed (+ Retry). Rises 8 px over 220 ms expo; leaves after the TTL.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, Animation, App, Context, Div, SharedString, Window};

use super::{icon, nid, popover_shadow, Button, Tone};
use crate::theme::{self, motion, size, text, Palette, FONT_MONO, TOAST_TTL};

type OnAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
pub struct Toast {
    pub tone: Tone,
    pub text: SharedString,
    pub small: Option<SharedString>,
    pub action: Option<(SharedString, OnAction)>,
    /// Show the drain bar (an Undo toast).
    pub drain: bool,
    pub ttl: Duration,
}

impl Toast {
    pub fn ok(text: impl Into<SharedString>) -> Self {
        Self {
            tone: Tone::Ok,
            text: text.into(),
            small: None,
            action: None,
            drain: false,
            ttl: TOAST_TTL,
        }
    }
    pub fn danger(text: impl Into<SharedString>) -> Self {
        Self {
            tone: Tone::Danger,
            ..Self::ok(text)
        }
    }
    /// An Undo-style action; `drain` shows the bar counting down the TTL.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        drain: bool,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), Rc::new(f)));
        self.drain = drain;
        self
    }
    pub fn small(mut self, s: impl Into<SharedString>) -> Self {
        self.small = Some(s.into());
        self
    }
}

/// One toast box. `key` keys its rise and drain animations.
pub fn toast_box(t: &Toast, key: u64, p: &Palette) -> Div {
    let (tone, ..) = t.tone.colors(p);
    let glyph = match t.tone {
        Tone::Danger | Tone::Warn => IconName::TriangleAlert,
        Tone::Info => IconName::Info,
        _ => IconName::Check,
    };
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(size::TOAST_MIN_H)
        .max_w(size::TOAST_MAX_W)
        .pl(px(12.))
        .py(px(5.))
        .pr(if t.action.is_some() { px(5.) } else { px(14.) })
        .rounded(theme::radius::MD)
        .bg(p.overlay_bg_strong)
        .border_1()
        .border_color(p.overlay_border)
        .shadow(popover_shadow(p))
        // Clicks and hover on the toast stay off the rows under it.
        .occlude()
        .overflow_hidden()
        .font_family(theme::FONT_UI)
        .font_weight(theme::W_MEDIUM)
        .text_size(text::MD)
        .line_height(relative(1.35))
        .text_color(p.text_strong)
        .child(
            div()
                .flex_none()
                .text_color(tone)
                .child(icon(glyph.path(), px(15.))),
        )
        // Wraps inside the 520 px cap instead of pushing the action out.
        .child(div().min_w_0().child(t.text.clone()))
        .when_some(t.small.clone(), |d, s| {
            d.child(
                div()
                    .font_family(FONT_MONO)
                    .font_weight(theme::W_REGULAR)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(s),
            )
        })
        .when_some(t.action.clone(), |d, (label, f)| {
            d.child(
                Button::new(nid("toast-act", key), label)
                    .ghost()
                    .sm()
                    .on_click(move |_, window, cx| f(window, cx)),
            )
        })
        .when(t.drain, |d| {
            d.child(
                div()
                    .id(nid("toast-drain", key))
                    .test_support()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .h(px(2.))
                    .w_full()
                    .bg(tone)
                    .opacity(0.55)
                    .with_animation(
                        nid("toast-drain-anim", key),
                        Animation::new(t.ttl),
                        |el, t| el.w(relative(1. - t)),
                    ),
            )
        })
}

/// The window's toast stack. Hold one per window; `push` from anywhere.
#[derive(Default)]
pub struct Toasts {
    items: Vec<(u64, Toast)>,
    next: u64,
}

impl Toasts {
    pub fn push(&mut self, toast: Toast, cx: &mut Context<Self>) -> u64 {
        let key = self.next;
        self.next += 1;
        let ttl = toast.ttl;
        self.items.push((key, toast));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            this.update(cx, |this, cx| this.dismiss(key, cx)).ok();
        })
        .detach();
        cx.notify();
        key
    }

    pub fn dismiss(&mut self, key: u64, cx: &mut Context<Self>) {
        self.items.retain(|(k, _)| *k != key);
        cx.notify();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        div()
            .id("toasts")
            .absolute()
            .left_0()
            .right_0()
            .bottom(size::TOAST_BOTTOM)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .children(self.items.iter().map(|(key, t)| {
                toast_box(t, *key, &p)
                    .id(nid("toast", *key))
                    .test_support()
                    .role(if t.tone == Tone::Danger {
                        gpui_kit::Role::Alert
                    } else {
                        gpui_kit::Role::Status
                    })
                    .with_animation(
                        nid("toast-rise", *key),
                        Animation::new(motion::MED).with_easing(motion::expo),
                        move |el, t| el.top(px(8.) * (1. - t)).opacity(t),
                    )
            }))
    }
}
