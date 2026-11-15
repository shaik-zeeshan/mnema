//! The conviction map (`ConvictionMap.svelte`): the bands, the axis, and one
//! row per subject (the top 12) with its 14-day tail, its other conclusions'
//! ticks and its fortnight delta; "Go to the list" past the top 12, and the
//! no-match state with Clear filter.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Role};

use super::tiers::{pct, SubjectRow, DISPLAY_FLOOR, INITIAL_BASE, MAP_PAGE, STRONGLY_HELD};
use crate::insights::charts::hatch;
use crate::insights::parts::{label, lift, panel, sec, PHI};
use crate::insights::Insights;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::Tracking as _;
use crate::ui::{alpha, kicker, nid, ring_box, Tone};

/// A row's name and its conclusion count (`.in-map__name` + `small`), inline on one baseline.
const NAME: Line = Line {
    family: FONT_UI,
    size: text::MD,
    lh: PHI,
};
const COUNT: Line = Line {
    family: FONT_MONO,
    size: text::XS,
    lh: PHI,
};

/// `ConvictionMap.svelte`: bands, axis, and one row per subject (top 12).
pub(super) fn map(
    listed: &[&SubjectRow],
    q: &str,
    width: gpui_kit::Pixels,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> AnyElement {
    // `--map-name`: 168 px, 128 px on a window ≤1000 px wide.
    let name_w = if width <= crate::theme::breakpoint::READ_STACK {
        128.
    } else {
        168.
    };
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
                .child(div().size(px(9.)).rounded_full().bg(p.text_strong))
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
                        .border(px(1.5))
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
        let clear = crate::ui::Button::new("insights-filter-clear", "Clear filter")
            .sm()
            .on_click(cx.listener(|this, _, window, cx| this.clear_filter(window, cx)));
        return panel_el.child(empty(EmptyLayout::Panel, Tone::Neutral, IconName::Search.path(), format!("No subjects match “{q}”."), "Filter looks at subject names and the wording of each belief. Try a shorter term, or clear it.", vec![clear.into_any_element()], p)).into_any_element();
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
        .left(px(name_w))
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
                // `.fade`: 1 px fg-3% stripes every 7 px.
                .when(fade, |d| {
                    d.child(
                        hatch(alpha(p.fg, 0.03), gpui_kit::transparent_black(), 1., 7.)
                            .absolute()
                            .inset_0(),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(8.))
                        // `max-width: calc(100% - 12px)`: a narrow band clips its label.
                        .right(px(4.))
                        .truncate()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .tracking(0.08)
                        .text_color(p.text_faint)
                        .child(l.to_uppercase()),
                )
        }));
    let axis = div()
        .flex()
        .h(px(44.))
        .child(div().w(px(name_w)).flex_none())
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
        let n = r.conclusions.len();
        let tip = format!(
            "{} · {c}% — {} · {n} conclusion{} · {}",
            r.subject,
            r.headline,
            if n == 1 { "" } else { "s" },
            match r.then {
                None => "formed this fortnight".to_string(),
                Some(t) => format!("was {}% 14 days ago", pct(t)),
            }
        );
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
                    .border(px(1.5))
                    .border_color(p.chart_grey_4)
            }))
            .child(
                // `.in-map__now`: a 3 px surface ring; a new one adds a 2 px glow past it.
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
                    })
                    .child(ring_box(p.surface, px(3.), px(6.)))
                    .when(r.then.is_none(), |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(-5.))
                                .left(px(-5.))
                                .right(px(-5.))
                                .bottom(px(-5.))
                                .rounded(px(11.))
                                .border_2()
                                .border_color(p.accent_glow),
                        )
                    }),
            );
        let row = div()
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
                    .w(px(name_w))
                    .flex_none()
                    .pl(space::S3)
                    .flex()
                    .items_end()
                    .gap(px(6.))
                    .child(
                        div()
                            .truncate()
                            .pb(lift(NAME, &[NAME, COUNT]))
                            .text_size(text::MD)
                            .font_weight(theme::W_SEMIBOLD)
                            .text_color(p.text_strong)
                            .child(r.subject.clone()),
                    )
                    .child(
                        div()
                            .pb(lift(COUNT, &[NAME, COUNT]))
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_faint)
                            .child(r.conclusions.len().to_string()),
                    ),
            )
            .child(track)
            .child(
                baseline::row(vec![
                    (
                        Line::new(FONT_MONO, text::MD, PHI),
                        div()
                            .text_color(p.text_strong)
                            .child(format!("{c}%"))
                            .into_any_element(),
                    ),
                    (
                        Line::new(FONT_MONO, text::XS, PHI),
                        div()
                            .w(px(28.))
                            .text_color(dcol)
                            .child(dtext)
                            .into_any_element(),
                    ),
                ])
                .w(px(VAL_W))
                .flex_none()
                .justify_end()
                .gap(space::S3)
                .pr(space::S3),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.open_subject(subject.clone(), cx)));
        crate::ui::with_tip(row, tip)
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
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .mt(space::S3)
                .child(label(
                    format!(
                        "Top {MAP_PAGE} by conviction · all {} are in the list below",
                        listed.len()
                    ),
                    p,
                ))
                .child(
                    crate::ui::Button::new("insights-go-list", "Go to the list")
                        .ghost()
                        .sm()
                        .icon(IconName::List.path())
                        .on_click(cx.listener(|this, _, _, cx| this.go_to_list(cx))),
                ),
        );
    }
    panel_el.into_any_element()
}
