//! Insights' charts (`lib/insights/charts/*`), drawn with quads and GPUI paths
//! so any surface can reuse them: `stacked_bar` (Where the time went),
//! `heatmap` (focus by time of day), `sparkline` (confidence micro-trend),
//! `line_chart` (the subject's confidence over time, `SubjectChart` + the
//! `Timeline` geometry), `mini_bars` (ranked rows), `confidence_bar` and
//! `timeline` (an answer's time-of-day rail).

mod lines;
mod stacked;
pub mod timeline;

pub use lines::{line_chart, sparkline, Line};
pub use stacked::stacked_bar;
pub use timeline::timeline;

use capture_types::FocusLevel;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Div, Hsla, SharedString};

use crate::insights::stats::{focus_color, hour_label, HeatRow};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::{mix, with_tip};

/// `Heatmap.svelte`: 30 px row labels, 12 two-hour cells 20 px tall, 3 px gaps.
/// A cell is the band's colour, strong past an hour, faint under it.
pub fn heatmap(rows: &[HeatRow], p: &Palette) -> Div {
    let label = |s: SharedString, c: Hsla| {
        div()
            .w(px(30.))
            .flex_none()
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .line_height(px(16.))
            .text_color(c)
            .child(s)
    };
    let head = div()
        .flex()
        .gap(px(3.))
        .child(label("".into(), p.text_faint))
        .children((0..12).map(|i| {
            div()
                .flex_1()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .line_height(px(16.))
                .text_color(p.text_faint)
                .child(if i % 3 == 0 {
                    hour_label(i as u8 * 2)
                } else {
                    String::new()
                })
        }));
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .mt(space::S4)
        .child(head)
        .children(rows.iter().map(|r| {
            div()
                .flex()
                .gap(px(3.))
                .child(label(r.label.clone().into(), p.text_subtle))
                .children(r.cells.iter().map(|c| {
                    let bg = match c {
                        Some((band, minutes)) => mix(
                            focus_color(*band, p),
                            p.surface,
                            if *minutes >= 60. { 0.88 } else { 0.42 },
                        ),
                        None => p.wash,
                    };
                    div().flex_1().h(px(20.)).rounded(px(3.)).bg(bg)
                }))
        }))
}

/// `FocusSplit.svelte`: the deep / mixed / scattered bar with its legend.
pub fn focus_split(c: crate::insights::stats::FocusCounts, p: &Palette) -> Div {
    let parts = [
        (FocusLevel::Deep, c.deep),
        (FocusLevel::Mixed, c.mixed),
        (FocusLevel::Distracted, c.distracted),
    ];
    let pct = |n: usize| {
        if c.n > 0 {
            ((n as f64 / c.n as f64) * 100.).round() as i64
        } else {
            0
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .child(div().flex().gap(px(3.)).h(px(8.)).children(
            parts.iter().filter(|(_, n)| *n > 0).map(|(f, n)| {
                div()
                    .flex_grow(*n as f32)
                    .flex_basis(px(0.))
                    .rounded(px(2.))
                    .bg(focus_color(*f, p))
            }),
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S4)
                .text_size(text::SM)
                .text_color(p.text_muted)
                .children(parts.iter().map(|(f, n)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(swatch(focus_color(*f, p), 8.))
                        .child(crate::insights::stats::focus_label(*f))
                        .child(
                            div()
                                .font_family(FONT_MONO)
                                .text_color(p.text_strong)
                                .child(format!("{}%", pct(*n))),
                        )
                }))
                .child(div().flex_1())
                .child(div().text_color(p.text_subtle).child("of activities")),
        )
}

/// A 2 px-radius colour square (`.sw`).
pub fn swatch(c: Hsla, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded(px(2.)).bg(c)
}

/// One proportional bar in a track (`.bar i`): `w` is 0..=1.
pub fn bar(w: f32, c: Hsla, h: f32, p: &Palette) -> Div {
    div()
        .flex_1()
        .h(px(h))
        .rounded(px(h / 2.))
        .bg(p.wash)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(gpui_kit::relative(w.clamp(0., 1.)))
                .rounded(px(h / 2.))
                .bg(c),
        )
}

/// `MiniBars.svelte`: label · track · value, the top row in full accent and the
/// rest fading toward the track.
pub fn mini_bars(items: &[(SharedString, f64, SharedString)], p: &Palette) -> Div {
    let max = items.iter().map(|i| i.1).fold(0., f64::max);
    let steps = items.len().saturating_sub(1).max(1) as f32;
    div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .children(items.iter().enumerate().map(|(i, (label, v, sub))| {
            let fill = mix(p.accent, p.surface_hover, 1. - (i as f32 / steps) * 0.58);
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .text_size(text::BASE)
                .child(
                    div()
                        .w(px(120.))
                        .truncate()
                        .text_color(p.text)
                        .child(label.clone()),
                )
                .child(bar(
                    if max > 0. { (*v / max) as f32 } else { 0. },
                    fill,
                    6.,
                    p,
                ))
                .child(
                    div()
                        .w(px(56.))
                        .text_right()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_strong)
                        .child(sub.clone()),
                )
        }))
}

/// `ConfidenceBar.svelte`: a 0..1 bar with its percent; faded rows dim.
pub fn confidence_bar(confidence: f64, faded: bool, p: &Palette) -> Div {
    let c = confidence.clamp(0., 1.);
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .when(faded, |d| d.opacity(theme::DISABLED_OPACITY))
        .child(
            div()
                .w(px(64.))
                .flex()
                .child(bar(c as f32, p.text_strong, 4., p)),
        )
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_strong)
                .child(format!("{}%", (c * 100.).round())),
        )
}

/// A hairline-topped ranked row list (`.mx-spec` apps): name · bar · value.
pub fn spec_rows(rows: Vec<(SharedString, f32, SharedString)>, color: Hsla, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .children(rows.into_iter().map(|(name, w, val)| {
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .py(px(7.))
                .border_t_1()
                .border_color(p.hairline)
                .text_size(text::BASE)
                .child(
                    div()
                        .flex_none()
                        .w(px(120.))
                        .truncate()
                        .text_color(p.text)
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(48.))
                        .flex()
                        .child(bar(w, color, 4., p)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(val),
                )
        }))
}

/// Tip wrapper for a chart part with an id.
pub fn tipped(
    id: impl Into<gpui_kit::ElementId>,
    el: Div,
    tip: impl Into<SharedString>,
) -> AnyElement {
    with_tip(el.id(id), tip).into_any_element()
}

pub(crate) fn rounded_top(d: Div) -> Div {
    d.rounded_tl(radius::XS).rounded_tr(radius::XS)
}
