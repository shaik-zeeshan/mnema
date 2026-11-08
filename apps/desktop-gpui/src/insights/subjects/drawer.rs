//! The evidence drawer (`EvidenceDrawer.svelte`): pinned right under the bar
//! at 44% of the pane, it pushes the subject page narrower so the open row stays
//! on screen. Statement, Pin / Dismiss, the spec line, then the evidence and
//! confidence moves newest first, ending at the origin. ↑/↓ walk the rows.

use std::collections::HashMap;

use capture_types::{Activity, EvidenceStance};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Role};

use super::detail::{flat, histories, TIERS};
use super::tier_of;
use super::tiers::pct;
use crate::insights::parts::{h2, label, mono_label, sec};
use crate::insights::stats::{self, humanize_ms};
use crate::insights::Insights;
use crate::theme::{space, text, Palette, FONT_MONO};
use crate::ui::{clock, kicker, Button};

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
    let spec = [
        format!("now {}%", pct(c.confidence)),
        format!("peak {}%", pct(peak)),
        format!("{support} support · {contra} contradict"),
        format!("formed {}", stats::month_day(c.formed_at_ms, off)),
        format!(
            "last support {}",
            stats::month_day(c.last_supported_at_ms, off)
        ),
        format!(
            "decay {}",
            if pinned {
                "pinned, won't fade"
            } else {
                "halves in 30d without support"
            }
        ),
    ];
    // Evidence, newest first, then the origin (`buildTimeline`).
    // shortcut: the "replaced an earlier take" audit row and evidence paging
    // ("1–8 of 9 ‹ ›") are left out of the drawer for now; it scrolls instead.
    let mut events: Vec<(Option<i64>, String, String, bool)> = c
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
            let contra = e.stance == EvidenceStance::Contradict;
            (
                at,
                title,
                if contra {
                    format!(
                        "contradicts this{}",
                        if note.is_empty() {
                            String::new()
                        } else {
                            format!(" · {note}")
                        }
                    )
                } else {
                    note
                },
                contra,
            )
        })
        .collect();
    events.extend(super::tiers::markers(&h).into_iter().map(|(at, from, to)| {
        (
            Some(at),
            format!("Confidence {}% → {}%", pct(from), pct(to)),
            String::new(),
            false,
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
    events.push((Some(c.formed_at_ms), formed, String::new(), false));
    let total = events.len();
    let list =
        div()
            .flex()
            .flex_col()
            .children(events.into_iter().map(|(at, title, note, contra)| {
                div()
                    .flex()
                    .gap(space::S3)
                    .py(px(8.))
                    .border_t_1()
                    .border_color(p.hairline)
                    .child(
                        div()
                            .w(px(52.))
                            .flex_none()
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(at.map_or("—".into(), |t| stats::month_day(t, off))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .text_size(text::BASE)
                            .text_color(if contra { p.focus_distracted } else { p.text })
                            .child(title)
                            .when(!note.is_empty(), |d| d.child(mono_label(note, p))),
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
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(space::S4)
                    .gap_y(space::S1)
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .children(spec.map(|s| div().child(s))),
            )
            .child(
                div()
                    .child(sec(
                        kicker(&format!("Evidence · {total}"), p),
                        Some(label("newest first", p).into_any_element()),
                    ))
                    .child(list),
            )
            .into_any_element(),
    )
}
