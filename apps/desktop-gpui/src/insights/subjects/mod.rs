//! Subjects (`Subjects.svelte`): the dossier by conviction. Filter + the
//! Conviction / Movement switch on the bar; the lede (headline over the tiers,
//! four tier counts); the conviction map with 14-day tails; the tiered list
//! beside "Moving this fortnight" and how conviction works. A subject opens
//! its page (`detail.rs`) with the push-drawer of evidence.

mod detail;
mod drawer;
mod map;
pub mod peek;
pub mod refresh;
pub use refresh::Ui;
use map::map;
pub mod pager;
pub mod tiers;

use crate::ui::Tracking as _;
use crate::ui::input::FieldText as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, relative, AnyElement, Context, Div, Focusable as _, Role, SharedString, Window,
};

pub use drawer::{drawer, push as drawer_push, step as step_conclusion};
pub use detail::{flat as flat_conclusions, reveal as reveal_conclusion, tier_key};

use super::charts::sparkline;
use super::parts::{display, inset, label, lede, lift, prose, read, sec, split, stat_text, PHI};
use super::Insights;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::baseline::Line;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::segmented::Segmented;
use crate::ui::{clock, icon, kicker, nid, Tone};
use tiers::{
    conviction_tier, headline, list_groups, move_pts, pct, rank_movers,
    subject_rows, tier_names, Axis, SubjectRow, Tier, TIER_PAGE,
};

/// A list row's name and its OPENED tag (`.in-subj__name` + `__tag`), inline on one baseline.
const NAME: Line = Line {
    family: crate::theme::FONT_UI,
    size: text::MD,
    lh: PHI,
};
const TAG: Line = Line {
    family: FONT_MONO,
    size: px(10.),
    lh: PHI,
};

pub fn controls(
    this: &Insights,
    p: &Palette,
    window: &Window,
    cx: &mut Context<Insights>,
) -> Vec<AnyElement> {
    if !this
        .subjects
        .as_ref()
        .is_some_and(|s| !s.conclusions.is_empty())
    {
        return Vec::new();
    }
    let focused = this.filter.focus_handle(cx).is_focused(window);
    let search = crate::chat::list::search_states(
        div()
            .flex()
            .items_center()
            .gap(space::S2)
            .w(px(220.))
            .h(size::H_MD)
            .px(space::S3)
            .rounded(radius::MD)
            .border_1()
            .bg(p.surface_subtle),
        focused,
        p,
    )
    .child(icon(IconName::Search.path(), px(14.)))
        .child(
            div().flex_1().min_w_0().text_color(p.text_strong).child(
                Input::new(&this.filter)
                    .id("insights-filter")
                    .aria_label("Filter subjects")
                    .appearance(false)
                    .px_0()
                    .field_text(text::BASE)
                    .field_h(size::H_MD - px(2.)),
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
            vec![crate::ui::Button::new("insights-subjects-retry", "Try again")
                .sm()
                .icon(IconName::RotateCcw.path())
                .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                .into_any_element()],
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
        let held = ctx
            .and_then(|c| c.last_distillation.as_ref())
            .map_or(0, |d| d.guardrail_suppressed + d.below_formation_bar + d.resurface_blocked + d.ungrounded);
        let (title, body) = if backfilling {
            ("No subjects yet.".to_string(), "As Mnema forms views about you, each one appears here with its own confidence over time. It's backfilling your history now — a view needs at least two supporting activities.".to_string())
        } else if dismissed > 0 {
            ("No subjects to show right now.".into(), format!("You've dismissed {dismissed}. Restoring one in Context only lifts the block — it forms again only if recent activity supports it."))
        } else {
            let tail = if acts > 0 {
                let held = if held > 0 {
                    format!("; the last pass held back {held} {}", if held == 1 { "draft" } else { "drafts" })
                } else {
                    String::new()
                };
                format!(
                    " {acts} {} summarized so far{held}.",
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
        let view = (dismissed > 0 && !backfilling).then(|| {
            crate::ui::Button::new("insights-view-dismissed", "View dismissed")
                .sm()
                .on_click(cx.listener(|this, _, _, cx| this.view_dismissed(cx)))
                .into_any_element()
        });
        return center(empty(
            EmptyLayout::Center,
            Tone::Neutral,
            IconName::Inbox.path(),
            title,
            body,
            view.into_iter().collect(),
            p,
        ));
    }
    let q = this.subj_ui.applied.clone();
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
        .child(display(lead, Some(tail), p, window))
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
    if std::mem::take(&mut this.reveal_opened) {
        reveal(this, &groups);
    }
    div()
        .flex()
        .flex_col()
        .child(read(lede_el, stats, this.width, p))
        .child(map(&listed, &q, this.width, p, cx))
        .child(
            split(
                div().child(list(this, groups, rows.len(), p, cx)),
                side(&listed, p, cx),
                this.width,
            )
            .items_start(),
        )
        .into_any_element()
}

fn list(
    this: &Insights,
    groups: Vec<tiers::Group>,
    total: usize,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> Div {
    let off = clock::now(cx).offset();
    let mut col = this.list_anchor.track(div().flex().flex_col());
    for g in groups {
        let folded = g.faded && !this.show_fading;
        let key = section_key(this.axis, g.id);
        let (from, to, _, _) = pager::slice(this, &key, g.rows.len(), TIER_PAGE);
        if !g.title.is_empty() {
            // `.in-group__link`: Show / Hide share the heading's mono caps, 22 px, hover fill.
            let link = |id: &'static str, s: String, show: bool| {
                div()
                    .id(id)
                    .role(Role::Button)
                    .h(px(22.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(radius::XS)
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text_subtle)
                    .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
                    .child(s.to_uppercase())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_fading = show;
                        cx.notify();
                    }))
            };
            let right = if folded {
                (!g.rows.is_empty()).then(|| {
                    link("insights-show-fading", format!("Show {} fading", g.rows.len()), true)
                        .into_any_element()
                })
            } else {
                let hide = g
                    .faded
                    .then(|| link("insights-hide-fading", "Hide".into(), false));
                Some(
                    div()
                        .flex()
                        .items_center()
                        .children(pager::pager(this, &key, g.title, g.rows.len(), TIER_PAGE, true, cx))
                        .children(hide)
                        .into_any_element(),
                )
            };
            col = col.child(
                super::parts::group_head(&format!("{} · {}", g.title, g.rows.len()), right, p)
                    .child(label(g.note, p).text_size(text::SM).tracking(0.)),
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
            let current = this.last_opened.as_deref() == Some(r.subject.as_str());
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
                // `.in-subj`: 170 / 1fr / 100 / 44 / 14, the fill 12 px past the column.
                div()
                    .id(nid(&format!("insights-subj-{}", g.id), k as u64))
                    .role(Role::Button)
                    .group("insights-subj")
                    .flex()
                    .items_center()
                    .gap(space::S4)
                    .py(px(12.))
                    .px(space::S3)
                    .mx(-space::S3)
                    // `.in-subj:last-of-type { border-bottom: 0 }`.
                    .when(from + k + 1 < to, |d| d.border_b_1())
                    .border_color(p.hairline)
                    .rounded(radius::SM)
                    .cursor_pointer()
                    .hover(|s| s.bg(p.surface_hover))
                    .when(current, |d| d.bg(p.wash))
                    .when(g.faded, |d| d.opacity(0.62))
                    .child(
                        div()
                            .w(px(170.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .flex()
                                    .items_end()
                                    .min_w_0()
                                    .text_size(text::MD)
                                    .font_weight(theme::W_SEMIBOLD)
                                    .text_color(p.text_strong)
                                    .child(crate::ui::with_tip(
                                        div()
                                            .id(nid(&format!("insights-subj-name-{}", g.id), k as u64))
                                            .truncate()
                                            .when(current, |d| d.pb(lift(NAME, &[NAME, TAG])))
                                            .child(r.subject.clone()),
                                        r.subject.clone(),
                                    ))
                                    .when(current, |d| {
                                        d.child(
                                            div()
                                                .ml(px(6.))
                                                .pb(lift(TAG, &[NAME, TAG]))
                                                .font_family(FONT_MONO)
                                                .text_size(px(10.))
                                                .tracking(0.06)
                                                .text_color(p.accent)
                                                .child("OPENED"),
                                        )
                                    }),
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
                            .w(px(100.))
                            .h(px(30.))
                            .flex_none()
                            .child(sparkline(spark)),
                    )
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .text_right()
                            .font_family(FONT_MONO)
                            .text_size(text::MD)
                            .text_color(p.text_strong)
                            .child(format!("{}%", pct(r.top))),
                    )
                    .child(
                        // The arrow nudges 3 px right and darkens on row hover.
                        div()
                            .relative()
                            .flex_none()
                            .text_color(p.text_faint)
                            .group_hover("insights-subj", |s| {
                                s.left(px(3.)).text_color(p.text_strong)
                            })
                            .child(icon(IconName::ChevronRight.path(), px(14.))),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_subject(subject.clone(), cx)),
                    ),
            );
        }
    }
    if total < tiers::SPARSE_LIMIT && this.subj_ui.applied.is_empty() {
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
                    .children(moving.iter().enumerate().map(|(i, r)| {
                        let (t, c) = match move_pts(r) {
                            None => (
                                format!("formed {}", super::stats::month_day(r.since_ms, off)),
                                p.accent,
                            ),
                            Some(x) if x > 0 => (format!("+{x} pts"), p.focus_deep),
                            Some(x) => (format!("−{} pts", x.abs()), p.focus_distracted),
                        };
                        let subject = r.subject.clone();
                        div()
                            .id(SharedString::from(format!("insights-move-{}", r.subject)))
                            .role(Role::Button)
                            .flex()
                            .justify_between()
                            .gap(space::S3)
                            .py(px(10.))
                            .when(i > 0, |d| d.border_t_1())
                            .border_color(p.hairline)
                            .text_size(text::BASE)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.open_subject(subject.clone(), cx)))
                            // One line, cut with "…" (the subject page has the full name).
                            .child(crate::ui::with_tip(
                                div()
                                    .id(SharedString::from(format!("insights-move-name-{}", r.subject)))
                                    .min_w_0()
                                    .truncate()
                                    .text_color(p.text)
                                    .child(r.subject.clone()),
                                r.subject.clone(),
                            ))
                            .child(
                                div()
                                    .flex_none()
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
                .p(space::S4)
                .child(sec(kicker("How conviction works", p), None))
                .children(key.into_iter().enumerate().map(|(i, (k, v))| {
                    div()
                        .flex()
                        .gap(space::S3)
                        .py(px(10.))
                        .when(i > 0, |d| d.border_t_1())
                        .border_color(p.hairline)
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
                                .min_w_0()
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

/// A list section's pager key: the tiers carry the axis (each axis keeps its
/// own pages), the matches are `search`.
pub fn section_key(axis: Axis, id: &str) -> String {
    match id {
        "search" | "all" => id.to_string(),
        _ => format!("{axis:?}:{id}"),
    }
}

/// Back from a subject's page: turn just its tier to its row (unfolding
/// Fading if that's where it is).
fn reveal(this: &mut Insights, groups: &[tiers::Group]) {
    let Some(cur) = this.last_opened.clone() else {
        return;
    };
    for g in groups {
        let Some(i) = g.rows.iter().position(|r| r.subject == cur) else {
            continue;
        };
        if g.faded {
            this.show_fading = true;
        }
        let key = section_key(this.axis, g.id);
        let (from, to, _, _) = pager::slice(this, &key, g.rows.len(), TIER_PAGE);
        if i < from || i >= to {
            let size = pager::state(this, &key, TIER_PAGE).1;
            this.pages.insert(key, (pager::holding(i, size), size));
        }
    }
}

/// ←/→ on the index: turn the tier holding the marked row, else the first
/// tier that pages (`SubjectsList.svelte` `onKey`).
pub fn turn_list(this: &mut Insights, d: isize, cx: &mut Context<Insights>) -> bool {
    let Some(data) = this.subjects.clone() else {
        return false;
    };
    let rows = subject_rows(&data.conclusions, &data.histories, clock::now_ms(cx));
    let q = this.subj_ui.applied.clone();
    let matches = (!q.is_empty()).then(|| tiers::filter(&rows, &q));
    let groups = list_groups(&rows, this.axis, matches);
    let pick = |g: &&tiers::Group| !(g.faded && !this.show_fading);
    let held = groups.iter().filter(pick).find(|g| {
        g.rows
            .iter()
            .any(|r| Some(&r.subject) == this.last_opened.as_ref())
    });
    let paged = groups
        .iter()
        .filter(pick)
        .find(|g| pager::slice(this, &section_key(this.axis, g.id), g.rows.len(), TIER_PAGE).3 > 1);
    let Some(g) = held.or(paged) else {
        return false;
    };
    let key = section_key(this.axis, g.id);
    let (_, _, page, pages) = pager::slice(this, &key, g.rows.len(), TIER_PAGE);
    let next = page as isize + d;
    if next < 0 || next >= pages as isize {
        return false;
    }
    this.turn_page(&key, next as usize, TIER_PAGE, cx);
    true
}

/// ←/→ on a subject's page: turn the first tier that pages.
pub fn turn_subject(this: &mut Insights, d: isize, cx: &mut Context<Insights>) -> bool {
    let live = detail::flat(this);
    for (t, ..) in detail::TIERS {
        let n = live.iter().filter(|c| tier_of(c) == t).count();
        let key = tier_key(t);
        let (_, _, page, pages) = pager::slice(this, &key, n, TIER_PAGE);
        if pages > 1 {
            let next = page as isize + d;
            if next >= 0 && next < pages as isize {
                this.turn_page(&key, next as usize, TIER_PAGE, cx);
            }
            return true;
        }
    }
    false
}

impl Insights {
    /// Subjects' "View dismissed": Context, with the Dismissed archive in view.
    pub fn view_dismissed(&mut self, cx: &mut Context<Self>) {
        self.set_tab(super::Tab::Context, cx);
        self.ctx_ui.show_dismissed = true;
    }

    /// "Go to the list": bring the list's top into view, clear of the bar.
    pub fn go_to_list(&mut self, cx: &mut Context<Self>) {
        let list = self.list_anchor.bounds(cx);
        let view = self.scroll.bounds();
        let y = list.top() - view.top() - self.scroll.offset().y - space::S7;
        self.scroll.set_offset(gpui_kit::point(px(0.), -y.max(px(0.))));
        cx.notify();
    }
}
