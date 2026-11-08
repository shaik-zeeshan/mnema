//! Work in flight: `.mx-spin` (always beside text, colour = current text colour)
//! and `.mx-progress` (determinate / indeterminate) with its meta row.

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::{
    div, px, radians, relative, svg, Animation, App, Div, Pixels, SharedString, Transformation,
    Window,
};

use super::alpha;
use crate::assets::{SPIN_ARC, SPIN_ARC_LG, SPIN_ARC_SM};
use crate::theme::{self, motion, size, text, Palette, FONT_MONO};

#[derive(IntoElement)]
pub struct Spinner {
    size: Pixels,
}

/// `size::SPIN_SM` (10, inside 22-26 px controls) · `SPIN` (14) · `SPIN_LG` (20).
pub fn spinner(size: Pixels) -> Spinner {
    Spinner { size }
}

impl RenderOnce for Spinner {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let color = window.text_style().color;
        let (arc, border) = if self.size >= size::SPIN_LG {
            (SPIN_ARC_LG, px(2.))
        } else if self.size <= size::SPIN_SM {
            (SPIN_ARC_SM, px(1.5))
        } else {
            (SPIN_ARC, px(1.5))
        };
        div()
            .relative()
            .flex_none()
            .size(self.size)
            .rounded_full()
            .border(border)
            .border_color(alpha(color, 0.22))
            .child(
                svg()
                    .path(arc)
                    .absolute()
                    .top(-border)
                    .left(-border)
                    .size(self.size)
                    .text_color(color)
                    .with_animation("spin", Animation::new(motion::SPIN).repeat(), |el, t| {
                        el.with_transformation(Transformation::rotate(radians(
                            t * std::f32::consts::TAU,
                        )))
                    }),
            )
    }
}

/// `.mx-progress`: `value` in 0..=1, or `None` for unknown length.
pub fn progress(value: Option<f32>, p: &Palette) -> Div {
    let bar = div()
        .relative()
        .w_full()
        .h(size::PROGRESS_H)
        .rounded(px(2.))
        .bg(p.wash_strong)
        .overflow_hidden();
    match value {
        Some(v) => bar.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(relative(v.clamp(0., 1.)))
                .rounded(px(2.))
                .bg(p.accent),
        ),
        // shortcut: the CSS -45° hatch (6 px accent / 6 px accent 35%) as upright
        // 6 px blocks; at 4 px tall the slant is ~invisible. Static: no sliding.
        None => bar.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .flex()
                .bg(alpha(p.accent, 0.35))
                .children(
                    (0..200).map(|_| div().flex_none().w(px(6.)).h_full().mr(px(6.)).bg(p.accent)),
                ),
        ),
    }
}

/// `.mx-progress__meta`: a sans label and mono numbers above the bar.
pub fn progress_meta(
    label: impl Into<SharedString>,
    numbers: impl Into<SharedString>,
    p: &Palette,
) -> Div {
    div()
        .flex()
        .justify_between()
        .gap(theme::space::S3)
        .mb(px(7.))
        .line_height(relative(1.))
        .child(
            div()
                .font_family(theme::FONT_UI)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::BASE)
                .text_color(p.text_strong)
                .child(label.into()),
        )
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_muted)
                .child(numbers.into()),
        )
}
