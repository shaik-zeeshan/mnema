//! `StackedBar.svelte`: "Where the <unit> went" — a 210 px plot with a 34 px
//! axis (five ticks over a nice top), gridlines, one stacked column per slot
//! (2 px gaps, top segment rounded 4), dashed stubs for empty slots, a "live"
//! cap over today in week mode, and the day labels with totals below.

use capture_types::ActivityCategory;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, Div};

use crate::insights::stats::{axis_max, category_color, fmt_axis_tick, fmt_duration, Column};
use crate::theme::{radius, space, text, Palette, FONT_MONO};
use crate::ui::calendar::Unit;

const PLOT_H: f32 = 210.;
const AXIS_W: f32 = 34.;

/// `highlight`: the category row under the pointer dims the other segments.
pub fn stacked_bar(
    cols: &[Column],
    unit: Unit,
    highlight: Option<Option<ActivityCategory>>,
    p: &Palette,
) -> Div {
    let max = axis_max(cols.iter().map(|c| c.total).max().unwrap_or(0) as f64);
    let ticks = [max, max * 0.75, max / 2., max / 4., 0.];
    let dense = unit == Unit::Month || cols.len() > 12;
    let gap = if dense { px(4.) } else { space::S3 };
    let mono = |d: Div| d.font_family(FONT_MONO).text_size(text::XS);
    let axis = div()
        .relative()
        .w(px(AXIS_W))
        .flex_none()
        .h_full()
        .children(ticks.iter().map(|v| {
            mono(div())
                .absolute()
                .right_0()
                .bottom(gpui_kit::relative((*v / max) as f32))
                .mb(px(-5.))
                .text_color(p.text_faint)
                .child(fmt_axis_tick(*v))
        }));
    let grid = div()
        .absolute()
        .left(px(AXIS_W + 12.))
        .right_0()
        .top_0()
        .bottom_0()
        .children(ticks[..4].iter().map(|v| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .h(px(1.))
                .bottom(gpui_kit::relative((*v / max) as f32))
                .bg(p.hairline)
        }));
    let col = |c: &Column| {
        if c.total <= 0 {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .child(
                    div()
                        .h(px(24.))
                        .rounded(radius::XS)
                        .border_1()
                        .border_dashed()
                        .border_color(p.border)
                        .when(c.future && unit == Unit::Month, |d| d.opacity(0.4)),
                );
        }
        let h = (c.total as f64 / max) as f32;
        let n = c.parts.len();
        let segs = c.parts.iter().enumerate().map(|(i, (k, ms))| {
            let color = if c.app_time {
                p.chart_grey_3
            } else {
                category_color(*k, p)
            };
            let dim = highlight.is_some_and(|h| h != *k);
            let seg = div()
                .w_full()
                .flex_grow(*ms as f32 / 60_000.)
                .flex_basis(px(0.))
                .bg(color)
                .when(dim, |d| d.opacity(0.2));
            if i + 1 == n {
                super::rounded_top(seg)
            } else {
                seg
            }
        });
        let live = c.live && unit == Unit::Week;
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .justify_end()
            .child(
                div()
                    .relative()
                    .h(gpui_kit::relative(h))
                    .flex()
                    .flex_col_reverse()
                    .gap(px(2.))
                    .children(segs)
                    .when(live, |d| {
                        d.child(
                            mono(div())
                                .absolute()
                                .top(px(-18.))
                                .left_0()
                                .right_0()
                                .flex()
                                .justify_center()
                                .text_color(p.accent)
                                .child("live"),
                        )
                    }),
            )
    };
    let plot = div()
        .relative()
        .flex()
        .gap(gap)
        .h(px(PLOT_H))
        .mt(space::S4)
        .child(axis)
        .child(grid)
        .children(cols.iter().map(col));
    let days = div()
        .flex()
        .gap(gap)
        .mt(space::S2)
        .child(div().w(px(AXIS_W)).flex_none())
        .children(cols.iter().map(|c| {
            let total = if c.total > 0 {
                fmt_duration(c.total)
                    + if c.live && unit == Unit::Day {
                        " so far"
                    } else {
                        ""
                    }
            } else {
                "—".into()
            };
            mono(div())
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .line_height(gpui_kit::relative(1.3))
                .text_color(p.text_subtle)
                .child(
                    div()
                        .font_weight(crate::theme::W_SEMIBOLD)
                        .text_color(p.text)
                        .child(c.label.clone()),
                )
                .when(!dense, |d| d.child(total))
        }));
    div().flex().flex_col().child(plot).child(days)
}
