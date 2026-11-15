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
use gpui_kit::{div, font, px, AnyElement, App, Div, Hsla, Pixels, SharedString};

use crate::insights::stats::{clock_range, fmt_duration, focus_color, focus_label, hour_label, HeatRow};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO, FONT_UI};
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
        .children(rows.iter().enumerate().map(|(ri, r)| {
            div()
                .flex()
                .gap(px(3.))
                .child(label(r.label.clone().into(), p.text_subtle))
                .children(r.cells.iter().enumerate().map(|(i, c)| {
                    let cell = div().flex_1().h(px(20.)).rounded(px(3.));
                    match c {
                        Some((band, minutes)) => {
                            let cell = cell.bg(mix(
                                focus_color(*band, p),
                                p.surface,
                                if *minutes >= 60. { 0.88 } else { 0.42 },
                            ));
                            // Cell i covers [2i, 2i+2) local hours: read off the UTC epoch day.
                            let hour = |k: usize| k as i64 * 2 * 3_600_000;
                            let slot = clock_range(hour(i), hour(i + 1), time::UtcOffset::UTC, "–");
                            let tip = format!(
                                "{} · {} · {} {slot}",
                                focus_label(*band),
                                fmt_duration((*minutes * 60_000.) as i64),
                                r.tip
                            );
                            tipped(crate::ui::nid("heat", (ri * 12 + i) as u64), cell, tip)
                        }
                        None => cell.bg(p.wash).into_any_element(),
                    }
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
            parts.iter().enumerate().filter(|(_, (_, n))| *n > 0).map(|(k, (f, n))| {
                let band = div()
                    .flex_grow(*n as f32)
                    .flex_basis(px(0.))
                    .rounded(px(2.))
                    .bg(focus_color(*f, p));
                let tip = format!(
                    "{} · {}% · {n} of {} activities",
                    focus_label(*f),
                    pct(*n),
                    c.n
                );
                tipped(crate::ui::nid("focus-band", k as u64), band, tip)
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
                        .child(focus_label(*f))
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

/// A hand-rolled `.mx-btn.mx-btn--sm` with a lead swatch (the kit `Button` has
/// no lead slot): `border_strong` in every state, hover changes the fill only,
/// `on` (`aria-pressed`) = the kit's selected fill; `live` = false while `:disabled`
/// (no pointer, so no hover).
pub fn swatch_chip(
    id: gpui_kit::ElementId,
    label: impl IntoElement,
    c: Hsla,
    on: bool,
    live: bool,
    p: &Palette,
) -> gpui_kit::Stateful<Div> {
    use crate::theme::{size, W_MEDIUM};
    div()
        .id(id)
        .role(gpui_kit::Role::Button)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(size::H_SM)
        .px(px(9.))
        .rounded(radius::BTN_SM)
        .border_1()
        .border_color(p.border_strong)
        .bg(if on { p.wash_strong } else { p.surface_raised })
        .text_size(text::BASE)
        .font_weight(W_MEDIUM)
        .text_color(p.text_strong)
        .child(swatch(c, 8.))
        .child(label)
        .when(live && !on, |d| d.hover(|s| s.bg(p.surface_hover)))
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
/// The name and value columns are shared (CSS grid `max-content` / `auto`), so
/// every bar starts and ends at one x. `name_w` fixes the name column
/// (`.in-apps`' 82 px); `None` sizes it to the widest name (`.apps`).
pub fn spec_rows(
    rows: Vec<(SharedString, f32, SharedString)>,
    color: Hsla,
    name_w: Option<Pixels>,
    p: &Palette,
    cx: &App,
) -> Div {
    // Spline Sans Mono: every glyph has the advance of `0`.
    let ts = cx.text_system();
    let ch = ts
        .ch_advance(ts.resolve_font(&font(FONT_MONO)), text::SM)
        .unwrap_or_default();
    let chars = rows.iter().map(|r| r.2.chars().count()).max().unwrap_or(0);
    let val_w = (ch * chars as f32).ceil();
    // `.mx-spec dt` pads its column S5 on the right.
    let name_w = name_w.unwrap_or_else(|| {
        // shortcut: sums glyph advances (no kerning), so allow a pixel of slack.
        let ui = ts.resolve_font(&font(FONT_UI));
        let widest = rows
            .iter()
            .map(|r| {
                r.0.chars()
                    .map(|c| ts.advance(ui, text::BASE, c).map_or(px(0.), |a| a.width))
                    .fold(px(0.), |a, b| a + b)
            })
            .fold(px(0.), |a, b| a.max(b));
        (widest + px(1.)).ceil() + space::S5
    });
    div()
        .flex()
        .flex_col()
        .children(rows.into_iter().enumerate().map(|(i, (name, w, val))| {
            // `.mx-spec > div:first-child > * { border-top: 0 }`.
            div()
                .flex()
                .items_center()
                .py(px(10.))
                .when(i > 0, |d| d.border_t_1())
                .border_color(p.hairline)
                .text_size(text::BASE)
                .child(with_tip(
                    // Gives way before the bar's 48 px floor does (`minmax(0, max-content)`).
                    div()
                        .id(SharedString::from(format!("spec-name-{i}-{name}")))
                        .min_w_0()
                        .w(name_w)
                        .pr(space::S5)
                        .truncate()
                        .text_color(p.text)
                        .child(name.clone()),
                    name,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(48.))
                        .flex()
                        .child(bar(w, color, 4., p)),
                )
                .child(
                    // `.mx-spec__note`: S4 clear of the bar.
                    div()
                        .flex_none()
                        .pl(space::S4)
                        .min_w(val_w + space::S4)
                        .text_right()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(val),
                )
        }))
}

/// CSS `repeating-linear-gradient(135deg, line 0 w, bg w period)`: `/`
/// stripes `w` px thick every `period` px over `bg`, clipped to the box.
pub fn hatch(line: Hsla, bg: Hsla, w: f32, period: f32) -> gpui_kit::Canvas<()> {
    gpui_kit::canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(gpui_kit::fill(b, bg));
            window.with_content_mask(Some(gpui_kit::ContentMask { bounds: b }), |window| {
                let (x0, y0) = (f32::from(b.left()), f32::from(b.top()));
                let h = f32::from(b.size.height);
                let span = f32::from(b.size.width) + h;
                // A stripe centred `d` along the gradient is the line x + y = d·√2.
                let mut d = w / 2.;
                while d * std::f32::consts::SQRT_2 < span + w {
                    let k = d * std::f32::consts::SQRT_2;
                    let mut s = gpui_kit::PathBuilder::stroke(px(w));
                    s.move_to(gpui_kit::point(px(x0 + k + w), px(y0 - w)));
                    s.line_to(gpui_kit::point(px(x0 + k - h - w), px(y0 + h + w)));
                    if let Ok(path) = s.build() {
                        window.paint_path(path, line);
                    }
                    d += period;
                }
            });
        },
    )
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
