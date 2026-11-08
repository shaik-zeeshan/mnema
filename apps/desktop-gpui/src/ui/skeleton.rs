//! `.mx-skel*`: the shape of what's coming, never a spinner in a big area.
//! Static (no shimmer). Wrap the region with `role(Role::Status)` + a label.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, Div};

use crate::theme::{radius, space, Palette};

/// One block; size it.
pub fn skel(p: &Palette) -> Div {
    div().rounded(radius::XS).bg(p.surface_hover).min_h(px(10.))
}

/// A 10 px text line at `w` (fraction of the row).
pub fn line(w: f32, p: &Palette) -> Div {
    skel(p).h(px(10.)).min_h_0().rounded(px(3.)).w(relative(w))
}

/// An 18 px title bar, 40% wide.
pub fn title(p: &Palette) -> Div {
    skel(p).h(px(18.)).w(relative(0.4))
}

/// A 16:10 thumbnail filling its column.
pub fn thumb(p: &Palette) -> Div {
    skel(p).w_full().aspect_ratio(16. / 10.).rounded(radius::MD)
}

pub fn circle(p: &Palette) -> Div {
    skel(p).flex_none().size(px(28.)).rounded_full()
}

/// `.mx-skel-lines`: a paragraph of `n` lines (every 3rd+2 at 88%, last 56%).
pub fn lines(n: usize, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .children((0..n).map(|i| {
            let w = if i + 1 == n {
                0.56
            } else if i % 3 == 1 {
                0.88
            } else {
                1.
            };
            line(w, p)
        }))
}

/// `.mx-skel-row`: lead (56×36 thumb, or a circle) + two lines + a 44 px meta.
pub fn row(lead_circle: bool, first_w: f32, top_rule: bool, p: &Palette) -> Div {
    let lead = if lead_circle {
        circle(p)
    } else {
        skel(p)
            .flex_none()
            .w(px(56.))
            .h(px(36.))
            .rounded(radius::SM)
    };
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .py(px(10.))
        .when(top_rule, |d| d.border_t_1().border_color(p.hairline))
        .child(lead)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(px(8.))
                .child(line(first_w, p))
                .child(line(0.56, p)),
        )
        .child(skel(p).flex_none().w(px(44.)).h(px(10.)).min_h_0())
}

/// `.mx-skel-chart`: `n` bars, the kit's height pattern, hairline baseline.
pub fn chart(n: usize, p: &Palette) -> Div {
    div()
        .flex()
        .items_end()
        .gap(px(6.))
        .h(px(120.))
        .pb(px(1.))
        .border_b_1()
        .border_color(p.hairline)
        .children((1..=n).map(|i| {
            // :nth-child(5n) beats 4n+2 beats 4n+1 (later rule wins on ties).
            let h = if i % 5 == 0 {
                0.86
            } else if i % 4 == 2 {
                0.36
            } else if i % 4 == 1 {
                0.70
            } else {
                0.48
            };
            skel(p)
                .flex_1()
                .min_h_0()
                .h(relative(h))
                .rounded_t(px(3.))
                .rounded_b(px(0.))
        }))
}
