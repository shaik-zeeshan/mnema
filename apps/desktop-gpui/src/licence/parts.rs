//! What both receipts share: the paper they are printed on (two halves meeting
//! at a perforated tear line with real semicircular bites cut from the edges),
//! the inputs `view.rs` hands them, and the small type helpers.
//!
//! The bites are cut, not painted: each half paints its own outline as a path
//! whose seam corners are concave quarter circles, and the modal paints no fill
//! or border of its own (`Modal::unpainted`). So a bite shows whatever really
//! sits behind the receipt (the scrim over the Timeline's frame), in any theme.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, point, px, AnyElement, Bounds, Div, Hsla, PathBuilder, Pixels, Role, SharedString,
    Stateful, Window,
};
use time::OffsetDateTime;

use super::receipt::Copy;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};

/// The ruler's whole celebration (hold, scrub, land): once, under ~1.4 s.
pub const CELEBRATE: Duration = Duration::from_millis(1400);

/// `receipt::copy`'s over-cap line says "its 3 devices"; the slots draw the same number.
pub const DEVICE_CAP: usize = 3;

/// A bite's radius.
pub const NOTCH: f32 = 9.;

/// What a receipt face gets from `view.rs`: its copy, its built buttons (ids,
/// roles and handlers unchanged), the reset note, and the frozen now.
pub struct Parts {
    pub copy: Copy,
    pub buttons: Vec<AnyElement>,
    pub note: Option<AnyElement>,
    pub now: OffsetDateTime,
}

/// The paper: `top` over `bottom`, the tear line between them. `top_fill` lets
/// the ruler's band sit a shade darker than the rest of the slip.
pub fn paper(top: impl IntoElement, top_fill: Hsla, bottom: impl IntoElement, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .child(half(true, top_fill, p).child(top))
        .child(half(false, p.surface_raised, p).child(bottom))
}

fn half(top: bool, fill: Hsla, p: &Palette) -> Div {
    let line = p.border_strong;
    div().relative().child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| paint_half(top, b, fill, line, window),
        )
        .absolute()
        .inset_0(),
    )
}

/// One half's fill, its outline (open along the seam), and, on the top half,
/// the dashed tear line.
fn paint_half(top: bool, b: Bounds<Pixels>, fill: Hsla, line: Hsla, window: &mut Window) {
    let r = f32::from(radius::XL) - 0.5;
    let n = NOTCH;
    let x0 = f32::from(b.left()) + 0.5;
    let x1 = f32::from(b.right()) - 0.5;
    // The seam is exact (the halves meet there); the outer edge is inset half a stroke.
    let (seam, edge) = if top {
        (f32::from(b.bottom()), f32::from(b.top()) + 0.5)
    } else {
        (f32::from(b.top()), f32::from(b.bottom()) - 0.5)
    };
    // `d` points from the seam towards the outer edge.
    let d = if top { -1. } else { 1. };
    let at = |x: f32, y: f32| point(px(x), px(y));
    let arc = |pb: &mut PathBuilder, rad: f32, sweep: bool, x: f32, y: f32| {
        pb.arc_to(at(rad, rad), px(0.), false, sweep, at(x, y));
    };
    // Convex outer corners sweep clockwise going round the top half; the
    // bottom half runs the other way, so every flag flips with `top`.
    let outline = |pb: &mut PathBuilder| {
        pb.move_to(at(x0 + n, seam));
        arc(pb, n, !top, x0, seam + d * n);
        pb.line_to(at(x0, edge - d * r));
        arc(pb, r, top, x0 + r, edge);
        pb.line_to(at(x1 - r, edge));
        arc(pb, r, top, x1, edge - d * r);
        pb.line_to(at(x1, seam + d * n));
        arc(pb, n, !top, x1 - n, seam);
    };
    let mut body = PathBuilder::fill();
    outline(&mut body);
    body.close();
    if let Ok(path) = body.build() {
        window.paint_path(path, fill);
    }
    let mut rim = PathBuilder::stroke(px(1.));
    outline(&mut rim);
    if let Ok(path) = rim.build() {
        window.paint_path(path, line);
    }
    if top {
        let inset = f32::from(space::S5);
        let mut tear = PathBuilder::stroke(px(1.)).dash_array(&[px(3.), px(3.)]);
        tear.move_to(at(x0 + inset, seam - 0.5));
        tear.line_to(at(x1 - inset, seam - 0.5));
        if let Ok(path) = tear.build() {
            window.paint_path(path, line);
        }
    }
}

/// A licence instant as a date, or `None` for "never" (`i64::MAX`).
pub fn date(ms: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000).ok()
}

/// "AUG 31 2027": the mono readout form.
pub fn stamp_date(t: OffsetDateTime) -> SharedString {
    format!("{} {:02} {}", short_month(t), t.day(), t.year()).into()
}

/// "AUG".
pub fn short_month(t: OffsetDateTime) -> String {
    t.month().to_string()[..3].to_uppercase()
}

/// The body every face wraps itself in (VoiceOver reads it as the receipt).
pub fn body(title: &'static str) -> Stateful<Div> {
    div()
        .id("receipt-body")
        .role(Role::Group)
        .aria_label(title)
        .relative()
        .flex()
        .flex_col()
}

/// The buttons, right-aligned, and the reset note under them.
pub fn actions(buttons: Vec<AnyElement>, note: Option<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(space::S2)
                .children(buttons),
        )
        .children(note.map(|n| div().text_size(text::BASE).child(n)))
}

/// A mono label: `UPDATES THROUGH`.
pub fn label(s: &str, p: &Palette) -> Div {
    div()
        .font_family(FONT_MONO)
        .font_weight(theme::W_MEDIUM)
        .text_size(text::XS)
        .text_color(p.text_subtle)
        .child(SharedString::from(s.to_uppercase()))
}

pub fn headline(s: &'static str, size: Pixels, p: &Palette) -> Div {
    div()
        .font_weight(theme::W_SEMIBOLD)
        .text_size(size)
        .line_height(gpui_kit::relative(1.15))
        .text_color(p.text_strong)
        .child(s)
}

/// A line of body copy.
pub fn line(s: String, size: Pixels, color: Hsla) -> Div {
    div()
        .text_size(size)
        .line_height(gpui_kit::relative(1.55))
        .text_color(color)
        .child(SharedString::from(s))
}
