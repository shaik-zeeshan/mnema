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
    eyebrow: Option<SharedString>,
    lead: Option<AnyElement>,
    body: Vec<AnyElement>,
    on_close: Option<OnClose>,
    max_w: Pixels,
    bare: bool,
    unpainted: bool,
    /// No head, no corner ×, no fill: the body is the whole panel (its own head).
    chromeless: bool,
    radius: Pixels,
    scrim: Option<gpui_kit::Hsla>,
    whole_window: bool,
}

impl Modal {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            eyebrow: None,
            lead: None,
            body: Vec::new(),
            on_close: None,
            max_w: size::MODAL_MAX_W,
            bare: false,
            unpainted: false,
            chromeless: false,
            radius: radius::XL,
            scrim: None,
            whole_window: false,
        }
    }
    /// A narrower panel (default `size::MODAL_MAX_W`).
    pub fn max_w(mut self, w: Pixels) -> Self {
        self.max_w = w;
        self
    }
    /// The `.cat-modal` head (Settings' connector modals): a small uppercase
    /// line over a 16 px title, no rule under it.
    pub fn eyebrow(mut self, text: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(text.into());
        self
    }
    /// A swatch / glyph before the title.
    pub fn lead(mut self, el: impl IntoElement) -> Self {
        self.lead = Some(el.into_any_element());
        self
    }
    /// No title row: the body owns the whole panel (the title still names the
    /// dialog for VoiceOver) and the × floats in the top-right corner.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }
    /// No fill or border: the body draws its own surface (shaped paper). The
    /// rise, the rounding and the drop shadow stay.
    pub fn unpainted(mut self) -> Self {
        self.unpainted = true;
        self
    }
    /// The body draws everything, head and close included (`FrameDetailModal`'s
    /// own header row); the panel keeps the rise, the shadow and `radius`.
    pub fn chromeless(mut self, radius: Pixels) -> Self {
        self.chromeless = true;
        self.unpainted = true;
        self.radius = radius;
        self
    }
    /// A scrim other than the shared 62% `bg` (a surface's own overlay token).
    pub fn scrim(mut self, c: gpui_kit::Hsla) -> Self {
        self.scrim = Some(c);
        self
    }
    /// Scrim the whole window, titlebar and status bar included (a CSS
    /// `position: fixed; inset: 0` modal), from a surface laid out between them.
    pub fn whole_window(mut self) -> Self {
        self.whole_window = true;
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
        let close_button = Button::icon_only("modal-close", IconName::X.path(), "Close")
            .ghost()
            .sm()
            .when_some(close, |b, f| b.on_click(move |_, window, cx| f(window, cx)));
        let (head, corner) = if self.chromeless {
            (None, None)
        } else if self.bare {
            let corner = div()
                .absolute()
                .top(space::S3)
                .right(space::S3)
                .child(close_button);
            (None, Some(corner))
        } else {
            let eyebrow = self.eyebrow.is_some();
            let title = div()
                .flex()
                .flex_1()
                .items_center()
                .gap(px(10.))
                .font_weight(theme::W_SEMIBOLD)
                .text_size(if eyebrow { text::LG } else { text::XL })
                .line_height(gpui_kit::relative(1.25))
                .text_color(p.text_strong)
                .children(self.lead)
                .child(self.title.clone());
            let title = match self.eyebrow {
                Some(e) => div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(text::SM)
                            .text_color(p.text_muted)
                            .child(SharedString::from(e.to_uppercase())),
                    )
                    .child(title),
                None => title,
            };
            let head = div()
                .flex()
                .items_start()
                .gap(space::S4)
                .px(space::S5)
                .pt(space::S5)
                .when(eyebrow, |d| d.pb(space::S3))
                .when(!eyebrow, |d| {
                    d.pb(space::S4).border_b_1().border_color(p.hairline)
                })
                .child(title)
                .child(close_button);
            (Some(head), None)
        };
        let panel = div()
            .id("modal-panel")
            .relative()
            .w_full()
            .max_w(self.max_w)
            .when(!self.unpainted, |el| {
                el.bg(p.surface_raised)
                    .border_1()
                    .border_color(p.border_strong)
            })
            .rounded(self.radius)
            .shadow(popover_shadow(&p))
            .overflow_hidden()
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .children(head)
            .children(self.body)
            .children(corner)
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
                .when(self.whole_window, |el| {
                    el.top(-size::TITLEBAR).bottom(-size::STATUS_BAR)
                })
                .flex()
                .items_center()
                .justify_center()
                .p(space::S6)
                .bg(self.scrim.unwrap_or(alpha(p.bg, 0.62)))
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
        .with_priority(super::popover::LAYER_MODAL)
    }
}
