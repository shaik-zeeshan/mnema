//! A subject's page (`SubjectDetail.svelte` + `SubjectHeader` + `SubjectChart`
//! + `ConclusionTiers`) and the evidence drawer (`EvidenceDrawer.svelte`). The
//! drawer pushes: the page narrows beside it so the clicked row stays on
//! screen. Each conclusion keeps its own confidence. A dismissed row collapses
//! to "Dismissed · Undo" until the 5 s window commits it.

use std::collections::HashMap;

use capture_types::{Conclusion, ConclusionStatus, ConfidenceSnapshot, EvidenceStance};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Pixels, Role, Window};

use super::tier_of;
use super::tiers::{derive_trend, fortnight_ago, interpolate, pct, Tier, DISPLAY_FLOOR};
use crate::insights::charts::{line_chart, sparkline, Line};
use crate::insights::parts::{
    display, group_head, label, lede, panel, prose, read, sec, stat, stat_value,
};
use crate::insights::stats;
use crate::insights::Insights;
use crate::theme::{self, breakpoint, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{clock, icon, kicker, nid, Button, Tone};

pub(super) const TIERS: [(Tier, &str, &str); 4] = [
    (Tier::Strong, "Strongly held", "confidence 68% and up"),
    (Tier::Forming, "Forming", "confidence 30–68%"),
    (Tier::Shaping, "Just taking shape", "confidence 15–30%"),
    (
        Tier::Fading,
        "Fading",
        "confidence under 15% · kept for history",
    ),
];

pub(super) fn histories(this: &Insights) -> HashMap<i64, Vec<ConfidenceSnapshot>> {
    this.detail
        .as_ref()
        .and_then(|d| d.view.as_ref())
        .map(|v| {
            v.trajectories
                .iter()
                .map(|t| (t.conclusion_id, t.history.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Every conclusion, confidence-desc (rows inside their Undo window included:
/// they collapse in place).
fn shown(this: &Insights) -> Vec<Conclusion> {
    let mut v: Vec<Conclusion> = this
        .detail
        .as_ref()
        .and_then(|d| d.view.as_ref())
        .map(|v| v.conclusions.clone())
        .unwrap_or_default();
    v.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    v
}

/// Reading order for ↑/↓ (tier by tier), minus pending dismissals.
pub fn flat(this: &Insights) -> Vec<Conclusion> {
    let all: Vec<Conclusion> = shown(this)
        .into_iter()
        .filter(|c| !this.undo.has(c.id))
        .collect();
    TIERS
        .iter()
        .flat_map(|(t, _, _)| {
            all.iter()
                .filter(|c| tier_of(c) == *t)
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect()
}

/// "3d" · "5h" · "12m" (`relativeTime`), split into number + unit.
fn ago(ms: i64, now: i64) -> (String, String) {
    let m = ((now - ms).max(0) / 60_000) as i64;
    if m < 60 {
        (m.to_string(), "m ago".into())
    } else if m < 24 * 60 {
        ((m / 60).to_string(), "h ago".into())
    } else {
        ((m / 1440).to_string(), "d ago".into())
    }
}

/// The subject page's column (`@container sd`): `.in-view`'s content box
/// (≤1280 less its padding) less the drawer's push.
pub fn column_width(this: &Insights) -> Pixels {
    this.width.min(px(super::drawer::PAGE_MAX)) - space::S7 * 2. - super::drawer::push(this)
}

pub fn render(
    this: &mut Insights,
    p: &Palette,
    _: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let subject = this.subject.clone().unwrap_or_default();
    let ask_ok = crate::insights::overview::engine_ready(&this.status);
    // `@container sd (max-width: 560px)`: the crumb's Ask, the section notes go.
    let tight = column_width(this) <= breakpoint::SUBJECT_TIGHT;
    let crumb = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .mb(space::S4)
        .child(
            Button::new("insights-subject-back", "Subjects")
                .ghost()
                .sm()
                .icon(IconName::ChevronLeft.path())
                .on_click(cx.listener(|this, _, _, cx| this.close_subject(cx))),
        )
        .child(div().text_color(p.text_faint).child("/"))
        .child(
            div()
                .text_size(text::BASE)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(subject.clone()),
        )
        .child(div().flex_1())
        .when(!tight, |d| {
            d.child(
                Button::new("insights-subject-ask", format!("Ask about {subject}"))
                    .ghost()
                    .sm()
                    .icon(IconName::Sparkle.path())
                    .accent_icon()
                    .kbd("⌘J")
                    .soft_disabled(!ask_ok)
                    .tip(if ask_ok {
                        "Opens Chat with this question"
                    } else {
                        "Ask needs AI — turn it on in Settings"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.ask(cx))),
            )
        });
    let Some(d) = this.detail.clone() else {
        return div()
            .child(crumb)
            .child(crate::ui::skeleton::lines(5, p))
            .into_any_element();
    };
    if let Some(e) = &d.error {
        return div()
            .child(crumb)
            .child(empty(
                EmptyLayout::Center,
                Tone::Danger,
                IconName::TriangleAlert.path(),
                "Couldn't load this subject.",
                format!("Reading its conclusions and history failed. {e}"),
                vec![Button::new("insights-subject-retry", "Try again")
                    .sm()
                    .icon(IconName::RotateCcw.path())
                    .on_click(cx.listener(|this, _, _, cx| this.reload_subject(cx)))
                    .into_any_element()],
                p,
            ))
            .into_any_element();
    }
    let all = shown(this);
    let live: Vec<&Conclusion> = all.iter().filter(|c| !this.undo.has(c.id)).collect();
    let Some(lead) = live.first().copied().cloned() else {
        return div()
            .child(crumb)
            .child(empty(EmptyLayout::Center, Tone::Neutral, IconName::Inbox.path(), format!("Nothing concluded about {subject} yet."), "Its conclusions were dismissed or haven't formed. Views appear here once two activities support them. Anything you dismissed is in Context › Dismissed.", Vec::new(), p))
            .into_any_element();
    };
    let hist = histories(this);
    let now = clock::now_ms(cx);
    let off = clock::now(cx).offset();
    let delta = |c: &Conclusion| {
        fortnight_ago(c, hist.get(&c.id).map_or(&[], |v| v), now)
            .map(|t| pct(c.confidence) - pct(t))
    };
    let signed = |x: i64| {
        format!(
            "{}{}",
            if x > 0 {
                "+"
            } else if x < 0 {
                "−"
            } else {
                "±"
            },
            x.abs()
        )
    };
    let live_owned: Vec<Conclusion> = live.iter().map(|c| (*c).clone()).collect();
    let trend = if live_owned
        .iter()
        .all(|c| c.status == ConclusionStatus::Faded)
    {
        "fading"
    } else {
        match derive_trend(&live_owned, Some(&hist)) {
            super::tiers::Trend::Up => "warming",
            super::tiers::Trend::Steady => "steady",
            super::tiers::Trend::Down => "cooling",
        }
    };
    let since = live.iter().map(|c| c.formed_at_ms).min().unwrap_or(0);
    let above = live
        .iter()
        .filter(|c| c.status != ConclusionStatus::Faded)
        .count();
    let mut ev: Vec<i64> = live
        .iter()
        .flat_map(|c| c.evidence.iter().map(|e| e.activity_id))
        .collect();
    ev.sort();
    ev.dedup();
    let last_support = live
        .iter()
        .map(|c| c.last_supported_at_ms)
        .max()
        .unwrap_or(0);
    let (n_ago, u_ago) = ago(last_support, now);
    let lead_delta = delta(&lead);
    let lede_el = lede(
        &format!("Subject · since {} · {trend}", stats::month_day(since, off)),
        p,
    )
    .child(display(subject.clone(), None, p))
    .child(prose(
        format!(
            "{} — its strongest conclusion, at {}%.",
            lead.statement,
            pct(lead.confidence)
        ),
        p,
    ));
    let n_ev = ev.len();
    let stats_el = vec![
        stat(
            "Lead confidence",
            stat_value(&format!("{}%", pct(lead.confidence)), p),
            Some(lead_delta.map_or("formed this fortnight".into(), |x| {
                format!("{} pts in 14d", signed(x))
            })),
            lead_delta.is_some_and(|x| x > 0),
            p,
        )
        .into_any_element(),
        stat(
            "Conclusions",
            stat_value(&above.to_string(), p),
            Some(format!("{} faded below the floor", live.len() - above)),
            false,
            p,
        )
        .into_any_element(),
        stat(
            "Evidence",
            stat_value(&n_ev.to_string(), p),
            Some(format!(
                "{} cited",
                if n_ev == 1 { "activity" } else { "activities" }
            )),
            false,
            p,
        )
        .into_any_element(),
        stat(
            "Last support",
            stat_value(&format!("{n_ago}{u_ago}"), p),
            Some(
                if stats::start_of_day(last_support, off) == stats::start_of_day(now, off) {
                    "today".into()
                } else {
                    stats::month_day(last_support, off)
                },
            ),
            false,
            p,
        )
        .into_any_element(),
    ];
    let chart_id = this.open_conclusion.unwrap_or(lead.id);
    let chart = chart(&live_owned, &hist, chart_id, off, p);
    let mut tiers_el = div().flex().flex_col().child(sec(
        kicker("Conclusions", p),
        Some(label(format!("{} · by confidence", live.len()), p).into_any_element()),
    ));
    for (t, title, note) in TIERS {
        // A pending Dismiss has left the list (its toast holds the Undo).
        let rows: Vec<&Conclusion> = live.iter().copied().filter(|c| tier_of(c) == t).collect();
        if rows.is_empty() {
            continue;
        }
        let key = tier_key(t);
        let (from, to, _, _) = super::pager::slice(this, &key, rows.len(), super::tiers::TIER_PAGE);
        let pager = super::pager::pager(
            this,
            &key,
            title,
            rows.len(),
            super::tiers::TIER_PAGE,
            true,
            cx,
        );
        tiers_el = tiers_el.child(group_head(
            &format!("{title} · {}", rows.len()),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .when(!tight, |d| d.child(label(note, p).text_size(text::SM)))
                    .children(pager)
                    .into_any_element(),
            ),
            p,
        ));
        for c in rows[from..to].iter().copied() {
            tiers_el = tiers_el.child(row(this, c, &hist, delta(c).map(signed), p, cx));
        }
    }
    div()
        .flex()
        .flex_col()
        .child(crumb)
        .child(read(lede_el, stats_el, column_width(this), p))
        .child(
            panel(p)
                .p(if tight { space::S4 } else { space::S5 })
                .mb(space::S6)
                .child(sec(
                    kicker("Confidence over time", p),
                    (!tight).then(|| {
                        label(
                            format!(
                                "{} conclusion in bold · others in grey · ● evidence",
                                if this.open_conclusion.is_some() {
                                    "open"
                                } else {
                                    "lead"
                                }
                            ),
                            p,
                        )
                        .into_any_element()
                    }),
                ))
                .child(chart),
        )
        .child(tiers_el)
        .into_any_element()
}

fn chart(
    cs: &[Conclusion],
    hist: &HashMap<i64, Vec<ConfidenceSnapshot>>,
    sel: i64,
    off: time::UtcOffset,
    p: &Palette,
) -> Div {
    let picked: Vec<&Conclusion> = cs
        .iter()
        .take(10)
        .chain(cs.iter().filter(|c| c.id == sel))
        .collect();
    let line_hist = |c: &Conclusion| -> Vec<ConfidenceSnapshot> {
        hist.get(&c.id)
            .filter(|h| !h.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                vec![ConfidenceSnapshot {
                    confidence: c.confidence,
                    snapshot_at_ms: c.formed_at_ms,
                }]
            })
    };
    let sel_c = cs.iter().find(|c| c.id == sel);
    let mut times: Vec<i64> = picked
        .iter()
        .flat_map(|c| {
            let mut t = vec![c.formed_at_ms, c.last_supported_at_ms];
            t.extend(line_hist(c).iter().map(|s| s.snapshot_at_ms));
            t
        })
        .collect();
    if let Some(s) = sel_c {
        times.extend(s.evidence.iter().filter_map(|e| e.activity_started_at_ms));
    }
    let to = times.iter().copied().max().unwrap_or(0);
    let from = times
        .iter()
        .copied()
        .min()
        .unwrap_or(to)
        .min(to - stats::DAY_MS);
    let x = |ms: i64| ((ms - from) as f32 / (to - from).max(1) as f32).clamp(0., 1.);
    let pts = |h: &[ConfidenceSnapshot]| {
        let mut v: Vec<(f32, f32)> = h
            .iter()
            .map(|s| (x(s.snapshot_at_ms), s.confidence as f32))
            .collect();
        if let Some(last) = v.last().copied().filter(|l| l.0 < 1.) {
            v.push((1., last.1));
        }
        v
    };
    let mut lines: Vec<Line> = picked
        .iter()
        .filter(|c| c.id != sel)
        .map(|c| Line {
            points: pts(&line_hist(c)),
            color: p.chart_grey_2,
            width: 1.5,
            dashed: false,
        })
        .collect();
    let mut dots = Vec::new();
    if let Some(s) = sel_c {
        let h = line_hist(s);
        lines.push(Line {
            points: pts(&h),
            color: p.text_strong,
            width: 2.,
            dashed: false,
        });
        dots.push((x(s.formed_at_ms), h[0].confidence as f32, true));
        for e in s
            .evidence
            .iter()
            .filter(|e| e.stance == EvidenceStance::Support)
        {
            if let Some(at) = e.activity_started_at_ms {
                dots.push((x(at), interpolate(&h, Some(at), s.confidence) as f32, false));
            }
        }
    }
    let mid = from + (to - from) / 2;
    line_chart(
        lines,
        dots,
        DISPLAY_FLOOR as f32,
        [
            stats::month_day(from, off),
            stats::month_day(mid, off),
            stats::month_day(to, off),
        ],
        p,
    )
}

fn row(
    this: &Insights,
    c: &Conclusion,
    hist: &HashMap<i64, Vec<ConfidenceSnapshot>>,
    delta: Option<String>,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let id = c.id;
    if this.undo.has(id) {
        return div()
            .id(nid("insights-concl", id as u64))
            .flex()
            .items_center()
            .gap(space::S2)
            .py(px(8.))
            .px(space::S2)
            .border_b_1()
            .border_color(p.hairline)
            .text_size(text::BASE)
            .text_color(p.text_subtle)
            .child("Dismissed")
            .child(div().text_color(p.text_faint).child("·"))
            .child(
                Button::new(nid("insights-undo", id as u64), "Undo")
                    .ghost()
                    .sm()
                    .on_click(cx.listener(move |this, _, _, cx| this.undo_dismiss(id, cx))),
            )
            .into_any_element();
    }
    let on = this.open_conclusion == Some(id);
    // `@container sd`: ≤760 px drops the evidence count, ≤560 px the sparkline.
    let cw = column_width(this);
    let pts: Vec<f64> = hist
        .get(&id)
        .filter(|h| h.len() > 1)
        .map(|h| h.iter().map(|s| s.confidence).collect())
        .unwrap_or_else(|| vec![c.confidence, c.confidence]);
    let dcol = match delta.as_deref() {
        None => p.accent,
        Some(s) if s.starts_with('+') => p.focus_deep,
        Some(s) if s.starts_with('−') => p.focus_distracted,
        _ => p.text_subtle,
    };
    div()
        .id(nid("insights-concl", id as u64))
        .role(Role::Button)
        .flex()
        .items_center()
        .gap(space::S3)
        .py(px(9.))
        .px(space::S2)
        .border_b_1()
        .border_color(p.hairline)
        .cursor_pointer()
        .when(on, |d| d.bg(p.wash_strong))
        .hover(|s| s.bg(p.wash))
        .when(c.status == ConclusionStatus::Faded, |d| d.opacity(0.6))
        .child(
            div()
                .w(px(40.))
                .flex_none()
                .font_family(FONT_MONO)
                .text_size(text::MD)
                .text_color(p.text_strong)
                .child(format!("{}%", pct(c.confidence))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(text::MD)
                .line_height(relative(1.4))
                .text_color(p.text)
                .child(c.statement.clone()),
        )
        .when(cw > breakpoint::SUBJECT_TIGHT, |d| {
            d.child(
                div()
                    .w(px(if cw > breakpoint::READ_NARROW {
                        80.
                    } else {
                        72.
                    }))
                    .h(px(22.))
                    .flex_none()
                    .child(sparkline(vec![(p.text_subtle, false, pts)])),
            )
        })
        .child(
            div()
                .w(px(36.))
                .text_right()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(dcol)
                .child(delta.unwrap_or_else(|| "new".into())),
        )
        .when(cw > breakpoint::READ_NARROW, |d| {
            d.child(
                div()
                    .w(px(40.))
                    .text_right()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .child(format!("{} ev", c.evidence.len())),
            )
        })
        .child(
            div()
                .text_color(p.text_faint)
                .child(icon(IconName::ChevronRight.path(), px(14.))),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.open_conclusion = if this.open_conclusion == Some(id) {
                None
            } else {
                Some(id)
            };
            cx.notify();
        }))
        .into_any_element()
}

/// A subject-page tier's pager key.
pub fn tier_key(t: Tier) -> String {
    format!("t:{t:?}")
}

/// Turn conclusion `id`'s tier to its row (`reveal`), on opening it.
pub fn reveal(this: &mut Insights, id: i64) {
    let live: Vec<Conclusion> = flat(this);
    let Some(c) = live.iter().find(|c| c.id == id) else {
        return;
    };
    let t = tier_of(c);
    let Some(i) = live
        .iter()
        .filter(|c| tier_of(c) == t)
        .position(|c| c.id == id)
    else {
        return;
    };
    let key = tier_key(t);
    let size = super::pager::state(this, &key, super::tiers::TIER_PAGE).1;
    this.pages
        .insert(key, (super::pager::holding(i, size), size));
}
