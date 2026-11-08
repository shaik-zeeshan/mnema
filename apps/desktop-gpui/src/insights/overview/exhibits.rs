//! Overview's exhibits row (`OverviewExhibits.svelte`) — "Where the <unit>
//! went" columns + category list beside Focus and Time in apps — and the feed
//! (`OverviewFeed.svelte`): "What changed" in the dossier, "Needs attention".

use capture_types::{Conclusion, ConclusionStatus};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Div, Role, SharedString, Window};

use super::Figures;
use crate::insights::charts::{
    self, focus_split, heatmap, sparkline, spec_rows, stacked_bar, swatch,
};
use crate::insights::model::OverviewData;
use crate::insights::parts::{group_head, h2, label, mono_label, panel, prose, sec};
use crate::insights::stats::{self, category_color, category_label, fmt_duration, CATEGORY_ORDER};
use crate::insights::subjects::tiers::pct;
use crate::insights::{Insights, Modal};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::calendar::Unit;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{clock, icon, kicker, nid, Button, Tone};

fn compact(
    icon_path: SharedString,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    p: &Palette,
) -> Div {
    empty(
        EmptyLayout::Compact,
        Tone::Neutral,
        icon_path,
        title,
        body,
        Vec::new(),
        p,
    )
}

pub fn exhibits(
    this: &Insights,
    d: &OverviewData,
    f: &Figures,
    p: &Palette,
    _: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let unit = this.period.1;
    let off = clock::now(cx).offset();
    let now = clock::now_ms(cx);
    let per = match (f.cap_only, unit) {
        (true, Unit::Day) => "app time per hour",
        (true, _) => "app time per day",
        (false, Unit::Day) => "activity minutes per hour, by category",
        (false, _) => "activity hours per day, by category",
    };
    let cols = if f.cap_only {
        let heat = d
            .usage
            .as_ref()
            .map(|u| u.activity_heatmap.as_slice())
            .unwrap_or(&[]);
        stats::app_time_columns(heat, f.tracked, f.range, unit, now, off)
    } else {
        stats::activity_columns(&f.acts, f.range, unit, now, off)
    };
    let cats = stats::category_rows(&f.acts, f.range.0, f.range.1);
    let cat_total: i64 = cats.iter().map(|c| c.ms).sum();
    let top_ms = cats.first().map_or(1, |c| c.ms);
    let scope = super::scope(this, cx);
    let cat_list: Div = if f.nothing {
        compact(
            IconName::Inbox.path(),
            format!("Nothing captured {scope}"),
            "Columns fill in as you record.",
            p,
        )
    } else if f.cap_only {
        compact(
            IconName::CircleOff.path(),
            "Categories need AI",
            "Activities are what carry a category; usage charts only know apps.",
            p,
        )
    } else if cats.is_empty() {
        compact(
            IconName::Inbox.path(),
            "No categories yet",
            "Categories appear once activities are written up.",
            p,
        )
    } else {
        div()
            .flex()
            .flex_col()
            .children(cats.iter().enumerate().map(|(i, c)| {
                let key = c.category;
                div()
                    .id(nid("insights-cat", i as u64))
                    .role(Role::Button)
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .py(px(8.))
                    .px(px(6.))
                    .mx(px(-6.))
                    .rounded(radius::SM)
                    .cursor_pointer()
                    .text_size(text::BASE)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text)
                    .hover(|s| s.bg(p.surface_hover))
                    .child(swatch(category_color(key, p), 8.))
                    .child(div().w(px(112.)).child(category_label(key)))
                    .child(charts::bar(
                        c.ms as f32 / top_ms as f32,
                        category_color(key, p),
                        6.,
                        p,
                    ))
                    .child(
                        div()
                            .w(px(64.))
                            .text_right()
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .text_color(p.text_strong)
                            .child(fmt_duration(c.ms)),
                    )
                    .child(
                        div()
                            .w(px(40.))
                            .text_right()
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(format!("{}%", stats::fmt_pct(c.ms, cat_total))),
                    )
                    .child(
                        div()
                            .text_color(p.text_faint)
                            .child(icon(IconName::ChevronRight.path(), px(13.))),
                    )
                    .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                        this.highlight = on.then_some(key);
                        cx.notify();
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.modal = Some(Modal::Categories);
                        cx.notify();
                    }))
            }))
    };
    let week = panel(p)
        .flex_1()
        .min_w_0()
        .p(space::S5)
        .child(sec(
            kicker(&format!("Where the {} went", unit.name()), p),
            Some(label(per, p).into_any_element()),
        ))
        .child(stacked_bar(&cols, unit, this.highlight, p))
        .child(
            div()
                .mt(space::S5)
                .border_t_1()
                .border_color(p.hairline)
                .child(cat_list),
        );
    let focus = stats::focus_counts(&f.acts);
    let detail = |id: &'static str, text_: String, m: Modal| {
        Button::new(id, text_)
            .ghost()
            .sm()
            .icon(IconName::ArrowRight.path())
            .icon_after()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.modal = Some(m);
                cx.notify();
            }))
            .into_any_element()
    };
    let focus_body: Div = if f.cap_only {
        compact(
            IconName::CircleOff.path(),
            "Focus needs AI",
            "Deep / mixed / scattered is judged per activity.",
            p,
        )
    } else if focus.n == 0 {
        compact(
            IconName::Inbox.path(),
            "No focus signal yet",
            "Appears once activities are written up.",
            p,
        )
    } else {
        div()
            .child(focus_split(focus, p))
            .child(heatmap(&stats::focus_heat(&f.acts, f.range, unit, off), p))
    };
    let apps_body: Div = if f.apps.is_empty() {
        compact(
            IconName::Inbox.path(),
            "No tracked app time in this range",
            "",
            p,
        )
    } else {
        let top = f.apps[0].active_ms.max(1) as f32;
        spec_rows(
            f.apps
                .iter()
                .take(6)
                .map(|a| {
                    (
                        a.app.clone().into(),
                        a.active_ms as f32 / top,
                        fmt_duration(a.active_ms).into(),
                    )
                })
                .collect(),
            p.chart_grey_3,
            p,
        )
    };
    let n_apps = f.apps.len();
    let side = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(space::S6)
        .child(
            div()
                .child(sec(
                    kicker("Focus", p),
                    (!f.cap_only && focus.n > 0).then(|| {
                        detail("insights-focus-detail", "Focus detail".into(), Modal::Focus)
                    }),
                ))
                .child(focus_body),
        )
        .child(
            div()
                .child(sec(
                    kicker("Time in apps", p),
                    (n_apps > 0).then(|| {
                        detail(
                            "insights-apps-all",
                            format!("All {n_apps} {}", if n_apps == 1 { "app" } else { "apps" }),
                            Modal::Apps,
                        )
                    }),
                ))
                .child(apps_body),
        );
    div()
        .flex()
        .gap(space::S6)
        .mb(space::S7)
        .child(
            div()
                .flex_grow(1.45)
                .flex_basis(px(0.))
                .min_w_0()
                .flex()
                .child(week),
        )
        .child(
            div()
                .flex_grow(1.)
                .flex_basis(px(0.))
                .min_w_0()
                .flex()
                .child(side),
        )
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Formed,
    Strengthened,
    Fading,
}

/// Each live conclusion in at most one group, latest change first; one cap of 10.
fn deltas(cs: &[Conclusion], (s, e): (i64, i64)) -> Vec<(Kind, usize, Vec<Conclusion>)> {
    let inr = |ms: i64| ms >= s && ms < e;
    let mut groups: [(Kind, Vec<Conclusion>); 3] = [
        (Kind::Formed, Vec::new()),
        (Kind::Strengthened, Vec::new()),
        (Kind::Fading, Vec::new()),
    ];
    for c in cs
        .iter()
        .filter(|c| c.status != ConclusionStatus::Dismissed)
    {
        if inr(c.formed_at_ms) {
            groups[0].1.push(c.clone());
        } else if c.status == ConclusionStatus::Visible && inr(c.last_supported_at_ms) {
            groups[1].1.push(c.clone());
        } else if c.status == ConclusionStatus::Faded && inr(c.updated_at_ms) {
            groups[2].1.push(c.clone());
        }
    }
    let at = |k: Kind, c: &Conclusion| match k {
        Kind::Formed => c.formed_at_ms,
        Kind::Strengthened => c.last_supported_at_ms,
        Kind::Fading => c.updated_at_ms,
    };
    let mut left = 10usize;
    groups
        .into_iter()
        .map(|(k, mut rows)| {
            rows.sort_by(|a, b| {
                at(k, b)
                    .cmp(&at(k, a))
                    .then(b.confidence.total_cmp(&a.confidence))
            });
            let all = rows.len();
            rows.truncate(left);
            left -= rows.len();
            (k, all, rows)
        })
        .filter(|g| !g.2.is_empty())
        .collect()
}

pub fn feed(
    this: &Insights,
    d: &OverviewData,
    f: &Figures,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let off = clock::now(cx).offset();
    let unit = this.period.1;
    let scope = super::scope(this, cx);
    let groups = deltas(&d.conclusions, f.range);
    let day = |ms: i64| {
        if unit == Unit::Month {
            stats::month_day(ms, off)
        } else {
            stats::weekday(ms, off).to_string()
        }
    };
    let changed: Div = if let Some(e) = &d.engine_error {
        empty(
            EmptyLayout::Panel,
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "Couldn't load your engine data.",
            format!("{e} Nothing is lost."),
            Vec::new(),
            p,
        )
    } else if groups.is_empty() {
        let open = Button::new("insights-open-subjects", "Open Subjects")
            .sm()
            .on_click(
                cx.listener(|this, _, _, cx| this.set_tab(crate::insights::Tab::Subjects, cx)),
            );
        empty(
            EmptyLayout::Panel,
            Tone::Neutral,
            IconName::Inbox.path(),
            format!("Nothing changed {scope}."),
            "No conclusions formed, strengthened or faded in this range. Your standing dossier lives on Subjects.",
            vec![open.into_any_element()],
            p,
        )
    } else {
        let mut col = div().flex().flex_col();
        for (k, all, rows) in groups {
            let head = match k {
                Kind::Formed => format!("Formed this {} · {all}", unit.name()),
                Kind::Strengthened => format!("Strengthened · {all}"),
                Kind::Fading => format!("Started fading · {all}"),
            };
            col = col.child(group_head(&head, None, p));
            for c in rows {
                let pts: Vec<f64> = d
                    .histories
                    .get(&c.subject)
                    .and_then(|h| h.get(&c.id))
                    .map(|h| h.iter().map(|s| s.confidence).collect())
                    .filter(|v: &Vec<f64>| v.len() >= 2)
                    .unwrap_or_else(|| vec![c.confidence, c.confidence]);
                let fading = k == Kind::Fading;
                let note = match k {
                    Kind::Formed => format!("formed {}", day(c.formed_at_ms)),
                    Kind::Strengthened => format!("supported {}", day(c.last_supported_at_ms)),
                    Kind::Fading => "fading".into(),
                };
                let subject = c.subject.clone();
                col = col.child(
                    div()
                        .id(nid("insights-change", c.id as u64))
                        .role(Role::Button)
                        .flex()
                        .items_center()
                        .gap(space::S4)
                        .py(px(10.))
                        .border_b_1()
                        .border_color(p.hairline)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.wash))
                        .when(fading, |d| d.opacity(0.6))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(text::MD)
                                        .text_color(p.text_strong)
                                        .child(c.statement.clone()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(space::S2)
                                        .text_size(text::SM)
                                        .text_color(p.text_subtle)
                                        .child(crate::ui::chip::chip(
                                            c.subject.clone(),
                                            crate::ui::chip::ChipKind::Neutral,
                                            p,
                                        ))
                                        .when(k == Kind::Formed, |d| {
                                            d.child(div().text_color(p.accent).child("new"))
                                        })
                                        .when(fading, |d| {
                                            d.child("below the display floor — kept for history")
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .w(px(120.))
                                .h(px(32.))
                                .flex_none()
                                .child(sparkline(vec![(p.text_strong, fading, pts)])),
                        )
                        .child(
                            div()
                                .w(px(90.))
                                .flex()
                                .flex_col()
                                .items_end()
                                .font_family(FONT_MONO)
                                .child(
                                    div()
                                        .text_size(text::MD)
                                        .text_color(p.text_strong)
                                        .child(format!("{}%", pct(c.confidence))),
                                )
                                .child(
                                    div()
                                        .text_size(text::XS)
                                        .text_color(if fading {
                                            p.focus_distracted
                                        } else {
                                            p.focus_deep
                                        })
                                        .child(note),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_subject(subject.clone(), cx)
                        })),
                );
            }
        }
        col
    };
    let attention: Vec<&capture_types::Activity> = {
        let mut v: Vec<_> = f.acts.iter().filter(|a| a.category.is_none()).collect();
        v.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
        v
    };
    let attn_body: Div = if attention.is_empty() {
        empty(
            EmptyLayout::Compact,
            Tone::Ok,
            IconName::Check.path(),
            "All caught up",
            "Every activity in this range has a category.",
            Vec::new(),
            p,
        )
    } else {
        div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(prose("Mnema couldn't place these. One tap teaches it.", p))
            .children(attention.iter().take(5).map(|a| {
                let id = a.id;
                let when = format!(
                    "{} {} · {}",
                    stats::weekday(a.started_at_ms, off),
                    stats::clock(a.started_at_ms, off),
                    fmt_duration(a.ended_at_ms - a.started_at_ms)
                );
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S2)
                    .child(
                        div()
                            .text_size(text::MD)
                            .font_weight(theme::W_SEMIBOLD)
                            .text_color(p.text_strong)
                            .child(a.title.clone()),
                    )
                    .child(mono_label(when, p))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(
                        CATEGORY_ORDER.iter().map(|k| {
                            let k = *k;
                            Button::new(
                                nid(&format!("insights-tag-{id}"), k as u64),
                                category_label(Some(k)),
                            )
                            .sm()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.correct_category(id, k, cx)),
                            )
                        }),
                    ))
            }))
    };
    let untagged = (!attention.is_empty())
        .then(|| mono_label(format!("{} untagged", attention.len()), p).into_any_element());
    div()
        .flex()
        .items_start()
        .gap(space::S6)
        .child(
            div()
                .flex_grow(1.45)
                .flex_basis(px(0.))
                .min_w_0()
                .child(sec(
                    div()
                        .flex()
                        .items_end()
                        .gap(space::S3)
                        .child(h2("What changed", p))
                        .child(label(format!("in your dossier {scope}"), p)),
                    None,
                ))
                .child(changed),
        )
        .child(
            panel(p)
                .flex_grow(1.)
                .flex_basis(px(0.))
                .min_w_0()
                .p(space::S5)
                .child(sec(kicker("Needs attention", p), untagged))
                .child(attn_body),
        )
        .into_any_element()
}
