//! The river (`JournalRiver.svelte`): "What you did", newest first — cards
//! with their category / focus / app chips and a Receipt toggle, folded
//! sub-lines, away-gaps and failed windows — beside "This week" (stacked day
//! bars, click to step), the day mix and apps.

use capture_types::FocusLevel;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role};

use super::day::{self, Card, DayModel, Row};
use crate::insights::charts::{bar, spec_rows, swatch};
use crate::insights::model::JournalData;
use crate::insights::parts::{h2, label, mono_label, sec, sec_line, split, H2, KICKER, LABEL, PHI};
use crate::insights::stats::{
    self, category_color, category_label, focus_color, focus_label, humanize_ms,
};
use crate::insights::Insights;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::baseline::{row, Line};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::Tracking as _;
use crate::ui::{clock, icon, kbd, kicker, nid, ring_box, Button, Tone};

#[allow(clippy::too_many_arguments)]
pub fn render(
    this: &mut Insights,
    j: &JournalData,
    model: &DayModel,
    cards: &[Card],
    shown: &[capture_types::Activity],
    (day_start, why, pending): (i64, Option<&str>, bool),
    p: &Palette,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let off = clock::now(cx).offset();
    let rows = day::river(cards, &model.gaps, &model.failed);
    let n = cards.len();
    let day_label = stats::day_label(day_start, off);
    // `.in-gap`: a hairline-topped line, indented past the time column, the
    // label then a dashed rule filling the rest.
    let gap_row = |s: String| {
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .pt(px(10.))
            .pb(px(10.))
            .pl(px(92.))
            .border_t_1()
            .border_color(p.hairline)
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .font_weight(theme::W_MEDIUM)
            .line_height(relative(1.))
            .text_color(p.text_faint)
            .child(div().flex_none().child(s))
            .child(
                div()
                    .flex_1()
                    .border_t_1()
                    .border_dashed()
                    .border_color(p.border),
            )
    };
    let mut col = div().flex().flex_col().child(sec_line(vec![
        (H2, h2("What you did", p).into_any_element()),
        (
            LABEL,
            label(
                if why.is_some() {
                    "nothing written up".to_string()
                } else {
                    format!(
                        "{n} activit{} · newest first",
                        if n == 1 { "y" } else { "ies" }
                    )
                },
                p,
            )
            .into_any_element(),
        ),
    ]));
    if let Some(why) = why {
        let side = side(this, j, shown, day_start, false, p, cx);
        let body = col.child(empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            IconName::CircleOff.path(),
            format!("Nothing written up on {day_label}"),
            format!("{why} The Timeline has everything captured."),
            Vec::new(),
            p,
        ));
        return split(body, side, this.width)
            .items_start()
            .into_any_element();
    }
    if let Some(since) = model.pending.since.filter(|_| model.pending.active) {
        let s = stats::clock(since, off);
        let failure = this
            .status
            .ctx
            .as_ref()
            .and_then(|c| c.summarizing_failure.clone())
            .filter(|_| crate::insights::overview::engine_on(&this.status));
        let body = match (&model.pending.unavailable, failure) {
            (None, Some(f)) => div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(kicker(&format!("Not written up · {s} → now"), p))
                .child(crate::ui::notice::inline(
                    Tone::Danger,
                    Some(icon(IconName::TriangleAlert.path(), px(13.)).into_any_element()),
                    vec![
                        div()
                            .min_w_0()
                            .child(format!(
                                "Summaries are failing — {} turned away the last {}.",
                                f.provider.as_deref().unwrap_or("the AI provider"),
                                if f.failures == 1 { "try".to_string() } else { format!("{} tries", f.failures) }
                            ))
                            .into_any_element(),
                        Button::new("insights-pending-settings", "Open engine settings")
                            .ghost()
                            .inline()
                            .on_click(|_, _, cx| crate::window::open_settings(Some("intelligence"), cx))
                            .into_any_element(),
                    ],
                    p,
                )),
            (None, None) => div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(live_kicker(&format!("Writing up {s} → now"), p))
                .child(crate::ui::skeleton::skel(p).w(relative(0.46)).h(px(12.)))
                .child(crate::ui::skeleton::skel(p).w(relative(0.72)).h(px(10.)))
                .child(label(
                    if model.audio_only {
                        "Screen capture is off, so this stretch is built from what the mic and system audio picked up."
                    } else {
                        "The journal trails live capture by a few minutes — the footage is already on the Timeline."
                    },
                    p,
                )),
            (Some(reason), _) => div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(kicker(&format!("Not written up · {s} → now"), p))
                .child(crate::ui::notice::inline(
                    Tone::Neutral,
                    Some(icon(IconName::CircleOff.path(), px(13.)).into_any_element()),
                    vec![div().min_w_0().child(day::pending_reason_copy(reason)).into_any_element()],
                    p,
                )),
        };
        col = col.child(
            div()
                .id("insights-pending")
                .test_support()
                .flex()
                .gap(space::S4)
                .py(space::S3)
                // `.in-pending time`: 500 MD mono in the accent, 2 px down.
                .child(
                    div()
                        .w(TIME_W)
                        .flex_none()
                        .pt(px(2.))
                        .font_family(FONT_MONO)
                        .text_size(text::MD)
                        .font_weight(theme::W_MEDIUM)
                        .line_height(relative(1.))
                        .text_color(p.accent)
                        .child(s.clone()),
                )
                .child(body.flex_1().min_w_0()),
        );
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
            Row::Card(c) => card(this, j, c, p, cx),
        });
    }
    if n == 0 && pending {
        col = col.child(empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            IconName::Inbox.path(),
            "Your day is being written",
            "Capture is landing. The first card appears within a few minutes.",
            Vec::new(),
            p,
        ));
    }
    let side = side(this, j, shown, day_start, true, p, cx);
    split(div().child(col), side, this.width)
        .items_start()
        .into_any_element()
}

/// `.mx-kicker--live`: the kicker with a glowing accent dot in place of `//`.
fn live_kicker(s: &str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .font_family(FONT_MONO)
        .text_size(text::XS)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_subtle)
        .tracking(0.12)
        .child(
            div()
                .relative()
                .flex_none()
                .size(px(6.))
                .rounded_full()
                .bg(p.accent)
                .child(ring_box(p.accent_glow, px(3.), px(3.))),
        )
        .child(s.to_uppercase())
}

/// The time column, `.in-act`'s `76px` grid column.
const TIME_W: gpui_kit::Pixels = px(76.);

/// The card's start time (`.in-act__t time`): mono MD, inherited line height.
const TIME: Line = Line {
    family: FONT_MONO,
    size: text::MD,
    lh: PHI,
};
/// Its duration (`.in-act__t span.num`): mono XS, inherited line height.
const DUR: Line = Line {
    family: FONT_MONO,
    size: text::XS,
    lh: PHI,
};

fn card(
    this: &mut Insights,
    j: &JournalData,
    c: &Card,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let off = clock::now(cx).offset();
    let a = &c.slot.activity;
    let id = a.id;
    let span = (a.started_at_ms, a.ended_at_ms);
    let open = this.receipt.as_ref().is_some_and(|r| r.activity_id == id);
    let col = category_color(a.category, p);
    let receipt_btn = || {
        Button::new(nid("insights-receipt-toggle", id as u64), "Receipt")
            .ghost()
            .sm()
            .icon(IconName::Play.path())
            .pressed(open)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_receipt(id, span, cx)))
    };
    let slim = c.slim;
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
            .when(c.slot.media == day::Media::Expired, |d| {
                d.child(chip("footage expired", ChipKind::Neutral, p))
            })
            .child(div().flex_1())
            .child(receipt_btn());
    let head = div()
        .flex()
        .items_center()
        .gap(px(9.))
        .child(swatch(col, 9.))
        .child(crate::ui::with_tip(
            // `.in-act h4`: 600 LG strong; `.in-act--slim h4`: 500 MD.
            if slim {
                div()
                    .text_size(text::MD)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text)
            } else {
                div()
                    .text_size(text::LG)
                    .font_weight(theme::W_SEMIBOLD)
                    .text_color(p.text_strong)
            }
            .tracking(-0.01)
            // One line, cut with an ellipsis (the tooltip has the rest); without
            // `min_w_0` it keeps its full width and runs past the card.
            .min_w_0()
            .truncate()
            .id(nid("insights-act-title", id as u64))
            .child(a.title.clone()),
            a.title.clone(),
        ))
        .when(slim, |d| d.child(div().flex_1()).child(receipt_btn()));
    let open_sub = this.receipt.as_ref().map(|r| r.activity_id);
    let mut subs = div().flex().flex_col().mt(space::S2);
    for (s, st, en) in &c.subs {
        let sid = s.activity.id;
        let sspan = (s.activity.started_at_ms, s.activity.ended_at_ms);
        let sub_open = open_sub == Some(sid);
        subs = subs.child(
            div()
                .id(nid("insights-sub", sid as u64))
                .role(Role::Button)
                .flex()
                .items_center()
                .gap(space::S2)
                .py(px(5.))
                .px(px(6.))
                .ml(px(-6.))
                .rounded(radius::SM)
                .cursor_pointer()
                // `.in-sub:hover, .in-sub[aria-expanded="true"]`.
                .when(sub_open, |d| d.bg(p.wash_strong).text_color(p.text_strong))
                .when(!sub_open, |d| d.text_color(p.text_muted))
                .hover(|d| d.bg(p.wash_strong).text_color(p.text_strong))
                .text_size(text::SM)
                .font_weight(theme::W_MEDIUM)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_receipt(sid, sspan, cx)))
                .child(icon(IconName::CornerDownRight.path(), px(12.)))
                .child(mono_label(stats::clock(*st, off), p).text_size(text::XS))
                .child(crate::ui::with_tip(
                    div()
                        .id(nid("insights-sub-title", sid as u64))
                        .truncate()
                        .text_color(p.text)
                        .child(s.activity.title.clone()),
                    s.activity.title.clone(),
                ))
                .child(mono_label(format!("· {}", humanize_ms(en - st)), p).text_size(text::XS)),
        );
        if sub_open {
            subs = subs.child(super::receipt::render(this, p, cx));
        }
    }
    let receipt = open.then(|| super::receipt::render(this, p, cx));
    let bounds = this.card_bounds.clone();
    div()
        .id(nid("insights-act", id as u64))
        .relative()
        .when(this.hot == Some(id), |d| {
            d.bg(crate::ui::alpha(p.fg, 0.025))
        })
        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
            if *on {
                this.hot = Some(id);
            } else if this.hot == Some(id) {
                this.hot = None;
            }
            cx.notify();
        }))
        .child(
            gpui_kit::canvas(
                move |b, _, _| {
                    bounds.borrow_mut().insert(id, b);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .flex()
        .gap(space::S4)
        // `.in-act`: the fill reaches 12 px past the content, which stays put;
        // `.in-act--slim` pads S3 top and bottom.
        .py(if slim { space::S3 } else { space::S4 })
        .px(space::S3)
        .mx(-space::S3)
        .rounded(radius::SM)
        .border_t_1()
        .border_color(p.hairline)
        .child({
            let time = div()
                .font_family(FONT_MONO)
                .text_size(text::MD)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_strong)
                .child(stats::clock(c.start, off));
            let dur = div()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(humanize_ms(c.end - c.start));
            let col = div().w(TIME_W).flex_none().pt(px(2.));
            // `.in-act--slim .in-act__t`: time and duration side by side on one baseline.
            if slim {
                col.child(
                    row(vec![
                        (TIME, time.into_any_element()),
                        (DUR, dur.into_any_element()),
                    ])
                    // 8, not Svelte's 12: "12:45 PM" + "45s" at 12 runs into the colour dot.
                    .gap(space::S2),
                )
            } else {
                col.flex().flex_col().gap(px(5.)).child(time).child(dur)
            }
        })
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
                .when(!slim, |d| d.child(chips))
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
    with_mix: bool,
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
        .gap(px(6.))
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
            let cap_only = d.activities == 0 && d.captured_hours > 0;
            // `.in-wk__bar`: column-reverse over the reversed mix, so top to
            // bottom is the mix's own order; the top piece rounds 2 px.
            let segs: Vec<(gpui_kit::Hsla, f32)> = if cap_only {
                vec![(p.chart_grey_3, d.captured_hours as f32 / 8. * 48.)]
            } else {
                d.mix
                    .iter()
                    .map(|(c, ms)| (category_color(*c, p), *ms as f32 / BAR_MS * 48.))
                    .collect()
            };
            // `.in-wk__bar:empty`: a 20 px dashed stub on the baseline.
            let bar = if d.future || segs.is_empty() {
                div()
                    .w(px(14.))
                    .h(px(20.))
                    .mt(px(28.))
                    .border_1()
                    .border_dashed()
                    .border_color(p.border)
                    .rounded(px(2.))
            } else {
                div()
                    .w(px(14.))
                    .h(px(48.))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .gap(px(1.))
                    .children(segs.into_iter().enumerate().map(|(k, (c, h))| {
                        div()
                            .flex_none()
                            .w_full()
                            .h(px(h.min(48.)))
                            .bg(c)
                            .when(k == 0, |d| d.rounded_t(px(2.)))
                    }))
            };
            let tip = format!(
                "{} · {}",
                stats::day_label(d.start, off),
                if d.activities > 0 {
                    format!(
                        "{} · {} activities",
                        humanize_ms(d.activity_ms),
                        d.activities
                    )
                } else if cap_only {
                    format!("~{}h captured · nothing written up", d.captured_hours)
                } else {
                    "nothing captured".into()
                }
            );
            let el = div().id(nid("insights-week", i as u64));
            let el = if d.future {
                el
            } else {
                crate::ui::with_tip(el, tip)
            };
            el.role(Role::Button)
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .pt(px(8.))
                .pb(px(7.))
                .rounded(radius::MD)
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .line_height(gpui_kit::relative(1.))
                .font_weight(theme::W_MEDIUM)
                .text_color(if d.future {
                    p.text_faint
                } else if sel {
                    p.text_strong
                } else {
                    p.text_subtle
                })
                // `[aria-pressed]`: surface + strong hairline (inset: no layout shift).
                .when(sel, |e| {
                    e.bg(p.surface)
                        .shadow(vec![crate::ui::inset(p.border_strong, px(1.))])
                })
                .when(!d.future, |e| {
                    e.cursor_pointer().hover(|s| s.bg(p.surface_hover))
                })
                .child(bar)
                .child(div().child(stats::weekday(d.start, off)))
                .child(div().child(if d.future { "—".into() } else { caption }))
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
            .font_weight(theme::W_MEDIUM)
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
        .when(with_mix && !mix.is_empty(), |d| {
            d.child(
                div()
                    .child(
                        sec_line(vec![
                            (KICKER, kicker("Day mix", p).into_any_element()),
                            (LABEL, label("activity time", p).into_any_element()),
                        ])
                        .justify_between(),
                    )
                    .child(mix_rows),
            )
        })
        .when(!apps.is_empty(), |d| {
            d.child(
                div()
                    .child(
                        sec_line(vec![
                            (KICKER, kicker("Apps", p).into_any_element()),
                            (LABEL, label("app time", p).into_any_element()),
                        ])
                        .justify_between(),
                    )
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
                        // `.in-apps`: an 82 px name column.
                        Some(px(82.)),
                        p,
                        cx,
                    )),
            )
        })
}
