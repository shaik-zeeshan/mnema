//! The modal (insights.html `.in-modal`): a 62% scrim over the window and a
//! centred raised panel (radius 16, ≤640 px) with a title + close head, rising
//! 10 px over 420 ms expo while the scrim fades 220 ms. Esc and the scrim close.
//! The CSS scrim blur(4px) is dropped (GPUI can't blur in-window).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    deferred, div, px, Animation, AnyElement, App, ElementId, Pixels, SharedString, Window,
};

use super::{alpha, popover_shadow, Button};
use crate::theme::{self, motion, radius, size, space, text};

type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Modal {
    id: ElementId,
    title: SharedString,
    lead: Option<AnyElement>,
    body: Vec<AnyElement>,
    on_close: Option<OnClose>,
    max_w: Pixels,
}

impl Modal {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            lead: None,
            body: Vec::new(),
            on_close: None,
            max_w: size::MODAL_MAX_W,
        }
    }
    /// A narrower panel (default `size::MODAL_MAX_W`).
    pub fn max_w(mut self, w: Pixels) -> Self {
        self.max_w = w;
        self
    }
    /// A swatch / glyph before the title.
    pub fn lead(mut self, el: impl IntoElement) -> Self {
        self.lead = Some(el.into_any_element());
        self
    }
    pub fn child(mut self, el: impl IntoElement) -> Self {
        self.body.push(el.into_any_element());
        self
    }
    pub fn on_close(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let close = self.on_close.clone();
        let (by_scrim, by_key) = (close.clone(), close.clone());
        let fade = motion::MED.as_secs_f32() / motion::SLOW.as_secs_f32();
        let panel = div()
            .id("modal-panel")
            .relative()
            .w_full()
            .max_w(self.max_w)
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.border_strong)
            .rounded(radius::XL)
            .shadow(popover_shadow(&p))
            .overflow_hidden()
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(space::S4)
                    .px(space::S5)
                    .pt(space::S5)
                    .pb(space::S4)
                    .border_b_1()
                    .border_color(p.hairline)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap(px(10.))
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::XL)
                            .line_height(gpui_kit::relative(1.25))
                            .text_color(p.text_strong)
                            .children(self.lead)
                            .child(self.title.clone()),
                    )
                    .child(
                        Button::icon_only("modal-close", IconName::X.path(), "Close")
                            .ghost()
                            .sm()
                            .when_some(close, |b, f| {
                                b.on_click(move |_, window, cx| f(window, cx))
                            }),
                    ),
            )
            .children(self.body)
            .with_animation(
                "modal-rise",
                Animation::new(motion::SLOW).with_easing(motion::expo),
                |el, t| el.top(px(10.) * (1. - t)),
            );
        deferred(
            div()
                .id(self.id)
                .test_support()
                .role(gpui_kit::Role::Dialog)
                .aria_label(self.title)
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p(space::S6)
                .bg(alpha(p.bg, 0.62))
                .occlude()
                .font_family(theme::FONT_UI)
                .text_size(text::MD)
                .text_color(p.text)
                .when_some(by_scrim, |el, f| {
                    el.on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                        f(window, cx)
                    })
                })
                .when_some(by_key, |el, f| {
                    el.on_key_down(move |e, window, cx| {
                        if e.keystroke.key == "escape" {
                            f(window, cx)
                        }
                    })
                })
                .child(panel)
                .with_animation(
                    "modal-fade",
                    Animation::new(motion::SLOW).with_easing(motion::quart),
                    move |el, t| el.opacity((t / fade).min(1.)),
                ),
        )
        .with_priority(2)
    }
}
