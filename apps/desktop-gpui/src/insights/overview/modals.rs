//! Overview's three detail modals, as Svelte draws them:
//! - "What you worked on" (`CategoryDetailModal.svelte`): one thread per
//!   category (dot · label · "6 sessions · 4h 20m · across 3 days · mostly
//!   deep" · chevron), each expanding to every activity in it, newest first,
//!   with a focus hint and an "adjust" popover to correct its category / focus;
//! - "App usage" (`AppDetailModal.svelte`): every app, icon (or initial), name,
//!   share, a bar and time / frame count;
//! - "Focus detail" (`FocusDetailModal.svelte`): the split, the heat by time
//!   of day and the twelve longest focused sessions.
//!
//! The kit Modal closes on Esc, the scrim and ×; the body scrolls. Esc closes
//! an open adjust popover first (`close_top`), and the scrim can't close the
//! category modal while a correction saves.

use std::collections::HashMap;

use capture_types::{Activity, ActivityCategory, FocusLevel};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role, SharedString};

use super::{Figures, Fix};
use crate::insights::charts::{focus_split, heatmap, swatch};
use crate::insights::parts::{group_head, h2, label, mono_label};
use crate::insights::stats::{
    self, category_color, category_label, fmt_duration, focus_color, humanize_ms, CATEGORY_ORDER,
};
use crate::insights::{Insights, Modal};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::calendar::{capsule_label, period_of};
use crate::ui::dialog::Modal as Dialog;
use crate::ui::select::Select;
use crate::ui::Tracking as _;
use crate::ui::{app_icon, clock, kicker, nid};

pub fn modal(this: &Insights, p: &Palette, cx: &mut Context<Insights>) -> Option<AnyElement> {
    let m = this.modal?;
    let d = this.overview.as_ref().filter(|o| !o.stale)?;
    let f = super::figures(this, d, cx);
    let (a, b) = period_of(this.period.0, this.period.1);
    let range_label = capsule_label(Some(this.period.1), a, b, clock::now(cx).date());
    let off = clock::now(cx).offset();
    let me = cx.entity().downgrade();
    let on_close = move |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        let _ = me.update(cx, |i, cx| {
            // A stray scrim click can't close the modal while a correction saves.
            if i.modal == Some(Modal::Categories) && !i.ov_ui.correcting.is_empty() {
                return;
            }
            i.modal = None;
            i.ov_ui.adjusting = None;
            cx.notify();
        });
    };
    let body = |el: Div| {
        div()
            .id("insights-modal-body")
            .max_h(px(620.))
            .overflow_y_scroll()
            .px(px(18.))
            .pb(px(18.))
            .child(el)
    };
    let dialog = match m {
        Modal::Categories => Dialog::new("insights-modal", "What you worked on")
            .eyebrow(range_label)
            .child(body(categories(this, &f, off, p, cx))),
        Modal::Apps => Dialog::new("insights-modal", "App usage")
            .eyebrow(range_label)
            .child(body(apps(this, d, p))),
        // `.in-modal`: `color-mix(--app-bg 62%)`, not the others' overlay token.
        Modal::Focus => Dialog::new("insights-modal", "Focus detail")
            .scrim(crate::ui::alpha(p.bg, 0.62))
            .bare()
            .child(focus(&f, this.period.1, &range_label, off, p)),
    };
    Some(dialog.on_close(on_close).into_any_element())
}

/// `buildActivityThreads`: per category, most time first, uncategorized
/// always last; each thread's activities newest first.
pub struct Thread<'a> {
    pub category: Option<ActivityCategory>,
    pub acts: Vec<&'a Activity>,
    pub ms: i64,
    pub days: usize,
    pub dominant: Option<FocusLevel>,
}

pub fn threads(acts: &[Activity], off: time::UtcOffset) -> Vec<Thread<'_>> {
    let mut out: Vec<Thread> = Vec::new();
    for a in acts {
        match out.iter_mut().find(|t| t.category == a.category) {
            Some(t) => t.acts.push(a),
            None => out.push(Thread {
                category: a.category,
                acts: vec![a],
                ms: 0,
                days: 0,
                dominant: None,
            }),
        }
    }
    for t in &mut out {
        t.ms = t
            .acts
            .iter()
            .map(|a| (a.ended_at_ms - a.started_at_ms).max(0))
            .sum();
        let mut days: Vec<i64> = t
            .acts
            .iter()
            .map(|a| stats::start_of_day(a.started_at_ms, off))
            .collect();
        days.sort();
        days.dedup();
        t.days = days.len();
        // Most frequent focus; ties go deep → mixed → distracted.
        let count = |f: FocusLevel| t.acts.iter().filter(|a| a.focus == Some(f)).count();
        let mut best = (None, 0);
        for f in [FocusLevel::Deep, FocusLevel::Mixed, FocusLevel::Distracted] {
            if count(f) > best.1 {
                best = (Some(f), count(f));
            }
        }
        t.dominant = best.0;
        t.acts.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    }
    out.sort_by(|x, y| {
        x.category
            .is_none()
            .cmp(&y.category.is_none())
            .then(y.ms.cmp(&x.ms))
    });
    out
}

/// `threadStats`: "6 sessions · 4h 20m · across 3 days · mostly deep".
pub fn thread_stats(t: &Thread, day_range: bool) -> String {
    let n = t.acts.len();
    let mut parts = vec![
        format!("{n} {}", if n == 1 { "session" } else { "sessions" }),
        humanize_ms(t.ms),
    ];
    if !day_range {
        parts.push(format!(
            "across {} {}",
            t.days,
            if t.days == 1 { "day" } else { "days" }
        ));
    }
    if let Some(f) = t.dominant {
        parts.push(format!("mostly {}", focus_hint(f).to_lowercase()));
    }
    parts.join(" · ")
}

/// `focusHint`: "Scattered" is the app's label for distracted.
pub fn focus_hint(f: FocusLevel) -> &'static str {
    match f {
        FocusLevel::Deep => "Deep",
        FocusLevel::Mixed => "Mixed",
        FocusLevel::Distracted => "Scattered",
    }
}

const FOCUS_OPTIONS: [Option<FocusLevel>; 4] = [
    None,
    Some(FocusLevel::Deep),
    Some(FocusLevel::Mixed),
    Some(FocusLevel::Distracted),
];

fn categories(
    this: &Insights,
    f: &Figures,
    off: time::UtcOffset,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> Div {
    let ts = threads(&f.acts, off);
    if ts.is_empty() {
        return div()
            .py(px(8.))
            .text_size(text::SM)
            .text_color(p.text_muted)
            .child("No categorized activity in this range.");
    }
    let day_range = this.period.1 == crate::ui::calendar::Unit::Day;
    let mut col = div().flex().flex_col();
    for (ti, t) in ts.iter().enumerate() {
        let key = t.category;
        let open = this.ov_ui.expanded.contains(&key);
        let head = div()
            .id(nid("insights-thread", ti as u64))
            .test_support()
            .role(Role::Button)
            .aria_expanded(open)
            .flex()
            .items_center()
            .gap(px(9.))
            .py(px(10.))
            .cursor_pointer()
            .group("insights-thread")
            .child(
                div()
                    .size(px(8.))
                    .flex_none()
                    .rounded_full()
                    .bg(category_color(key, p)),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(p.text_strong)
                    .child(category_label(key)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(text::SM)
                    .text_color(p.text_muted)
                    .child(thread_stats(t, day_range)),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(p.text_faint)
                    .group_hover("insights-thread", |s| s.text_color(p.text_strong))
                    .child(crate::ui::icon(
                        if open {
                            gpui_kit::assets::IconName::ChevronDown.path()
                        } else {
                            gpui_kit::assets::IconName::ChevronRight.path()
                        },
                        px(13.),
                    )),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                let e = &mut this.ov_ui.expanded;
                match e.iter().position(|k| *k == key) {
                    Some(i) => {
                        e.remove(i);
                    }
                    None => e.push(key),
                }
                cx.notify();
            }));
        let mut thread = div()
            .flex()
            .flex_col()
            .when(ti > 0, |d| {
                d.border_t_1().border_dashed().border_color(p.border)
            })
            .child(head);
        if open {
            let rows = t
                .acts
                .iter()
                .enumerate()
                .map(|(i, a)| act_row(this, a, i > 0, off, p, cx));
            thread = thread.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .pl(px(17.))
                    .pb(px(8.))
                    .children(rows),
            );
        }
        col = col.child(thread);
    }
    col
}

fn act_row(
    this: &Insights,
    a: &Activity,
    rule: bool,
    off: time::UtcOffset,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let id = a.id;
    let busy = this.ov_ui.correcting.contains(&id);
    let editing = this.ov_ui.adjusting == Some(id);
    let when = format!(
        "{}, {} · {}",
        stats::month_day(a.started_at_ms, off),
        stats::clock(a.started_at_ms, off),
        humanize_ms((a.ended_at_ms - a.started_at_ms).max(0))
    );
    let adjust = div()
        .id(nid("insights-adjust", id as u64))
        .test_support()
        .role(Role::Button)
        .aria_expanded(editing)
        .flex()
        .items_center()
        .gap(px(4.))
        .pb(px(1.))
        .border_b_1()
        .border_dashed()
        .border_color(p.border_strong)
        .cursor_pointer()
        .whitespace_nowrap()
        .text_size(text::SM)
        .text_color(p.text_muted)
        .hover(|s| s.text_color(p.text_strong).border_color(p.border_hover))
        .child("adjust")
        .child(crate::ui::icon(
            if editing {
                gpui_kit::assets::IconName::ChevronUp.path()
            } else {
                gpui_kit::assets::IconName::ChevronDown.path()
            },
            px(9.),
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.ov_ui.adjusting = if this.ov_ui.adjusting == Some(id) {
                None
            } else {
                Some(id)
            };
            cx.notify();
        }));
    let pop = editing.then(|| {
        let cat_ix = a
            .category
            .and_then(|c| CATEGORY_ORDER.iter().position(|k| *k == c))
            .map_or(0, |i| i + 1);
        let focus_ix = FOCUS_OPTIONS
            .iter()
            .position(|f| *f == a.focus)
            .unwrap_or(0);
        let me = cx.entity().downgrade();
        let me2 = me.clone();
        let cat = Select::new(
            nid("insights-adjust-category", id as u64),
            "Category",
            std::iter::once("Uncategorized")
                .chain(CATEGORY_ORDER.iter().map(|k| category_label(Some(*k)))),
            Some(cat_ix),
        )
        .compact()
        .in_modal()
        .disabled(busy)
        .on_change(move |ix, _, cx| {
            let c = ix.checked_sub(1).map(|i| CATEGORY_ORDER[i]);
            let _ = me.update(cx, |this, cx| this.correct(id, Fix::Category(c), cx));
        });
        let focus = Select::new(
            nid("insights-adjust-focus", id as u64),
            "Focus",
            ["—", "Deep", "Mixed", "Scattered"],
            Some(focus_ix),
        )
        .compact()
        .in_modal()
        .disabled(busy)
        .on_change(move |ix, _, cx| {
            let f = FOCUS_OPTIONS[ix];
            let _ = me2.update(cx, |this, cx| this.correct(id, Fix::Focus(f), cx));
        });
        let corr = |name: &'static str, el: AnyElement| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(9.))
                        .tracking(0.05)
                        .text_color(p.text_subtle)
                        .child(name),
                )
                .child(div().w(px(132.)).flex_none().child(el))
        };
        crate::ui::popover::float_in_modal(
            div()
                .id(nid("insights-adjust-pop", id as u64))
                .role(Role::Group)
                .aria_label("Adjust activity")
                .absolute()
                .top(px(22.))
                .right_0()
                .min_w(px(220.))
                .flex()
                .flex_col()
                .gap(px(11.))
                .py(px(11.))
                .px(px(12.))
                .bg(p.surface)
                .border_1()
                .border_color(p.border)
                .rounded(radius::MD)
                .shadow(crate::ui::popover::popover_shadow(p))
                .occlude()
                .child(corr("CATEGORY", cat.into_any_element()))
                .child(corr("FOCUS", focus.into_any_element())),
        )
    });
    div()
        .id(nid("insights-act-row", id as u64))
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(9.))
        .when(rule, |d| {
            d.border_t_1().border_dashed().border_color(p.border)
        })
        .when(busy, |d| d.opacity(0.5))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .text_color(p.text_strong)
                        .child(a.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.text_muted)
                        .child(when),
                ),
        )
        .children(a.focus.map(|f| {
            div()
                .flex_none()
                .text_size(px(9.5))
                .tracking(0.06)
                .text_color(p.text_faint)
                .child(focus_hint(f).to_uppercase())
        }))
        .child(div().relative().flex_none().child(adjust).children(pop))
        .into_any_element()
}

fn apps(this: &Insights, d: &crate::insights::model::OverviewData, p: &Palette) -> Div {
    let rows = d
        .usage
        .as_ref()
        .map(|u| stats::listed_apps(&u.time_per_app))
        .unwrap_or_default();
    if rows.is_empty() {
        return div()
            .py(px(8.))
            .text_size(text::SM)
            .text_color(p.text_muted)
            .child("No tracked app time in this range.");
    }
    let max = rows.iter().map(|a| a.active_ms).max().unwrap_or(0).max(1) as f32;
    let total: i64 = rows.iter().map(|a| a.active_ms.max(0)).sum();
    let icons: &HashMap<String, std::path::PathBuf> = &this.ov_ui.icons;
    div()
        .flex()
        .flex_col()
        .children(rows.into_iter().enumerate().map(|(i, a)| {
            let key = a.app_bundle_id.clone().unwrap_or_else(|| a.app.clone());
            let path = icons.get(&key).cloned();
            let share = if total > 0 {
                format!("{}%", stats::fmt_pct(a.active_ms, total))
            } else {
                "0%".into()
            };
            let frames = a.frame_count;
            let tile = {
                let bg = p.surface_hover;
                let fg = p.text_muted;
                move || {
                    div()
                        .size(px(22.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(bg)
                        .text_size(px(11.))
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(fg)
                }
            };
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(9.))
                .when(i > 0, |d| {
                    d.border_t_1().border_dashed().border_color(p.border)
                })
                .child(
                    div()
                        .size(px(22.))
                        .flex_none()
                        .rounded(px(6.))
                        .overflow_hidden()
                        .child(app_icon::icon_or_initial(path, &a.app, 22., tile)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.5))
                                        .text_color(p.text_strong)
                                        .child(a.app.clone()),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.5))
                                        .text_color(p.text_muted)
                                        .child(share),
                                ),
                        )
                        .child(
                            div()
                                .h(px(4.))
                                .rounded_full()
                                .bg(p.surface_hover)
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(p.accent)
                                        .w(relative(a.active_ms.max(0) as f32 / max)),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(text::BASE)
                                .text_color(p.text_strong)
                                .child(humanize_ms(a.active_ms)),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(p.text_faint)
                                .child(format!(
                                    "{} {}",
                                    stats::thousands(frames),
                                    if frames == 1 { "frame" } else { "frames" }
                                )),
                        ),
                )
        }))
}

fn focus(
    f: &Figures,
    unit: crate::ui::calendar::Unit,
    range_label: &str,
    off: time::UtcOffset,
    p: &Palette,
) -> Div {
    let c = stats::focus_counts(&f.acts);
    let mut sessions: Vec<&Activity> = f.acts.iter().filter(|a| a.focus.is_some()).collect();
    sessions.sort_by_key(|a| std::cmp::Reverse((a.ended_at_ms - a.started_at_ms).max(0)));
    sessions.truncate(12);
    let note = |s: &'static str| label(s, p).py(px(4.));
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .px(space::S6)
                .pt(space::S5)
                .pb(space::S4)
                .border_b_1()
                .border_color(p.hairline)
                .child(kicker(&format!("Focus · {range_label}"), p))
                .child(h2("Focus detail", p))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(
                            div()
                                .font_family(FONT_MONO)
                                .text_color(p.text_strong)
                                .child(c.n.to_string()),
                        )
                        .child(SharedString::from(format!(
                            "activit{} with a focus read",
                            if c.n == 1 { "y" } else { "ies" }
                        ))),
                ),
        )
        .child(
            div()
                .id("insights-modal-body")
                .max_h(px(560.))
                .overflow_y_scroll()
                .px(space::S6)
                .pb(space::S6)
                .child(group_head("Split", None, p))
                .child(if c.n > 0 {
                    focus_split(c, p)
                } else {
                    note("No focus signal in this range.")
                })
                .child(group_head("By time of day", None, p))
                .child(heatmap(&stats::focus_heat(&f.acts, f.range, unit, off), p))
                .child(group_head("Longest sessions", None, p))
                .when(sessions.is_empty(), |d| {
                    d.child(note("No focused sessions in this range."))
                })
                .children(sessions.into_iter().enumerate().map(|(i, a)| {
                    let fl = a.focus.unwrap_or(FocusLevel::Mixed);
                    // `.session` border-top, none under the heading (`.in-group + .session`).
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S3)
                        .py(px(7.))
                        .when(i > 0, |d| d.border_t_1())
                        .border_color(p.hairline)
                        .text_size(text::BASE)
                        .child(swatch(focus_color(fl, p), 8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(p.text_strong)
                                .child(a.title.clone()),
                        )
                        .child(mono_label(
                            format!(
                                "{} {} · {}",
                                stats::weekday(a.started_at_ms, off),
                                stats::clock(a.started_at_ms, off),
                                focus_hint(fl)
                            ),
                            p,
                        ))
                        .child(
                            div()
                                .w(px(64.))
                                .text_right()
                                .font_family(FONT_MONO)
                                .text_size(text::SM)
                                .text_color(p.text_strong)
                                .child(fmt_duration((a.ended_at_ms - a.started_at_ms).max(0))),
                        )
                })),
        )
}

impl Insights {
    /// Ask once, off the main thread, for the listed apps' icons (the All-apps modal).
    pub fn request_app_icons(&mut self, cx: &mut Context<Self>) {
        let Some(u) = self.overview.as_ref().and_then(|o| o.usage.as_ref()) else {
            return;
        };
        let ask: Vec<String> = u
            .time_per_app
            .iter()
            .map(|a| a.app_bundle_id.clone().unwrap_or_else(|| a.app.clone()))
            .filter(|k| !k.is_empty() && self.ov_ui.icons_asked.insert(k.clone()))
            .collect();
        if ask.is_empty() {
            return;
        }
        self.write(
            cx,
            move |b| b.app_icons(&ask),
            |this, got, _| this.ov_ui.icons.extend(got),
        );
    }
}
