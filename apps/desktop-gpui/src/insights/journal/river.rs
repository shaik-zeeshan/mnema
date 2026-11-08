//! The river (`JournalRiver.svelte`): "What you did", newest first — cards
//! with their category / focus / app chips and a Receipt toggle, folded
//! sub-lines, away-gaps and failed windows — beside "This week" (stacked day
//! bars, click to step), the day mix and apps.

use capture_types::FocusLevel;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role, Window};

use super::day::{self, Card, DayModel, Row};
use crate::insights::charts::{bar, spec_rows, swatch};
use crate::insights::model::JournalData;
use crate::insights::parts::{h2, label, mono_label, sec};
use crate::insights::stats::{
    self, category_color, category_label, focus_color, focus_label, humanize_ms,
};
use crate::insights::Insights;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{clock, icon, kbd, kicker, nid, Button};

#[allow(clippy::too_many_arguments)]
pub fn render(
    this: &mut Insights,
    j: &JournalData,
    model: &DayModel,
    cards: &[Card],
    shown: &[capture_types::Activity],
    day_start: i64,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let off = clock::now(cx).offset();
    let rows = day::river(cards, &model.gaps, &model.failed);
    let n = cards.len();
    let gap_row = |s: String| {
        div()
            .py(space::S3)
            .px(space::S4)
            .my(space::S2)
            .rounded(radius::SM)
            .border_1()
            .border_dashed()
            .border_color(p.border)
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .text_color(p.text_subtle)
            .child(s)
    };
    let mut col = div().flex().flex_col().child(sec(
        div()
            .flex()
            .items_end()
            .gap(space::S3)
            .child(h2("What you did", p))
            .child(label(
                format!(
                    "{n} activit{} · newest first",
                    if n == 1 { "y" } else { "ies" }
                ),
                p,
            )),
        None,
    ));
    if model.pending.active {
        if let Some(since) = model.pending.since {
            let s = stats::clock(since, off);
            let text_ = match &model.pending.unavailable {
                None => "The journal trails live capture by a few minutes — the footage is already on the Timeline.".to_string(),
                Some(_) => "Summaries are paused — the Reasoning Engine isn't available right now.".to_string(),
            };
            col = col.child(
                div()
                    .flex()
                    .gap(space::S4)
                    .py(space::S3)
                    .child(div().w(px(64.)).child(mono_label(s.clone(), p)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space::S2)
                            .child(kicker(&format!("Writing up {s} → now"), p))
                            .child(label(text_, p)),
                    ),
            );
        }
    }
    for r in rows {
        col = col.child(match r {
            Row::Gap(g) => gap_row(format!(
                "Away · {} · {}{}",
                humanize_ms(g.end - g.start),
                stats::clock_range(g.start, g.end, off, "–"),
                if g.audio { " · audio only" } else { "" }
            ))
            .into_any_element(),
            Row::Failed(s, e) => gap_row(format!(
                "Captured, but summarizing it failed · {}",
                stats::clock_range(s, e, off, "–")
            ))
            .into_any_element(),
            Row::Card(c) => card(this, j, c, p, window, cx),
        });
    }
    let side = side(this, j, shown, day_start, p, cx);
    div()
        .flex()
        .items_start()
        .gap(space::S6)
        .child(
            div()
                .flex_grow(1.45)
                .flex_basis(px(0.))
                .min_w_0()
                .child(col),
        )
        .child(side.flex_grow(1.).flex_basis(px(0.)).min_w_0())
        .into_any_element()
}

fn card(
    this: &mut Insights,
    j: &JournalData,
    c: &Card,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let off = clock::now(cx).offset();
    let a = &c.slot.activity;
    let id = a.id;
    let span = (a.started_at_ms, a.ended_at_ms);
    let open = this.receipt.as_ref().is_some_and(|r| r.activity_id == id);
    let col = category_color(a.category, p);
    let receipt_btn = Button::new(nid("insights-receipt-toggle", id as u64), "Receipt")
        .ghost()
        .sm()
        .icon(IconName::Play.path())
        .pressed(open)
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_receipt(id, span, cx)));
    let meta = j.meta.iter().find(|m| m.activity_id == id);
    let chips =
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .mt(space::S2)
            .child(chip(category_label(a.category), ChipKind::Category(col), p))
            .children(a.focus.map(|f: FocusLevel| {
                chip(focus_label(f), ChipKind::Category(focus_color(f, p)), p)
            }))
            .children(meta.into_iter().flat_map(|m| {
                m.apps
                    .iter()
                    .map(|app| chip(app.clone(), ChipKind::Neutral, p))
            }))
            .children(meta.filter(|m| m.speakers > 0).map(|m| {
                chip(
                    format!(
                        "{} speaker{}",
                        m.speakers,
                        if m.speakers == 1 { "" } else { "s" }
                    ),
                    ChipKind::Mic,
                    p,
                )
            }))
            .child(div().flex_1())
            .child(receipt_btn);
    let head = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .child(swatch(col, 8.))
        .child(
            div()
                .text_size(text::MD)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(a.title.clone()),
        );
    let subs = div()
        .flex()
        .flex_col()
        .mt(space::S2)
        .children(c.subs.iter().map(|(s, st, en)| {
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .py(px(3.))
                .text_size(text::BASE)
                .text_color(p.text_muted)
                .child(icon(IconName::CornerDownRight.path(), px(12.)))
                .child(mono_label(stats::clock(*st, off), p))
                .child(
                    div()
                        .truncate()
                        .text_color(p.text)
                        .child(s.activity.title.clone()),
                )
                .child(mono_label(format!("· {}", humanize_ms(en - st)), p))
        }));
    let receipt = open.then(|| super::receipt::render(this, p, window, cx));
    div()
        .id(nid("insights-act", id as u64))
        .flex()
        .gap(space::S4)
        .py(space::S4)
        .border_b_1()
        .border_color(p.hairline)
        .child(
            div()
                .w(px(64.))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::BASE)
                        .text_color(p.text)
                        .child(stats::clock(c.start, off)),
                )
                .child(mono_label(humanize_ms(c.end - c.start), p)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(head)
                .when(!c.slim && !a.summary.is_empty(), |d| {
                    d.child(
                        div()
                            .mt(space::S2)
                            .text_size(text::MD)
                            .line_height(relative(1.5))
                            .text_color(p.text_muted)
                            .child(a.summary.clone()),
                    )
                })
                .child(chips)
                .children(receipt)
                .when(!c.subs.is_empty(), |d| d.child(subs)),
        )
        .into_any_element()
}

fn side(
    this: &Insights,
    j: &JournalData,
    shown: &[capture_types::Activity],
    day_start: i64,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> Div {
    let off = clock::now(cx).offset();
    let now = clock::now_ms(cx);
    let hours = j
        .history
        .as_ref()
        .map(|h| day::captured_hours(&h.activity_heatmap, off))
        .unwrap_or_default();
    let week = day::week_days(day_start, &j.week_activities, &hours, now, off);
    const BAR_MS: f32 = 8. * 3_600_000.;
    let bars = div()
        .flex()
        .gap(space::S2)
        .children(week.iter().enumerate().map(|(i, d)| {
            let sel = d.start == day_start;
            let date = stats::local(d.start, off).date();
            let caption = if d.activities > 0 {
                humanize_ms(d.activity_ms)
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else if d.captured_hours > 0 {
                format!("{}h", d.captured_hours)
            } else {
                "—".into()
            };
            let segs = d.mix.iter().rev().map(|(c, ms)| {
                div()
                    .w_full()
                    .h(px((*ms as f32 / BAR_MS * 48.).min(48.)))
                    .bg(category_color(*c, p))
            });
            let cap_only = d.activities == 0 && d.captured_hours > 0;
            div()
                .id(nid("insights-week", i as u64))
                .role(Role::Button)
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .py(px(6.))
                .rounded(radius::SM)
                .when(sel, |e| e.bg(p.wash_strong))
                .when(!d.future, |e| e.cursor_pointer().hover(|s| s.bg(p.wash)))
                .child(
                    div()
                        .w(px(14.))
                        .h(px(48.))
                        .flex()
                        .flex_col()
                        .justify_end()
                        .rounded(px(3.))
                        .overflow_hidden()
                        .bg(p.wash)
                        .when(cap_only, |b| {
                            b.child(
                                div()
                                    .w_full()
                                    .h(px((d.captured_hours as f32 / 8. * 48.).min(48.)))
                                    .bg(p.chart_grey_3),
                            )
                        })
                        .when(!cap_only, |b| b.children(segs)),
                )
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text)
                        .child(stats::weekday(d.start, off)),
                )
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(if d.future { "—".into() } else { caption }),
                )
                .when(!d.future, |e| {
                    e.on_click(cx.listener(move |this, _, _, cx| this.set_day(date, cx)))
                })
        }));
    let mix = day::category_mix(shown);
    let mix_total: i64 = mix.iter().map(|m| m.1).sum();
    let top = mix.first().map_or(1, |m| m.1).max(1);
    let mix_rows = div().flex().flex_col().children(mix.iter().map(|(c, ms)| {
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .py(px(6.))
            .text_size(text::BASE)
            .child(swatch(category_color(*c, p), 8.))
            .child(
                div()
                    .w(px(104.))
                    .text_color(p.text)
                    .child(category_label(*c)),
            )
            .child(bar(*ms as f32 / top as f32, category_color(*c, p), 6., p))
            .child(
                div()
                    .w(px(56.))
                    .text_right()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(p.text_strong)
                    .child(humanize_ms(*ms)),
            )
            .child(
                div()
                    .w(px(36.))
                    .text_right()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .child(format!("{}%", stats::fmt_pct(*ms, mix_total))),
            )
    }));
    let mut apps: Vec<_> = j
        .usage
        .as_ref()
        .map(|u| stats::listed_apps(&u.time_per_app))
        .unwrap_or_default();
    apps.sort_by(|a, b| b.active_ms.cmp(&a.active_ms));
    apps.truncate(5);
    let app_top = apps.first().map_or(1, |a| a.active_ms).max(1) as f32;
    let _ = this;
    div()
        .flex()
        .flex_col()
        .gap(space::S6)
        .child(
            div()
                .child(sec(
                    kicker("This week", p),
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(kbd("[", p))
                            .child(kbd("]", p))
                            .child(label("to step", p))
                            .into_any_element(),
                    ),
                ))
                .child(bars),
        )
        .when(!mix.is_empty(), |d| {
            d.child(
                div()
                    .child(sec(
                        kicker("Day mix", p),
                        Some(label("activity time", p).into_any_element()),
                    ))
                    .child(mix_rows),
            )
        })
        .when(!apps.is_empty(), |d| {
            d.child(
                div()
                    .child(sec(
                        kicker("Apps", p),
                        Some(label("app time", p).into_any_element()),
                    ))
                    .child(spec_rows(
                        apps.iter()
                            .map(|a| {
                                (
                                    a.app.clone().into(),
                                    a.active_ms as f32 / app_top,
                                    humanize_ms(a.active_ms).into(),
                                )
                            })
                            .collect(),
                        p.chart_grey_3,
                        p,
                    )),
            )
        })
}
