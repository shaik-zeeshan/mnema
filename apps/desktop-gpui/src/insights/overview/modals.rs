//! Overview's three detail modals: "What you worked on" (categories as
//! threads, `CategoryDetailModal`), "App usage" (`AppDetailModal`) and "Focus
//! detail" (`FocusDetailModal`). Scrim, Esc and × close (the kit Modal).

use capture_types::{Activity, ActivityCategory};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Div};

use super::Figures;
use crate::insights::charts::{focus_split, heatmap, mini_bars, swatch};
use crate::insights::parts::{group_head, mono_label};
use crate::insights::stats::{
    self, category_color, category_label, fmt_duration, focus_color, focus_label, humanize_ms,
};
use crate::insights::{Insights, Modal};
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::calendar::{capsule_label, period_of};
use crate::ui::clock;
use crate::ui::dialog::Modal as Dialog;

pub fn modal(this: &Insights, p: &Palette, cx: &mut Context<Insights>) -> Option<AnyElement> {
    let m = this.modal?;
    let d = this.overview.as_ref()?;
    let f = super::figures(this, d, cx);
    let (a, b) = period_of(this.period.0, this.period.1);
    let range_label = capsule_label(Some(this.period.1), a, b, clock::now(cx).date());
    let off = clock::now(cx).offset();
    let (title, body) = match m {
        Modal::Categories => ("What you worked on", categories(&f, off, p)),
        Modal::Apps => ("App usage", apps(&f, p)),
        Modal::Focus => ("Focus detail", focus(&f, this.period.1, off, p)),
    };
    let dialog = Dialog::new("insights-modal", title)
        .lead(
            div()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(range_label),
        )
        .child(
            div()
                .px(space::S5)
                .pb(space::S5)
                .max_h(px(520.))
                .child(body),
        )
        .on_close({
            let this = cx.entity().downgrade();
            move |_, cx| {
                this.update(cx, |i, cx| {
                    i.modal = None;
                    cx.notify();
                })
                .ok();
            }
        });
    Some(dialog.into_any_element())
}

/// `buildActivityThreads`: per category, most time first, uncategorized last.
fn threads(acts: &[Activity]) -> Vec<(Option<ActivityCategory>, Vec<&Activity>, i64)> {
    let mut out: Vec<(Option<ActivityCategory>, Vec<&Activity>, i64)> = Vec::new();
    for a in acts {
        let ms = (a.ended_at_ms - a.started_at_ms).max(0);
        match out.iter_mut().find(|t| t.0 == a.category) {
            Some(t) => {
                t.1.push(a);
                t.2 += ms;
            }
            None => out.push((a.category, vec![a], ms)),
        }
    }
    out.sort_by(|x, y| x.0.is_none().cmp(&y.0.is_none()).then(y.2.cmp(&x.2)));
    for t in &mut out {
        t.1.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    }
    out
}

fn categories(f: &Figures, off: time::UtcOffset, p: &Palette) -> Div {
    let ts = threads(&f.acts);
    if ts.is_empty() {
        return div()
            .text_color(p.text_muted)
            .child("No categorized activity in this range.");
    }
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .children(ts.into_iter().map(|(c, acts, ms)| {
            let n = acts.len();
            div()
                .flex()
                .flex_col()
                .gap(space::S1)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S2)
                        .py(px(6.))
                        .child(swatch(category_color(c, p), 8.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(text::MD)
                                .font_weight(theme::W_SEMIBOLD)
                                .text_color(p.text_strong)
                                .child(category_label(c)),
                        )
                        .child(mono_label(
                            format!(
                                "{} · {n} session{}",
                                humanize_ms(ms),
                                if n == 1 { "" } else { "s" }
                            ),
                            p,
                        )),
                )
                .children(acts.into_iter().take(4).map(|a| {
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S3)
                        .pl(space::S4)
                        .text_size(text::BASE)
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_color(p.text)
                                .child(a.title.clone()),
                        )
                        .child(mono_label(
                            format!(
                                "{} · {}",
                                stats::clock(a.started_at_ms, off),
                                humanize_ms(a.ended_at_ms - a.started_at_ms)
                            ),
                            p,
                        ))
                }))
        }))
}

fn apps(f: &Figures, p: &Palette) -> Div {
    if f.apps.is_empty() {
        return div()
            .text_color(p.text_muted)
            .child("No tracked app time in this range.");
    }
    let total: i64 = f.apps.iter().map(|a| a.active_ms).sum();
    let items: Vec<_> = f
        .apps
        .iter()
        .map(|a| {
            (
                a.app.clone().into(),
                a.active_ms as f64,
                format!(
                    "{} · {}%",
                    humanize_ms(a.active_ms),
                    stats::fmt_pct(a.active_ms, total)
                )
                .into(),
            )
        })
        .collect();
    mini_bars(&items, p)
}

fn focus(f: &Figures, unit: crate::ui::calendar::Unit, off: time::UtcOffset, p: &Palette) -> Div {
    let c = stats::focus_counts(&f.acts);
    let mut sessions: Vec<&Activity> = f.acts.iter().filter(|a| a.focus.is_some()).collect();
    sessions.sort_by_key(|a| std::cmp::Reverse(a.ended_at_ms - a.started_at_ms));
    div()
        .flex()
        .flex_col()
        .child(mono_label(
            format!(
                "{} activit{} with a focus read",
                c.n,
                if c.n == 1 { "y" } else { "ies" }
            ),
            p,
        ))
        .child(group_head("Split", None, p))
        .child(focus_split(c, p))
        .child(group_head("By time of day", None, p))
        .child(heatmap(&stats::focus_heat(&f.acts, f.range, unit, off), p))
        .child(group_head("Longest sessions", None, p))
        .children(sessions.into_iter().take(5).map(|a| {
            let fl = a.focus.unwrap_or(capture_types::FocusLevel::Mixed);
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .py(px(6.))
                .text_size(text::BASE)
                .child(swatch(focus_color(fl, p), 8.))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_color(p.text)
                        .child(a.title.clone()),
                )
                .child(mono_label(
                    format!(
                        "{} · {}",
                        stats::clock(a.started_at_ms, off),
                        focus_label(fl)
                    ),
                    p,
                ))
                .child(
                    div()
                        .w(px(64.))
                        .text_right()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .child(fmt_duration(a.ended_at_ms - a.started_at_ms)),
                )
        }))
}
