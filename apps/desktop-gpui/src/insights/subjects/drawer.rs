//! The evidence drawer (`EvidenceDrawer.svelte`): pinned right under the bar
//! at 44% of the pane, it pushes the subject page narrower so the open row stays
//! on screen. Statement, Pin / Dismiss, the spec line, then the evidence and
//! confidence moves newest first on a dot rail, ending at the origin, paged
//! `EV_PAGE` at a time. ↑/↓ walk the rows.

use std::collections::HashMap;

use capture_types::{Activity, EvidenceStance};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Role};

use super::detail::{flat, histories, TIERS};
use super::tier_of;
use super::tiers::pct;
use crate::insights::parts::{h2, label, sec};
use crate::insights::stats::{self, humanize_ms};
use crate::insights::Insights;
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::{clock, icon, kicker, Button};

/// ↑/↓ walk the conclusions and the drawer follows.
pub fn step(this: &mut Insights, d: i64, cx: &mut Context<Insights>) -> bool {
    let flat = flat(this);
    if flat.is_empty() || this.subject.is_none() {
        return false;
    }
    let cur = this
        .open_conclusion
        .and_then(|id| flat.iter().position(|c| c.id == id));
    let next = match cur {
        None if d > 0 => Some(0),
        None => Some(flat.len() - 1),
        Some(i) => (i as i64 + d)
            .try_into()
            .ok()
            .filter(|n: &usize| *n < flat.len()),
    };
    if let Some(n) = next {
        this.open_conclusion = Some(flat[n].id);
        cx.notify();
    }
    true
}

/// The push-drawer: pinned right under the bar, 44% of the pane.
pub fn drawer(this: &Insights, p: &Palette, cx: &mut Context<Insights>) -> Option<AnyElement> {
    let id = this
        .open_conclusion
        .filter(|_| this.subject.is_some() && this.tab == crate::insights::Tab::Subjects)?;
    let flat = flat(this);
    let i = flat.iter().position(|c| c.id == id)?;
    let c = flat[i].clone();
    let off = clock::now(cx).offset();
    let hist = histories(this);
    let h = hist.get(&id).cloned().unwrap_or_default();
    let peak = h.iter().map(|s| s.confidence).fold(c.confidence, f64::max);
    let support = c
        .evidence
        .iter()
        .filter(|e| e.stance == EvidenceStance::Support)
        .count();
    let contra = c.evidence.len() - support;
    let tier = TIERS
        .iter()
        .find(|t| t.0 == tier_of(&c))
        .map_or("", |t| t.1);
    let acts: HashMap<i64, Activity> = this
        .detail
        .as_ref()
        .map(|d| d.activities.clone())
        .unwrap_or_default();
    let icon_btn = |id_: &'static str, path: gpui_kit::SharedString, aria: &'static str| {
        Button::icon_only(id_, path, aria).ghost().sm()
    };
    let n = flat.len();
    let bar = div()
        .flex()
        .items_center()
        .gap(space::S1)
        .child(kicker(
            &format!("Conclusion · {} of {n} · {tier}", i + 1),
            p,
        ))
        .child(div().flex_1())
        .child(
            icon_btn(
                "insights-drawer-prev",
                IconName::ChevronLeft.path(),
                "Previous conclusion",
            )
            .disabled(i == 0)
            .on_click(cx.listener(|this, _, _, cx| {
                step(this, -1, cx);
            })),
        )
        .child(
            icon_btn(
                "insights-drawer-next",
                IconName::ChevronRight.path(),
                "Next conclusion",
            )
            .disabled(i + 1 >= n)
            .on_click(cx.listener(|this, _, _, cx| {
                step(this, 1, cx);
            })),
        )
        .child(
            icon_btn("insights-drawer-close", IconName::X.path(), "Close").on_click(cx.listener(
                |this, _, _, cx| {
                    this.open_conclusion = None;
                    cx.notify();
                },
            )),
        );
    let pinned = c.pinned;
    let acts_row = div()
        .flex()
        .gap(space::S2)
        .child(
            Button::new("insights-pin", if pinned { "Pinned" } else { "Pin" })
                .sm()
                .icon(IconName::Pin.path())
                .pressed(pinned)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_pin(id, pinned, cx))),
        )
        .child(
            Button::new("insights-dismiss", "Dismiss")
                .ghost()
                .sm()
                .on_click(cx.listener(move |this, _, _, cx| this.dismiss(id, cx))),
        );
    // `.sd-spec`: mono, subtle words with the values bold and strong.
    let b =
        |s: String, c: gpui_kit::Hsla| div().font_weight(theme::W_SEMIBOLD).text_color(c).child(s);
    let strong = p.text_strong;
    let item = |parts: Vec<gpui_kit::AnyElement>| div().flex().gap(px(4.)).children(parts);
    let w = |s: &'static str| div().child(s).into_any_element();
    let decay = if pinned {
        "pinned, won't fade"
    } else {
        "halves in 30d without support"
    };
    let newest = c
        .evidence
        .iter()
        .filter(|e| e.stance == EvidenceStance::Support)
        .filter_map(|e| acts.get(&e.activity_id))
        .filter(|a| !a.evidence.is_empty())
        .max_by_key(|a| a.started_at_ms)
        .map(|a| a.evidence[0].clone());
    let spec = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(14.))
        .gap_y(px(4.))
        .py(px(8.))
        .border_t_1()
        .border_b_1()
        .border_color(p.hairline)
        .font_family(FONT_MONO)
        .font_weight(theme::W_MEDIUM)
        .text_size(text::XS)
        .line_height(relative(1.5))
        .text_color(p.text_subtle)
        .child(item(vec![
            w("now"),
            b(format!("{}%", pct(c.confidence)), strong).into_any_element(),
        ]))
        .child(item(vec![
            w("peak"),
            b(format!("{}%", pct(peak)), strong).into_any_element(),
        ]))
        .child(item(vec![
            b(support.to_string(), strong).into_any_element(),
            w("support ·"),
            b(
                contra.to_string(),
                if contra > 0 {
                    p.focus_distracted
                } else {
                    strong
                },
            )
            .into_any_element(),
            w("contradict"),
        ]))
        .child(item(vec![
            w("formed"),
            b(stats::month_day(c.formed_at_ms, off), strong).into_any_element(),
        ]))
        .child(item(vec![
            w("last support"),
            b(stats::month_day(c.last_supported_at_ms, off), strong).into_any_element(),
        ]))
        .child(item(vec![
            w("decay"),
            b(decay.into(), strong).into_any_element(),
        ]))
        .children(newest.map(|r| {
            let muted = (p.text_muted, p.text_strong);
            div()
                .id("insights-drawer-open-newest")
                .role(Role::Link)
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .text_color(muted.0)
                .hover(move |s| s.text_color(muted.1))
                .child("Open newest in Timeline")
                .child(icon(IconName::ArrowRight.path(), px(12.)))
                .on_click(cx.listener(move |_, _, _, cx| {
                    // shortcut: an audio ref opens the Timeline without seeking to its segment.
                    cx.emit(if r.subject_type == "frame" {
                        crate::insights::InsightsEvent::OpenFrame(r.subject_id)
                    } else {
                        crate::insights::InsightsEvent::OpenTimeline
                    })
                }))
        }));
    // Evidence, newest first, then the origin (`buildTimeline`).
    // shortcut: the "replaced an earlier take" audit row is left out, and the
    // page is a fixed EV_PAGE rows (Tauri fits rows to the drawer's height).
    let mut events: Vec<(Option<i64>, String, String, Dot)> = c
        .evidence
        .iter()
        .map(|e| {
            let a = acts.get(&e.activity_id);
            let at = a.map(|a| a.started_at_ms).or(e.activity_started_at_ms);
            let title = a
                .map(|a| a.title.clone())
                .or(e.activity_title.clone())
                .unwrap_or_else(|| format!("Activity #{}", e.activity_id));
            let note = a
                .map(|a| {
                    let frames = a
                        .evidence
                        .iter()
                        .filter(|r| r.subject_type == "frame")
                        .count();
                    let cited = match frames {
                        0 => String::new(),
                        1 => " · cited 1 frame".into(),
                        n => format!(" · cited {n} frames"),
                    };
                    format!(
                        "{} · {}{cited}",
                        stats::clock(a.started_at_ms, off),
                        humanize_ms(a.ended_at_ms - a.started_at_ms)
                    )
                })
                .unwrap_or_default();
            if e.stance == EvidenceStance::Contradict {
                let note = if note.is_empty() {
                    "contradicts this".to_string()
                } else {
                    format!("contradicts this · {note}")
                };
                (at, title, note, Dot::Contra)
            } else {
                (at, title, note, Dot::Evidence)
            }
        })
        .collect();
    events.extend(super::tiers::markers(&h).into_iter().map(|(at, from, to)| {
        (
            Some(at),
            format!("Confidence {}% → {}%", pct(from), pct(to)),
            String::new(),
            Dot::Marker,
        )
    }));
    events.sort_by(|a, b| b.0.unwrap_or(i64::MIN).cmp(&a.0.unwrap_or(i64::MIN)));
    let formed = match c.formed_evidence_count {
        Some(k) => format!(
            "Formed from {k} supporting {} · {}%",
            if k == 1 { "activity" } else { "activities" },
            pct(h.first().map_or(c.confidence, |s| s.confidence))
        ),
        None => format!(
            "Formed at {}%",
            pct(h.first().map_or(c.confidence, |s| s.confidence))
        ),
    };
    events.push((Some(c.formed_at_ms), formed, String::new(), Dot::Origin));
    let total = events.len();
    let page = if this.evidence_page.0 == id {
        this.evidence_page.1
    } else {
        0
    };
    let (from, to, page, pages) = super::tiers::page_slice(total, page, EV_PAGE);
    let turn = |k: isize, enabled: bool, id_: &'static str, path, aria: &'static str| {
        icon_btn(id_, path, aria)
            .disabled(!enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.evidence_page = (id, (page as isize + k).max(0) as usize);
                cx.notify();
            }))
    };
    let pager = div()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(
            div()
                .mr(px(4.))
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(format!("{}\u{2013}{to} OF {total}", from + 1)),
        )
        .child(turn(
            -1,
            page > 0,
            "insights-ev-prev",
            IconName::ChevronLeft.path(),
            "Previous evidence page",
        ))
        .child(turn(
            1,
            page + 1 < pages,
            "insights-ev-next",
            IconName::ChevronRight.path(),
            "Next evidence page",
        ));
    let rows = events.into_iter().skip(from).take(to - from);
    // `.in-tl`: a hairline rail through a dot per row.
    let list = div()
        .relative()
        .mt(space::S2)
        .child(
            div()
                .absolute()
                .left(px(5.))
                .top(px(6.))
                .bottom(px(6.))
                .w(px(1.))
                .bg(p.border),
        )
        .children(rows.map(|(at, title, note, dot)| {
            let marker = match dot {
                Dot::Marker => div()
                    .size(px(7.))
                    .mt(px(5.))
                    .ml(px(2.))
                    .rounded_full()
                    .bg(p.text_strong),
                Dot::Origin => div()
                    .size(px(11.))
                    .mt(px(3.))
                    .rounded_full()
                    .bg(p.accent)
                    .shadow(vec![crate::ui::spread(p.accent_glow, px(3.))]),
                _ => div()
                    .size(px(11.))
                    .mt(px(3.))
                    .rounded_full()
                    .bg(p.bg)
                    .border(px(1.5))
                    .border_color(if dot == Dot::Contra {
                        p.focus_distracted
                    } else {
                        p.border_strong
                    }),
            };
            div()
                .flex()
                .py(px(9.))
                .child(div().w(px(22.)).flex_none().child(marker))
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_size(text::XS)
                        .line_height(relative(1.9))
                        .text_color(p.text_subtle)
                        .child(at.map_or("—".into(), |t| stats::month_day(t, off))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .text_size(text::BASE)
                        .text_color(p.text)
                        .when(dot == Dot::Marker, |d| {
                            d.font_family(FONT_MONO)
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::SM)
                                .text_color(p.text_strong)
                        })
                        .child(title)
                        .when(!note.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_size(text::SM)
                                    .line_height(relative(1.4))
                                    .text_color(if dot == Dot::Contra {
                                        p.focus_distracted
                                    } else {
                                        p.text_subtle
                                    })
                                    .child(note),
                            )
                        }),
                )
        }));
    Some(
        div()
            .id("insights-drawer")
            .role(Role::Complementary)
            .w(relative(0.44))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(p.border)
            .bg(p.surface)
            .p(space::S5)
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(bar)
            .child(h2(c.statement.clone(), p).line_height(relative(1.3)))
            .child(acts_row)
            .child(spec)
            .child(
                div()
                    .child(sec(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(kicker(&format!("Evidence · {total}"), p))
                            .child(label("newest first", p).text_color(p.text_faint)),
                        Some(pager.into_any_element()),
                    ))
                    .child(list),
            )
            .into_any_element(),
    )
}

/// Evidence rows per drawer page.
const EV_PAGE: usize = 8;

/// One evidence row's rail dot (`.in-tl li` variants).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dot {
    Evidence,
    Contra,
    Marker,
    Origin,
}
