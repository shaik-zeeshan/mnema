//! Subjects (`Subjects.svelte`): the dossier by conviction. Filter + the
//! Conviction / Movement switch on the bar; the lede (headline over the tiers,
//! four tier counts); the conviction map with 14-day tails; the tiered list
//! beside "Moving this fortnight" and how conviction works. A subject opens
//! its page (`detail.rs`) with the push-drawer of evidence.

mod detail;
mod drawer;
pub mod tiers;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role, Window};

pub use drawer::{drawer, step as step_conclusion};

use super::charts::sparkline;
use super::parts::{display, inset, label, lede, panel, prose, read, sec, stat_text};
use super::Insights;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::segmented::Segmented;
use crate::ui::{alpha, clock, icon, kicker, nid, Tone};
use tiers::{
    conviction_tier, headline, list_groups, move_pts, page_label, page_slice, pct, rank_movers,
    subject_rows, tier_names, Axis, SubjectRow, Tier, DISPLAY_FLOOR, INITIAL_BASE, MAP_PAGE,
    STRONGLY_HELD, TIER_PAGE,
};

pub fn controls(this: &Insights, p: &Palette, cx: &mut Context<Insights>) -> Vec<AnyElement> {
    if !this
        .subjects
        .as_ref()
        .is_some_and(|s| !s.conclusions.is_empty())
    {
        return Vec::new();
    }
    let search = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .w(px(220.))
        .h(size::H_MD)
        .px(space::S3)
        .rounded(radius::MD)
        .border_1()
        .border_color(p.border)
        .bg(p.input_recess)
        .text_color(p.text_subtle)
        .child(icon(IconName::Search.path(), px(14.)))
        .child(
            div().flex_1().min_w_0().text_color(p.text_strong).child(
                Input::new(&this.filter)
                    .id("insights-filter")
                    .aria_label("Filter subjects")
                    .appearance(false)
                    .px_0()
                    .text_size(text::BASE)
                    .h(size::H_MD - px(2.)),
            ),
        );
    let axis = if this.axis == Axis::Conviction { 0 } else { 1 };
    let entity = cx.entity().downgrade();
    let seg = Segmented::new(
        "insights-axis",
        "Group by",
        ["Conviction", "Movement"],
        axis,
    )
    .md()
    .on_change(move |ix, _, cx| {
        entity
            .update(cx, |this, cx| {
                this.axis = if ix == 0 {
                    Axis::Conviction
                } else {
                    Axis::Movement
                };
                cx.notify();
            })
            .ok();
    });
    vec![search.into_any_element(), seg.into_any_element()]
}

pub fn render(
    this: &mut Insights,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    if this.subject.is_some() {
        return detail::render(this, p, window, cx);
    }
    let Some(d) = this.subjects.clone() else {
        return div()
            .id("insights-subjects-loading")
            .child(crate::ui::skeleton::lines(4, p))
            .into_any_element();
    };
    let center = |el: Div| {
        div()
            .pt(space::S8)
            .flex()
            .justify_center()
            .child(el)
            .into_any_element()
    };
    if let Some(e) = &d.error {
        log::debug!("insights: subjects load failed ({} chars)", e.len());
        return center(empty(
            EmptyLayout::Center,
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "Couldn't load Subjects.",
            "Reading your conclusions failed. Nothing is lost.",
            Vec::new(),
            p,
        ));
    }
    let now = clock::now_ms(cx);
    let rows = subject_rows(&d.conclusions, &d.histories, now);
    if rows.is_empty() {
        let ctx = this.status.ctx.as_ref();
        let dismissed = ctx.map_or(0, |c| c.dismissed_count);
        let backfilling = ctx.is_some_and(|c| c.backfilling);
        let acts = ctx.map_or(0, |c| c.activity_count);
        let (title, body) = if backfilling {
            ("No subjects yet.".to_string(), "As Mnema forms views about you, each one appears here with its own confidence over time. It's backfilling your history now — a view needs at least two supporting activities.".to_string())
        } else if dismissed > 0 {
            ("No subjects to show right now.".into(), format!("You've dismissed {dismissed}. Restoring one in Context only lifts the block — it forms again only if recent activity supports it."))
        } else {
            let tail = if acts > 0 {
                format!(
                    " {acts} {} summarized so far.",
                    if acts == 1 {
                        "activity is"
                    } else {
                        "activities are"
                    }
                )
            } else {
                String::new()
            };
            ("No subjects yet.".into(), format!("As Mnema forms views about you, each one appears here with its own confidence over time. A view needs at least two supporting activities.{tail}"))
        };
        return center(empty(
            EmptyLayout::Center,
            Tone::Neutral,
            IconName::Inbox.path(),
            title,
            body,
            Vec::new(),
            p,
        ));
    }
    let q = this.filter.read(cx).value().trim().to_string();
    let matches = (!q.is_empty()).then(|| tiers::filter(&rows, &q));
    let listed: Vec<&SubjectRow> = matches.clone().unwrap_or_else(|| rows.iter().collect());
    let names = tier_names(&rows);
    let visible = d
        .conclusions
        .iter()
        .filter(|c| c.status == capture_types::ConclusionStatus::Visible)
        .count();
    let (lead, tail) = headline(&names);
    let lede_el = lede(&format!("Your dossier · {} subjects · {visible} conclusions", rows.len()), p)
        .child(display(lead, Some(tail), p))
        .child(prose("What Mnema has come to believe about you. Each subject is scored by its strongest conclusion; confidence rises with fresh evidence, halves every 30 days without it, and drops faster when activity contradicts it.", p));
    let tiles = [
        ("Strongly held", "lead ≥ 68%"),
        ("Forming", "30 – 68%"),
        ("Just taking shape", "under 30%"),
        ("Fading", "kept for history"),
    ];
    let stats = tiles
        .iter()
        .zip(names.iter())
        .map(|((l, note), n)| {
            stat_text(l, &n.len().to_string(), Some(note.to_string()), p).into_any_element()
        })
        .collect();
    let groups = list_groups(&rows, this.axis, matches);
    div()
        .flex()
        .flex_col()
        .child(read(lede_el, stats, p))
        .child(map(&listed, &q, p, cx))
        .child(
            div()
                .flex()
                .items_start()
                .gap(space::S6)
                .child(
                    div()
                        .flex_grow(1.45)
                        .flex_basis(px(0.))
                        .min_w_0()
                        .child(list(this, groups, rows.len(), p, cx)),
                )
                .child(
                    side(&listed, p, cx)
                        .flex_grow(1.)
                        .flex_basis(px(0.))
                        .min_w_0(),
                ),
        )
        .into_any_element()
}

/// `ConvictionMap.svelte`: bands, axis, and one row per subject (top 12).
fn map(listed: &[&SubjectRow], q: &str, p: &Palette, cx: &mut Context<Insights>) -> AnyElement {
    const NAME_W: f32 = 168.;
    const VAL_W: f32 = 104.;
    let (floor, base, held) = (pct(DISPLAY_FLOOR), pct(INITIAL_BASE), pct(STRONGLY_HELD));
    let legend = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .text_size(text::SM)
        .text_color(p.text_subtle)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(div().size(px(8.)).rounded_full().bg(p.text_strong))
                .child("now"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .border_1()
                        .border_color(p.chart_grey_4),
                )
                .child("14 days ago"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(div().w(px(2.)).h(px(10.)).bg(p.chart_grey_3))
                .child("other conclusions"),
        );
    let mut panel_el = panel(p)
        .id("insights-map")
        .p(space::S5)
        .mb(space::S7)
        .child(sec(
            kicker("Conviction map", p),
            Some(legend.into_any_element()),
        ));
    if listed.is_empty() {
        return panel_el.child(empty(EmptyLayout::Panel, Tone::Neutral, IconName::Search.path(), format!("No subjects match “{q}”."), "Filter looks at subject names and the wording of each belief. Try a shorter term, or clear it.", Vec::new(), p)).into_any_element();
    }
    let at = |v: i64| relative(v as f32 / 100.);
    let bands = [
        (0, floor, "fading", true, false),
        (floor, base, "taking shape", false, false),
        (base, held, "forming", false, false),
        (held, 100, "strongly held", false, true),
    ];
    let band_layer = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(NAME_W))
        .right(px(VAL_W))
        .children(bands.map(|(a, b, l, fade, firm)| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(at(a))
                .w(at(b - a))
                .border_l_1()
                .border_dashed()
                .border_color(p.hairline)
                .when(firm, |d| d.bg(alpha(p.fg, 0.025)))
                .when(fade, |d| d.bg(alpha(p.fg, 0.015)))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(8.))
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_faint)
                        .child(l.to_uppercase()),
                )
        }));
    let axis = div()
        .flex()
        .h(px(44.))
        .child(div().w(px(NAME_W)).flex_none())
        .child(
            div()
                .relative()
                .flex_1()
                .h_full()
                .children([0, floor, base, held, 100].map(|v| {
                    div()
                        .absolute()
                        .bottom(px(4.))
                        .left(at(v))
                        .ml(px(-20.))
                        .w(px(40.))
                        .flex()
                        .justify_center()
                        .whitespace_nowrap()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_faint)
                        .child(format!("{v}%"))
                })),
        )
        .child(div().w(px(VAL_W)).flex_none());
    let rows = listed.iter().take(MAP_PAGE).enumerate().map(|(i, r)| {
        let c = pct(r.top);
        let others: Vec<i64> = r
            .conclusions
            .iter()
            .skip(1)
            .map(|x| pct(x.confidence))
            .collect();
        let delta = r.then.map(|t| c - pct(t));
        let (dtext, dcol) = match delta {
            None => ("new".to_string(), p.accent),
            Some(d) if d > 0 => (format!("+{d}"), p.focus_deep),
            Some(d) if d < 0 => (format!("−{}", -d), p.focus_distracted),
            Some(_) => ("±0".into(), p.text_subtle),
        };
        let subject = r.subject.clone();
        let track = div()
            .relative()
            .flex_1()
            .h_full()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(relative(0.5))
                    .h(px(1.))
                    .bg(p.hairline),
            )
            .children(others.iter().map(|o| {
                div()
                    .absolute()
                    .top(px(14.))
                    .left(at(*o))
                    .ml(px(-1.))
                    .w(px(2.))
                    .h(px(10.))
                    .rounded(px(1.))
                    .bg(p.chart_grey_3)
            }))
            .children(r.then.filter(|_| delta.is_some_and(|d| d != 0)).map(|t| {
                let (a, b) = (pct(t).min(c), pct(t).max(c));
                let col = if c > pct(t) {
                    p.focus_deep
                } else {
                    p.focus_distracted
                };
                div()
                    .absolute()
                    .top(px(17.5))
                    .left(at(a))
                    .w(at(b - a))
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(alpha(col, 0.75))
            }))
            .children(r.then.map(|t| {
                div()
                    .absolute()
                    .top(px(15.))
                    .left(at(pct(t)))
                    .ml(px(-4.))
                    .size(px(8.))
                    .rounded_full()
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.chart_grey_4)
            }))
            .child(
                div()
                    .absolute()
                    .top(px(13.))
                    .left(at(c))
                    .ml(px(-6.))
                    .size(px(12.))
                    .rounded_full()
                    .bg(if r.then.is_none() {
                        p.accent
                    } else {
                        p.text_strong
                    }),
            );
        div()
            .id(nid("insights-map-row", i as u64))
            .role(Role::Button)
            .flex()
            .items_center()
            .h(px(38.))
            .rounded(radius::SM)
            .cursor_pointer()
            .hover(|s| s.bg(p.wash))
            .when(r.faded, |d| d.opacity(0.55))
            .child(
                div()
                    .w(px(NAME_W))
                    .flex_none()
                    .pl(space::S3)
                    .flex()
                    .items_end()
                    .gap(px(6.))
                    .child(
                        div()
                            .truncate()
                            .text_size(text::MD)
                            .font_weight(theme::W_SEMIBOLD)
                            .text_color(p.text_strong)
                            .child(r.subject.clone()),
                    )
                    .child(
                        div()
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_faint)
                            .child(r.conclusions.len().to_string()),
                    ),
            )
            .child(track)
            .child(
                div()
                    .w(px(VAL_W))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(space::S3)
                    .pr(space::S3)
                    .font_family(FONT_MONO)
                    .child(
                        div()
                            .text_size(text::MD)
                            .text_color(p.text_strong)
                            .child(format!("{c}%")),
                    )
                    .child(
                        div()
                            .w(px(28.))
                            .text_size(text::XS)
                            .text_color(dcol)
                            .child(dtext),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.open_subject(subject.clone(), cx)))
    });
    panel_el = panel_el.child(
        div()
            .relative()
            .mt(space::S4)
            .child(band_layer)
            .child(axis)
            .children(rows),
    );
    if listed.len() > MAP_PAGE {
        panel_el = panel_el.child(
            label(
                format!(
                    "Top {MAP_PAGE} by conviction · all {} are in the list below",
                    listed.len()
                ),
                p,
            )
            .mt(space::S3),
        );
    }
    panel_el.into_any_element()
}

fn list(
    this: &Insights,
    groups: Vec<tiers::Group>,
    total: usize,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> Div {
    let off = clock::now(cx).offset();
    let mut col = div().flex().flex_col();
    for g in groups {
        let folded = g.faded && !this.show_fading;
        let (from, to, _, _) = page_slice(g.rows.len(), 0, TIER_PAGE);
        if !g.title.is_empty() {
            let right = if folded {
                (!g.rows.is_empty()).then(|| {
                    div()
                        .id("insights-show-fading")
                        .role(Role::Button)
                        .cursor_pointer()
                        .text_size(text::SM)
                        .text_color(p.text_muted)
                        .child(format!("Show {} fading", g.rows.len()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_fading = true;
                            cx.notify();
                        }))
                        .into_any_element()
                })
            } else {
                Some(
                    super::parts::mono_label(page_label(from, to, g.rows.len()), p)
                        .into_any_element(),
                )
            };
            col = col.child(
                super::parts::group_head(&format!("{} · {}", g.title, g.rows.len()), right, p)
                    .child(label(g.note, p).text_size(text::SM)),
            );
        }
        if folded {
            continue;
        }
        if g.rows.is_empty() {
            col = col.child(
                div()
                    .py(space::S2)
                    .text_size(text::BASE)
                    .text_color(p.text_faint)
                    .child("None right now."),
            );
        }
        for (k, r) in g.rows[from..to].iter().enumerate() {
            let subject = r.subject.clone();
            let spark = r
                .spark
                .iter()
                .enumerate()
                .map(|(i, (faded, pts))| {
                    (
                        if i == 0 {
                            p.text_strong
                        } else {
                            p.chart_grey_2
                        },
                        *faded,
                        pts.clone(),
                    )
                })
                .collect();
            col = col.child(
                div()
                    .id(nid(&format!("insights-subj-{}", g.id), k as u64))
                    .role(Role::Button)
                    .flex()
                    .items_center()
                    .gap(space::S4)
                    .py(px(10.))
                    .px(space::S2)
                    .border_b_1()
                    .border_color(p.hairline)
                    .cursor_pointer()
                    .hover(|s| s.bg(p.wash))
                    .child(
                        div()
                            .w(px(180.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(text::MD)
                                    .font_weight(theme::W_SEMIBOLD)
                                    .text_color(p.text_strong)
                                    .child(r.subject.clone()),
                            )
                            .child(
                                div()
                                    .font_family(FONT_MONO)
                                    .text_size(text::XS)
                                    .text_color(p.text_subtle)
                                    .child(format!(
                                        "{} concl. · since {}",
                                        r.conclusions.len(),
                                        super::stats::month_day(r.since_ms, off)
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(text::BASE)
                            .line_height(relative(1.4))
                            .text_color(p.text_muted)
                            .child(r.headline.clone()),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .h(px(32.))
                            .flex_none()
                            .child(sparkline(spark)),
                    )
                    .child(
                        div()
                            .w(px(40.))
                            .text_right()
                            .font_family(FONT_MONO)
                            .text_size(text::MD)
                            .text_color(p.text_strong)
                            .child(format!("{}%", pct(r.top))),
                    )
                    .child(
                        div()
                            .text_color(p.text_faint)
                            .child(icon(IconName::ChevronRight.path(), px(14.))),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_subject(subject.clone(), cx)),
                    ),
            );
        }
    }
    if total < tiers::SPARSE_LIMIT && this.filter.read(cx).value().trim().is_empty() {
        col = col.child(
            div()
                .mt(space::S3)
                .text_size(text::BASE)
                .text_color(p.text_subtle)
                .child("Tiers appear once there are five or more subjects."),
        );
    }
    col
}

fn side(listed: &[&SubjectRow], p: &Palette, cx: &mut Context<Insights>) -> Div {
    let off = clock::now(cx).offset();
    let moving = rank_movers(listed);
    let key = [
        ("strongly held", "Lead conclusion at 68% or more."),
        ("forming", "30–68%. Building support."),
        ("taking shape", "Under 30% — early, or cooling."),
        ("fading", "Every conclusion below the 15% display floor: leaves the dossier, history kept. Never deleted."),
        ("decay", "Halves every 30 days without fresh evidence. Pin stops it."),
    ];
    div()
        .flex()
        .flex_col()
        .gap(space::S6)
        .when(!moving.is_empty(), |d| {
            d.child(
                div()
                    .child(sec(kicker("Moving this fortnight", p), None))
                    .children(moving.iter().map(|r| {
                        let (t, c) = match move_pts(r) {
                            None => (
                                format!("formed {}", super::stats::month_day(r.since_ms, off)),
                                p.accent,
                            ),
                            Some(x) if x > 0 => (format!("+{x} pts"), p.focus_deep),
                            Some(x) => (format!("−{} pts", x.abs()), p.focus_distracted),
                        };
                        div()
                            .flex()
                            .justify_between()
                            .py(px(7.))
                            .border_t_1()
                            .border_color(p.hairline)
                            .text_size(text::BASE)
                            .child(div().text_color(p.text).child(r.subject.clone()))
                            .child(
                                div()
                                    .font_family(FONT_MONO)
                                    .text_size(text::SM)
                                    .text_color(c)
                                    .child(t),
                            )
                    })),
            )
        })
        .child(
            inset(p)
                .p(space::S5)
                .child(sec(kicker("How conviction works", p), None))
                .children(key.map(|(k, v)| {
                    div()
                        .flex()
                        .gap(space::S3)
                        .py(px(5.))
                        .text_size(text::BASE)
                        .child(
                            div()
                                .w(px(96.))
                                .flex_none()
                                .font_family(FONT_MONO)
                                .text_size(text::SM)
                                .text_color(p.text_strong)
                                .child(k),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(p.text_muted)
                                .line_height(relative(1.4))
                                .child(v),
                        )
                })),
        )
}

/// Tier of one conclusion on the subject page.
pub fn tier_of(c: &capture_types::Conclusion) -> Tier {
    conviction_tier(
        c.confidence,
        c.status == capture_types::ConclusionStatus::Faded,
    )
}
