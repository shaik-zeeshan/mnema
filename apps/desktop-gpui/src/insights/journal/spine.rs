//! The day spine (`DaySpine.svelte`): the day's first→last span rounded out to
//! hours with gridlines; activities as category blocks in stacked lanes, dashed
//! "away" gaps, the pending edge, a focus strip under them, and today's now-line.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{canvas, div, point, px, relative, AnyElement, Context, Div, PathBuilder, Role};
use time::UtcOffset;

use super::day::{spine_axis, stack_lanes, Card, DayModel};
use crate::insights::charts::swatch;
use crate::insights::parts::{mono_label, panel, KICKER, MONO_LABEL};
use crate::insights::stats::{self, category_color, focus_color, humanize_ms};
use crate::insights::Insights;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::baseline;
use crate::ui::{alpha, kicker, mix, nid, spread};

/// `.in-sp__lane { --lh: 64px }`, 40 px a lane when overlaps stack, 4 px apart.
const LANE_H: f32 = 64.;
const LANE_H_MULTI: f32 = 40.;
const LANE_GAP: f32 = 4.;
/// `.in-sp__focus`: 6 px tall, 6 px under the lane.
const FOCUS_H: f32 = 6.;
/// `.in-sp { padding-bottom: 22px }`: the hour labels' row.
const LABEL_ROW: f32 = 22.;

/// An absolutely placed slot `w` of the plot wide; `trim` = the CSS
/// `calc(w% - 2px)` (the box is the slot's child, 2 px short).
fn slot(left: f32, w: f32, trim: bool) -> Div {
    div()
        .absolute()
        .left(relative(left))
        .w(relative(w))
        .when(trim, |d| d.pr(px(2.)))
}

/// `.in-sp__a` / `.in-sp__run`: top border 2 px, corners 3 3 r-xs r-xs, pad
/// 9 9 0, gap 4, filling its slot.
fn tower<E: Styled>(d: E, col: gpui_kit::Hsla) -> E {
    d.size_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .pt(px(9.))
        .px(px(9.))
        .overflow_hidden()
        .rounded_t(px(3.))
        .rounded_b(radius::XS)
        .border_t_2()
        .border_color(col)
}

/// `.in-sp__pend`'s `repeating-linear-gradient(135deg, accent 12% 0 1.5px,
/// transparent 1.5px 6px)`: 1.5 px "/" stripes every 6 px across the box.
fn hatch(color: gpui_kit::Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (x0, y0) = (f32::from(b.left()), f32::from(b.top()));
            let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
            let step = 6. * std::f32::consts::SQRT_2;
            let mut path = PathBuilder::stroke(px(1.5));
            let mut c = 0.75 * std::f32::consts::SQRT_2;
            while c - h < w {
                // The line x + y = c, from the top edge down to the bottom one.
                path.move_to(point(px(x0 + c), px(y0)));
                path.line_to(point(px(x0 + c - h), px(y0 + h)));
                c += step;
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .inset_0()
}

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
    let lanes = if runs.is_some() { 1 } else { lanes };
    let lane_h = if lanes > 1 { LANE_H_MULTI } else { LANE_H };
    let lane_total = lanes as f32 * lane_h + (lanes as f32 - 1.) * LANE_GAP;
    // `.in-sp`: the lane, the focus strip (not on capture-run days), the labels.
    let strip = if runs.is_none() { FOCUS_H * 2. } else { 0. };
    let plot_h = lane_total + strip + LABEL_ROW;
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
            let el = div()
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
            let el = tower(crate::ui::with_tip(el, tip), col)
                .bg(mix(col, p.surface, if on { 0.28 } else { 0.16 }))
                .when(w > 0.06, |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(text::SM)
                            .line_height(relative(1.25))
                            .font_weight(theme::W_SEMIBOLD)
                            .text_color(p.text_strong)
                            .child(a.title.clone()),
                    )
                    .when(lanes == 1, |d| {
                        d.child(
                            div()
                                .font_family(FONT_MONO)
                                .text_size(text::XS)
                                .line_height(relative(1.))
                                .whitespace_nowrap()
                                .text_color(p.text_subtle)
                                .child(humanize_ms(c.end - c.start)),
                        )
                    })
                });
            slot(x(c.start), w, true)
                .top(px(lane[i] as f32 * (lane_h + LANE_GAP)))
                .h(px(lane_h))
                .child(el)
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
        let el = div().id(nid("insights-spine-run", i as u64)).test_support();
        let el = tower(crate::ui::with_tip(el, tip), p.chart_grey_3)
            .bg(mix(p.fg, p.surface, 0.05))
            .when(w > 0.06, |d| {
                d.child(
                    div()
                        .truncate()
                        .text_size(text::SM)
                        .line_height(relative(1.25))
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
                        .line_height(relative(1.))
                        .whitespace_nowrap()
                        .text_color(p.text_subtle)
                        .child(format!("{took} captured")),
                )
            });
        slot(x(r.0), w, true).top_0().h(px(lane_h)).child(el)
    });
    let gaps = away.into_iter().enumerate().map(|(i, (gs, ge))| {
        let w = x(ge) - x(gs);
        let tip = format!(
            "Away · {} · {} · no capture",
            humanize_ms(ge - gs),
            stats::clock_range(gs, ge, off, "–")
        );
        // `.in-sp__gap`: inset 10 px top and bottom, sans, centred.
        let el = div().id(nid("insights-spine-gap", i as u64));
        let el = crate::ui::with_tip(el, tip)
            .size_full()
            .rounded(radius::SM)
            .border_1()
            .border_dashed()
            .border_color(p.border)
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(text::SM)
            .line_height(relative(1.))
            .font_weight(theme::W_MEDIUM)
            .text_color(p.text_faint)
            .when(w > 0.04, |d| d.child("away"));
        slot(x(gs), w, false)
            .top(px(10.))
            .h(px(lane_total - 20.))
            .child(el)
    });
    let pending = model
        .pending
        .since
        .filter(|s| runs.is_none() && model.pending.active && now.is_some_and(|n| n > *s))
        .map(|s| {
            let w = x(now.unwrap_or(s)) - x(s);
            let state = if model.pending.unavailable.is_some() {
                "Paused"
            } else {
                "Writing up"
            };
            let el = slot(x(s), w, false)
                .id("insights-spine-pending")
                .top_0()
                .h(px(lane_total))
                .rounded(radius::XS)
                .border_1()
                .border_dashed()
                .border_color(p.accent_border)
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .whitespace_nowrap()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .line_height(relative(1.))
                .text_color(p.accent)
                .child(hatch(alpha(p.accent, 0.12)))
                .when(w > 0.06, |d| {
                    d.child(if model.pending.unavailable.is_some() {
                        "paused"
                    } else {
                        "writing up"
                    })
                });
            crate::ui::with_tip(el, format!("{state} · {} → now", stats::clock(s, off)))
        });
    let focus = cards.iter().filter(|_| runs.is_none()).filter_map(|c| {
        let f = c.slot.activity.focus?;
        let a = &c.slot.activity;
        let tip = format!(
            "{} · {}",
            stats::focus_label(f),
            stats::clock_range(a.started_at_ms, a.ended_at_ms, off, "–")
        );
        Some(
            slot(x(c.start), x(c.end) - x(c.start), true)
                .top_0()
                .bottom_0()
                .child(crate::ui::with_tip(
                    div()
                        .id(nid("insights-spine-focus", a.id as u64))
                        .size_full()
                        .rounded(px(3.))
                        .bg(focus_color(f, p)),
                    tip,
                )),
        )
    });
    // `.in-sp__grid` (inset -6px 0 0): each line stops 18 px above the plot's
    // bottom, its label hangs under it in that row.
    let grid = div().absolute().inset_0().children(ticks.iter().map(|t| {
        div()
            .absolute()
            .left(relative(x(*t)))
            .top(px(-6.))
            .bottom(px(18.))
            .w(px(1.))
            .bg(p.hairline)
            .child(
                div()
                    .absolute()
                    .bottom(px(-18.))
                    .left(px(-30.))
                    .w(px(60.))
                    .flex()
                    .justify_center()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .line_height(relative(1.))
                    .whitespace_nowrap()
                    .text_color(p.text_faint)
                    .child(stats::clock(*t, off)),
            )
    }));
    // `.in-sp__now`: top -10, bottom 16, 2 px with a 3 px glow, "now" 14 px above.
    let now_line = now.map(|n| {
        div()
            .absolute()
            .left(relative(x(n)))
            .ml(px(-1.))
            .top(px(-10.))
            .bottom(px(16.))
            .w(px(2.))
            .rounded(px(1.))
            .bg(p.accent)
            .shadow(vec![spread(p.accent_glow, px(3.))])
            .child(
                div()
                    .absolute()
                    .top(px(-14.))
                    .left(px(-20.))
                    .w(px(42.))
                    .flex()
                    .justify_center()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .line_height(relative(1.))
                    .text_color(p.accent)
                    .child("now"),
            )
    });
    // `.in-sp__legend`: SM sans, inherited line height.
    const LEGEND: baseline::Line = baseline::Line {
        family: theme::FONT_UI,
        size: text::SM,
        lh: baseline::PHI,
    };
    let legend = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(space::S4)
        .gap_y(space::S2)
        .text_size(text::SM)
        .text_color(p.text_subtle)
        .when(runs.is_some(), |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
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
                    .gap(px(6.))
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
        .mt(space::S5)
        .h(px(plot_h))
        .child(grid)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(lane_total))
                .children(gaps)
                .children(run_blocks)
                .children(blocks)
                .children(pending),
        )
        .when(runs.is_none(), |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(lane_total + FOCUS_H))
                    .left_0()
                    .right_0()
                    .h(px(FOCUS_H))
                    .rounded(px(3.))
                    .bg(alpha(p.fg, 0.04))
                    .children(focus),
            )
        })
        .children(now_line);
    panel(p)
        .id("insights-spine")
        .p(space::S5)
        .mb(space::S7)
        // `.in-sec`: kicker, span and the (wrapping) legend's first line on one baseline.
        .child(
            baseline::row_top(vec![
                (KICKER, kicker("Day spine", p)),
                (MONO_LABEL, mono_label(span_label, p)),
                (LEGEND, div().flex_1()),
                (LEGEND, legend),
            ])
            .gap(space::S3),
        )
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
