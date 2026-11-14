//! The modal (`.cat-modal` / `.in-modal`): a scrim over the whole window
//! (`position: fixed; inset: 0`, so the titlebar and status bar too, which it
//! also keeps from taking clicks and hover) and a centred raised panel
//! (radius 16, or `.cat-modal`'s 18 with an eyebrow head, ≤640 px) with a
//! title + close head, rising 10 px over 420 ms
//! expo while the scrim fades 220 ms. Esc and the scrim close. The scrim is
//! `--app-overlay-bg` unless [`Modal::scrim`] says otherwise; the CSS blur is
//! dropped (GPUI can't blur in-window).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    anchored, deferred, div, px, Animation, AnyElement, App, ElementId, Pixels, SharedString,
    Window,
};

use super::{popover_shadow, Button};
use crate::theme::{self, motion, radius, size, space, text};
use crate::ui::Tracking as _;

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
    /// A scrim other than `--app-overlay-bg` (`.in-modal`'s 62% `bg`).
    pub fn scrim(mut self, c: gpui_kit::Hsla) -> Self {
        self.scrim = Some(c);
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let close = self.on_close.clone();
        let (by_scrim, by_key) = (close.clone(), close.clone());
        let fade = motion::MED.as_secs_f32() / motion::SLOW.as_secs_f32();
        let cat = self.eyebrow.is_some();
        let close_button = if cat {
            cat_close(close, &p, window, cx).into_any_element()
        } else {
            Button::icon_only("modal-close", IconName::X.path(), "Close")
                .ghost()
                .sm()
                .when_some(close, |b, f| b.on_click(move |_, window, cx| f(window, cx)))
                .into_any_element()
        };
        // `.cat-modal__panel`: 18 px, where `.in-modal` keeps `--r-xl`.
        let corner_r = if cat { px(18.) } else { self.radius };
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
                // `.cat-modal__header h2` / `.in-modal__head .mx-h2`.
                .tracking(if eyebrow { -0.01 } else { -0.018 })
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
                            .tracking(0.07)
                            .text_color(p.text_muted)
                            .child(SharedString::from(e.to_uppercase())),
                    )
                    .child(title),
                None => title,
            };
            // `.cat-modal__header`: 18 / 18 / 12; `.in-modal__head`: 24 / 24 / 16.
            let head = div()
                .flex()
                .items_start()
                .gap(space::S4)
                .when(eyebrow, |d| d.px(px(18.)).pt(px(18.)).pb(space::S3))
                .when(!eyebrow, |d| d.px(space::S5).pt(space::S5))
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
            .rounded(corner_r)
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
        // Pinned to the window's origin at the window's size, wherever the
        // surface that opens it sits.
        deferred(
            anchored().position(gpui_kit::point(px(0.), px(0.))).child(
                div()
                    .id(self.id)
                    .test_support()
                    .role(gpui_kit::Role::Dialog)
                    .aria_label(self.title)
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(space::S6)
                    .bg(self.scrim.unwrap_or(p.overlay_bg))
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
            ),
        )
        .with_priority(super::popover::LAYER_MODAL)
    }
}

/// `.cat-modal__close`: 28 px, a 1 px border, 8 px corner, muted ink; hover and
/// keyboard focus fill it and strengthen border and ink, focus adds the
/// `--app-ring`; 1 px down while pressed.
fn cat_close(
    close: Option<OnClose>,
    p: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let focus = super::focus_handle("modal-close", window, cx);
    let ring = super::focus_visible(&focus, super::Force::None, window);
    let lit = (p.surface_hover, p.border_hover, p.text_strong);
    div()
        .id("modal-close")
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label("Close")
        .track_focus(&focus)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(radius::MD)
        .border_1()
        .map(|el| match ring {
            true => el
                .bg(lit.0)
                .border_color(lit.1)
                .text_color(lit.2)
                .shadow(vec![super::ring(p.accent_glow)]),
            false => el.border_color(p.border).text_color(p.text_muted),
        })
        .cursor_pointer()
        .hover(move |s| s.bg(lit.0).border_color(lit.1).text_color(lit.2))
        .active(|s| s.top(px(1.)))
        .child(super::icon(IconName::X.path(), size::ICON_SM))
        .when_some(close, |el, f| {
            el.on_click(move |_, window, cx| f(window, cx))
        })
}
