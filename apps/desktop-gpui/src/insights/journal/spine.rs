//! The day spine (`DaySpine.svelte`): the day's first→last span rounded out to
//! hours with gridlines; activities as category blocks in stacked lanes, dashed
//! "away" gaps, the pending edge, a focus strip under them, and today's now-line.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Div};
use time::UtcOffset;

use super::day::{spine_axis, stack_lanes, Card, DayModel};
use crate::insights::charts::swatch;
use crate::insights::parts::{mono_label, panel, sec};
use crate::insights::stats::{self, category_color, focus_color, humanize_ms};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::{alpha, kicker, mix};

const LANE_H: f32 = 44.;

pub fn render(
    model: &DayModel,
    cards: &[Card],
    now: Option<i64>,
    p: &Palette,
    off: UtcOffset,
) -> AnyElement {
    let spans: Vec<(i64, i64)> = cards.iter().map(|c| (c.start, c.end)).collect();
    let mut all: Vec<(i64, i64)> = spans.clone();
    all.extend(model.gaps.iter().map(|g| (g.start, g.end)));
    if let Some(s) = model.pending.since.filter(|_| model.pending.active) {
        all.push((s, s));
    }
    let first = all.iter().map(|s| s.0).min().unwrap_or(0);
    let last = all.iter().map(|s| s.1).chain(now).max().unwrap_or(first);
    let (a0, a1, ticks) = spine_axis(first, last, off);
    let x = |ms: i64| ((ms - a0) as f32 / (a1 - a0).max(1) as f32).clamp(0., 1.);
    let (lane, lanes) = stack_lanes(&spans);
    let lane_h = if lanes > 1 {
        LANE_H / lanes as f32
    } else {
        LANE_H
    };
    let block = |left: f32, w: f32| {
        div()
            .absolute()
            .left(relative(left))
            .w(relative(w))
            .mr(px(2.))
    };
    let blocks = cards.iter().enumerate().map(|(i, c)| {
        let a = &c.slot.activity;
        let w = x(c.end) - x(c.start);
        let col = category_color(a.category, p);
        block(x(c.start), w)
            .top(px(lane[i] as f32 * lane_h))
            .h(px(lane_h - 2.))
            .rounded(radius::XS)
            .border_t_2()
            .border_color(col)
            .bg(mix(col, p.surface, 0.18))
            .px(px(8.))
            .py(px(6.))
            .overflow_hidden()
            .when(w > 0.06, |d| {
                d.child(
                    div()
                        .truncate()
                        .text_size(text::BASE)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_strong)
                        .child(a.title.clone()),
                )
                .when(lanes == 1, |d| {
                    d.child(
                        div()
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_subtle)
                            .child(humanize_ms(c.end - c.start)),
                    )
                })
            })
    });
    let gaps = model.gaps.iter().map(|g| {
        let w = x(g.end) - x(g.start);
        block(x(g.start), w)
            .top_0()
            .h(px(LANE_H - 2.))
            .rounded(radius::XS)
            .border_1()
            .border_dashed()
            .border_color(p.border)
            .flex()
            .items_center()
            .justify_center()
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .text_color(p.text_faint)
            .when(w > 0.04, |d| d.child("away"))
    });
    let pending = model
        .pending
        .since
        .filter(|s| model.pending.active && now.is_some_and(|n| n > *s))
        .map(|s| {
            let w = x(now.unwrap_or(s)) - x(s);
            block(x(s), w)
                .top_0()
                .h(px(LANE_H - 2.))
                .rounded(radius::XS)
                .border_1()
                .border_dashed()
                .border_color(p.accent_border)
                .bg(alpha(p.accent, 0.06))
                .flex()
                .items_center()
                .justify_center()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .text_color(p.accent)
                .when(w > 0.06, |d| {
                    d.child(if model.pending.unavailable.is_some() {
                        "paused"
                    } else {
                        "writing up"
                    })
                })
        });
    let focus = cards.iter().filter_map(|c| {
        let f = c.slot.activity.focus?;
        Some(
            block(x(c.start), x(c.end) - x(c.start))
                .top_0()
                .h(px(4.))
                .rounded(px(2.))
                .bg(focus_color(f, p)),
        )
    });
    let grid = div().absolute().inset_0().children(ticks.iter().map(|t| {
        div()
            .absolute()
            .left(relative(x(*t)))
            .top_0()
            .bottom(px(-20.))
            .w(px(1.))
            .bg(p.hairline)
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left(px(-30.))
                    .w(px(60.))
                    .flex()
                    .justify_center()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_faint)
                    .child(stats::clock(*t, off)),
            )
    }));
    let now_line = now.map(|n| {
        div()
            .absolute()
            .left(relative(x(n)))
            .top(px(-6.))
            .bottom(px(14.))
            .w(px(2.))
            .bg(p.accent)
    });
    let legend = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .text_size(text::SM)
        .text_color(p.text_subtle)
        .child("focus strip")
        .children(
            [
                capture_types::FocusLevel::Deep,
                capture_types::FocusLevel::Mixed,
                capture_types::FocusLevel::Distracted,
            ]
            .map(|f| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(swatch(focus_color(f, p), 8.))
                    .child(stats::focus_label(f).to_lowercase())
            }),
        );
    let span_label = match now {
        Some(_) => format!("{} – now captured", stats::clock(first, off)),
        None => format!("{} captured", stats::clock_range(first, last, off, " – ")),
    };
    let plot: Div = div()
        .relative()
        .mt(space::S4)
        .h(px(LANE_H + 8. + 4. + 22.))
        .child(grid)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(LANE_H))
                .children(gaps)
                .children(blocks)
                .children(pending),
        )
        .child(
            div()
                .absolute()
                .top(px(LANE_H + 6.))
                .left_0()
                .right_0()
                .h(px(4.))
                .rounded(px(2.))
                .bg(p.wash)
                .children(focus),
        )
        .children(now_line);
    panel(p)
        .id("insights-spine")
        .p(space::S5)
        .mb(space::S7)
        .child(sec(
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .child(kicker("Day spine", p))
                .child(mono_label(span_label, p)),
            Some(legend.into_any_element()),
        ))
        .child(plot)
        .into_any_element()
}
