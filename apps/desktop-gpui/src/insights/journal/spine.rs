//! The day spine (`DaySpine.svelte`): the day's first→last span rounded out to
//! hours with gridlines; activities as category blocks in stacked lanes, dashed
//! "away" gaps, the pending edge, a focus strip under them, and today's now-line.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role};
use time::UtcOffset;

use super::day::{spine_axis, stack_lanes, Card, DayModel};
use crate::insights::charts::swatch;
use crate::insights::parts::{mono_label, panel, sec};
use crate::insights::stats::{self, category_color, focus_color, humanize_ms};
use crate::insights::Insights;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::{alpha, kicker, mix, nid};

const LANE_H: f32 = 44.;

/// A capture run on a day nothing was written up: `(start, end, top apps)`.
pub type Run = (i64, i64, Vec<String>);

#[allow(clippy::too_many_arguments)]
pub fn render(
    this: &Insights,
    model: &DayModel,
    cards: &[Card],
    runs: Option<&[Run]>,
    now: Option<i64>,
    p: &Palette,
    off: UtcOffset,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let hot = this.hot;
    let spans: Vec<(i64, i64)> = cards.iter().map(|c| (c.start, c.end)).collect();
    let mut all: Vec<(i64, i64)> = spans.clone();
    all.extend(runs.unwrap_or(&[]).iter().map(|r| (r.0, r.1)));
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
    let blocks = cards
        .iter()
        .enumerate()
        .filter(|_| runs.is_none())
        .map(|(i, c)| {
            let a = &c.slot.activity;
            let id = a.id;
            let on = hot == Some(id);
            let w = x(c.end) - x(c.start);
            let col = category_color(a.category, p);
            let tip = format!(
                "{} · {} · {}{}",
                a.title,
                stats::clock_range(a.started_at_ms, a.ended_at_ms, off, "–"),
                humanize_ms(a.ended_at_ms - a.started_at_ms),
                a.category
                    .map(|k| format!(" · {}", stats::category_label(Some(k))))
                    .unwrap_or_default()
            );
            let el = block(x(c.start), w)
                .id(nid("insights-spine-block", id as u64))
                .test_support()
                .role(Role::Button)
                .cursor_pointer()
                .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                    this.hot = on.then_some(id);
                    cx.notify();
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.pick_card(id, cx)))
                .when(hot.is_some() && !on, |d| d.opacity(0.4));
            crate::ui::with_tip(el, tip)
                .top(px(lane[i] as f32 * lane_h))
                .h(px(lane_h - 2.))
                .rounded(radius::XS)
                .border_t_2()
                .border_color(col)
                .bg(mix(col, p.surface, if on { 0.28 } else { 0.18 }))
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
    // Capture-run days cut only at gaps: the gaps are what lies between runs.
    let away: Vec<(i64, i64)> = match runs {
        Some(r) => r.windows(2).map(|w| (w[0].1, w[1].0)).collect(),
        None => model.gaps.iter().map(|g| (g.start, g.end)).collect(),
    };
    let run_blocks = runs.unwrap_or(&[]).iter().enumerate().map(|(i, r)| {
        let w = x(r.1) - x(r.0);
        let took = humanize_ms(r.1 - r.0);
        let apps = r.2.join(", ");
        let tip = format!(
            "Captured · {took} · {}{}",
            stats::clock_range(r.0, r.1, off, "–"),
            if apps.is_empty() {
                String::new()
            } else {
                format!(" · {apps}")
            }
        );
        let el = block(x(r.0), w)
            .id(nid("insights-spine-run", i as u64))
            .test_support();
        crate::ui::with_tip(el, tip)
            .top_0()
            .h(px(LANE_H - 2.))
            .rounded(radius::XS)
            .border_t_2()
            .border_color(p.chart_grey_3)
            .bg(mix(p.fg, p.surface, 0.05))
            .px(px(8.))
            .py(px(6.))
            .overflow_hidden()
            .when(w > 0.06, |d| {
                d.child(
                    div()
                        .truncate()
                        .text_size(text::SM)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text)
                        .child(if r.2.is_empty() {
                            "Captured".to_string()
                        } else {
                            r.2.join(" · ")
                        }),
                )
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(format!("{took} captured")),
                )
            })
    });
    let gaps = away.into_iter().enumerate().map(|(i, (gs, ge))| {
        let w = x(ge) - x(gs);
        let tip = format!(
            "Away · {} · {} · no capture",
            humanize_ms(ge - gs),
            stats::clock_range(gs, ge, off, "–")
        );
        let el = block(x(gs), w).id(nid("insights-spine-gap", i as u64));
        crate::ui::with_tip(el, tip)
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
        .filter(|s| runs.is_none() && model.pending.active && now.is_some_and(|n| n > *s))
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
    let focus = cards.iter().filter(|_| runs.is_none()).filter_map(|c| {
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
        .when(runs.is_some(), |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(swatch(p.chart_grey_3, 8.))
                    .child("capture runs"),
            )
            .child("no activities · categories and focus need AI")
        })
        .when(runs.is_none(), |d| d.child("focus strip"))
        .children(runs.is_none().then_some(()).into_iter().flat_map(|_| {
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
            })
        }));
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
                .children(run_blocks)
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

impl Insights {
    /// A spine block: bring its card into view, centred (`scrollIntoView`).
    pub fn pick_card(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(b) = self.card_bounds.borrow().get(&id).copied() else {
            return;
        };
        let view = self.scroll.bounds();
        let content_y = b.top() - view.top() - self.scroll.offset().y;
        let y = content_y - (view.size.height - b.size.height) / 2.;
        self.scroll
            .set_offset(gpui_kit::point(px(0.), -y.max(px(0.))));
        cx.notify();
    }

    /// The capture runs' top apps (one usage read per run), once per day.
    pub(super) fn load_run_apps(&mut self, runs: Vec<(i64, i64)>, cx: &mut Context<Self>) {
        let day = self.day;
        if self.run_apps.as_ref().is_some_and(|r| r.0 == day) {
            return;
        }
        self.run_apps = Some((day, Vec::new()));
        self.write(
            cx,
            move |b| {
                runs.iter()
                    .map(|(s, e)| {
                        let mut apps = b
                            .usage(*s, (*e).max(s + 1))
                            .map(|u| u.time_per_app)
                            .unwrap_or_default();
                        apps.sort_by(|a, b| b.active_ms.cmp(&a.active_ms));
                        apps.into_iter().take(3).map(|a| a.app).collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            },
            move |this, apps, _| {
                if this.day == day {
                    this.run_apps = Some((day, apps));
                }
            },
        );
    }
}
