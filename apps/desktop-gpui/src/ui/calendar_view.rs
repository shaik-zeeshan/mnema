//! The `.mx-cal` popover body (kit.css › Date input): a month head with ‹ ›,
//! one of three bodies, and the legend foot. Days = the 7 × N day grid (a
//! solid selected day, or range mode's in-span wash with rounded caps);
//! Weeks (`.mx-cal--weeks`) = one button per Mon–Sun row with its ISO week
//! number; Months (`.mx-cal__months`) = the year as 3 × 4 month cells.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, App, Div, FocusHandle, Hsla, KeyDownEvent, MouseButton, SharedString, Stateful, Window,
};
use time::{Date, Duration};

use super::calendar::{month_cells, month_of, span_label, WEEKDAYS};
use super::{alpha, focus_handle, focus_visible, mix, nid, Button, Force};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::Tracking as _;

pub type OnPickDay = Rc<dyn Fn(Date, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Days,
    Weeks,
    Months,
}

pub enum Foot {
    /// "Today" / "This week": `on_pick(max)`; disabled when already there.
    Reset { label: SharedString, disabled: bool },
    /// Range mode's instruction ("drag across days" / "pick an end day").
    Hint(&'static str),
    /// The Timeline jumper (`.jumper-cal`): legend only, 10/10/8 padding.
    Legend,
}

/// bits-ui's roving grid (the Timeline jumper): the day grid is one Tab stop;
/// its keys move `cursor` (drawn as the focus ring) and pick it.
pub struct GridKeys {
    pub focus: FocusHandle,
    pub cursor: Option<Date>,
    pub on_key: Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>,
}

/// Range mode: the pending span's marks, and press / hover instead of click.
pub struct RangeHooks {
    pub marks: Option<(Date, Date)>,
    pub on_press: OnPickDay,
    pub on_hover: OnPickDay,
}

/// Everything one `.mx-cal` render needs.
pub struct CalSpec {
    pub id: SharedString,
    /// Any day of the month on screen (Months: of the year).
    pub view: Date,
    pub today: Date,
    pub min: Date,
    pub max: Date,
    pub body: Body,
    /// The solid day · the selected week's Monday · the selected month's 1st.
    pub sel: Option<Date>,
    pub range: Option<RangeHooks>,
    /// The Timeline's committed day: an 11% accent tint unless it is `sel`.
    pub here: Option<Date>,
    /// Days only: one Tab stop for the whole grid instead of one per day.
    pub grid: Option<GridKeys>,
    pub density: Rc<dyn Fn(Date) -> u8>,
    pub foot: Foot,
    /// A day · a week's Monday · a month's 1st.
    pub on_pick: OnPickDay,
    /// `k` months (the Months body passes whole years).
    pub on_nav: Rc<dyn Fn(i32, &mut Window, &mut App)>,
}

#[derive(IntoElement)]
pub struct Calendar(CalSpec);

pub fn calendar(c: CalSpec) -> Calendar {
    Calendar(c)
}

/// Density dot size/colour for 0..=3 captured (0 = no dot).
fn dot_of(h: u8, p: &Palette) -> Option<(f32, Hsla)> {
    match h {
        0 => None,
        1 => Some((3., p.chart_grey_2)),
        2 => Some((4., p.chart_grey_3)),
        _ => Some((5., p.chart_grey_5)),
    }
}

/// The dot under a cell's number (`i { bottom: 5px }`); `ink` overrides its colour.
fn dot(h: u8, ink: Option<Hsla>, p: &Palette) -> Option<Div> {
    dot_of(h, p).map(|(s, c)| {
        div()
            .absolute()
            .bottom(px(5.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(div().size(px(s)).rounded_full().bg(ink.unwrap_or(c)))
    })
}

/// The number, underlined in accent on today (`span { box-shadow: 0 1.5px }`).
fn number(label: String, underline: bool, p: &Palette) -> Div {
    div()
        .when(underline, |el| el.border_b(px(1.5)).border_color(p.accent))
        .child(label)
}

/// `outline: 2px solid accent; outline-offset: -2px` (week rows, cells).
fn inset_ring(p: &Palette) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .rounded(radius::SM)
        .border_2()
        .border_color(p.accent)
}

/// The `.mx-cal__day` box every body shares.
fn cell(id: gpui_kit::ElementId, w: gpui_kit::Pixels, h: gpui_kit::Pixels) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(w)
        .h(h)
        .text_size(text::BASE)
        .line_height(gpui_kit::relative(1.))
}

const WEEK_GROUP: &str = "mx-cal-week";
const WN_W: f32 = 24.;
const MONTHS_W: f32 = 238.;
const MONTHS_GAP: f32 = 4.;

impl RenderOnce for Calendar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = self.0;
        let p = theme::tokens(cx);
        let (vfirst, _) = month_of(c.view, 0);
        let (min, max) = (c.min, c.max);
        let density = c.density.clone();
        let heat = move |d: Date| -> i8 {
            if d < min || d > max {
                -1
            } else {
                density(d) as i8
            }
        };
        let tint = mix(p.accent, gpui_kit::transparent_black(), 0.11);
        let wash14 = mix(p.accent, gpui_kit::transparent_black(), 0.14);
        let focus = |kind: &str, d: Date, window: &mut Window, cx: &mut App| {
            let h = focus_handle(
                nid(&format!("{}-{kind}-focus", c.id), d.to_julian_day() as u64),
                window,
                cx,
            );
            let ring = focus_visible(&h, Force::None, window);
            (h, ring)
        };
        let months = c.body == Body::Months;

        let body: Div = match c.body {
            Body::Days => {
                let marks = c.range.as_ref().and_then(|r| r.marks);
                let days = month_cells(vfirst).into_iter().enumerate().map(|(i, d)| {
                    let off = heat(d) < 0;
                    let out = d.month() != vfirst.month();
                    let sel = c.range.is_none() && c.sel == Some(d);
                    let (within, start, end) = marks.map_or((false, false, false), |(s, e)| {
                        (d >= s && d <= e, d == s, d == e)
                    });
                    let solid = sel || start || end;
                    let tinted = !solid && (within || c.here == Some(d));
                    let fg = if solid {
                        p.accent_contrast
                    } else if tinted {
                        p.text_strong
                    } else if off {
                        p.text_faint
                    } else if out {
                        p.text_subtle
                    } else {
                        p.text
                    };
                    let col = i % 7;
                    let r = radius::SM;
                    let (h, ring) = match &c.grid {
                        Some(g) => (
                            g.focus.clone(),
                            g.cursor == Some(d) && focus_visible(&g.focus, Force::None, window),
                        ),
                        None => focus("day", d, window, cx),
                    };
                    let hover = (
                        p.wash_strong,
                        if out { p.text_subtle } else { p.text_strong },
                    );
                    cell(
                        nid(&format!("{}-day", c.id), d.to_julian_day() as u64),
                        size::CAL_CELL_W,
                        size::CAL_CELL_H,
                    )
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .aria_label(format!("{}, {} {}", d.weekday(), d.month(), d.day()))
                    .aria_selected(solid)
                    .pb(px(4.))
                    .font_weight(if solid {
                        theme::W_SEMIBOLD
                    } else {
                        theme::W_MEDIUM
                    })
                    .text_color(fg)
                    .when(solid, |el| el.bg(p.accent))
                    .when(tinted, |el| el.bg(tint))
                    // In-span days are square but for the span's caps and the row's edges.
                    .map(|el| {
                        if within {
                            el.when(start || col == 0, |el| el.rounded_l(r))
                                .when(end || col == 6, |el| el.rounded_r(r))
                        } else {
                            el.rounded(r)
                        }
                    })
                    .when(!off && c.grid.is_none(), |el| el.track_focus(&h))
                    .when(!off && !solid && !tinted, |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.bg(hover.0).text_color(hover.1))
                    })
                    .when(!off && within, |el| el.cursor_pointer())
                    .map(|el| match (&c.range, off) {
                        (_, true) => el,
                        (Some(r), _) => {
                            let (press, over) = (r.on_press.clone(), r.on_hover.clone());
                            el.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                press(d, window, cx)
                            })
                            .on_hover(move |on, window, cx| {
                                if *on {
                                    over(d, window, cx)
                                }
                            })
                        }
                        (None, _) => {
                            let pick = c.on_pick.clone();
                            el.on_click(move |_, window, cx| pick(d, window, cx))
                        }
                    })
                    .child(number(d.day().to_string(), d == c.today && !solid, &p))
                    .children(dot(
                        heat(d).max(0) as u8,
                        solid.then(|| alpha(p.accent_contrast, 0.7)),
                        &p,
                    ))
                    .when(ring, |el| el.child(inset_ring(&p)))
                });
                let grid = div()
                    .flex()
                    .flex_wrap()
                    .w(size::CAL_CELL_W * 7.)
                    .gap_y(px(2.))
                    .children(days);
                match &c.grid {
                    // `APP_CONTEXT` excludes "MxButton": arrows move the cursor, not the timeline.
                    Some(g) => {
                        let on_key = g.on_key.clone();
                        grid.track_focus(&g.focus)
                            .key_context("MxButton")
                            .on_key_down(move |e, window, cx| on_key(e, window, cx))
                    }
                    None => grid,
                }
            }
            Body::Weeks => {
                let cells = month_cells(vfirst);
                let rows = cells.chunks(7).map(|row| {
                    let (r0, r6) = (row[0], row[6]);
                    let disabled = r0 > max || r6 < min;
                    let sel = c.sel == Some(r0);
                    let (h, ring) = focus("week", r0, window, cx);
                    let pick = c.on_pick.clone();
                    let hover = p.wash_strong;
                    let days = row.iter().map(|&d| {
                        let off = heat(d) < 0;
                        let out = d.month() != vfirst.month();
                        let fg = if off || disabled {
                            p.text_faint
                        } else if sel {
                            p.text_strong
                        } else if out {
                            p.text_subtle
                        } else {
                            p.text
                        };
                        let strong = p.text_strong;
                        cell(
                            nid(&format!("{}-wday", c.id), d.to_julian_day() as u64),
                            size::CAL_CELL_W,
                            size::CAL_CELL_H,
                        )
                        .pb(px(4.))
                        .font_weight(if sel && !off {
                            theme::W_SEMIBOLD
                        } else {
                            theme::W_MEDIUM
                        })
                        .text_color(fg)
                        .when(!off, |el| {
                            el.group_hover(WEEK_GROUP, move |s| s.text_color(strong))
                        })
                        .child(number(d.day().to_string(), d == c.today, &p))
                        .when(!off, |el| el.children(dot(heat(d).max(0) as u8, None, &p)))
                    });
                    div()
                        .id(nid(&format!("{}-week", c.id), r0.to_julian_day() as u64))
                        .test_support()
                        .role(gpui_kit::Role::Button)
                        .aria_label(format!("Week {}, {}", r0.iso_week(), span_label(r0, r6)))
                        .aria_selected(sel)
                        .group(WEEK_GROUP)
                        .relative()
                        .flex()
                        .rounded(radius::SM)
                        .when(sel, |el| el.bg(wash14))
                        .when(!disabled, |el| {
                            el.track_focus(&h)
                                .cursor_pointer()
                                .when(!sel, |el| el.hover(move |s| s.bg(hover)))
                                .on_click(move |_, window, cx| pick(r0, window, cx))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .w(px(WN_W))
                                .pb(px(4.))
                                .font_family(FONT_MONO)
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::XS)
                                .line_height(gpui_kit::relative(1.))
                                .text_color(if sel { p.accent } else { p.text_faint })
                                .child(r0.iso_week().to_string()),
                        )
                        .children(days)
                        .when(ring, |el| el.child(inset_ring(&p)))
                });
                div().flex().flex_col().gap_y(px(2.)).children(rows)
            }
            Body::Months => {
                let w = px((MONTHS_W - 2. * MONTHS_GAP) / 3.);
                let cells = (1..=12u8).map(|m| {
                    let m0 =
                        Date::from_calendar_date(vfirst.year(), m.try_into().unwrap(), 1).unwrap();
                    let (_, last) = month_of(m0, 0);
                    // Mean density of the month's captured-range days (`periodHeat`).
                    let hs: Vec<i8> = (0..=(last - m0).whole_days())
                        .map(|i| heat(m0 + Duration::days(i)))
                        .filter(|h| *h >= 0)
                        .collect();
                    let disabled = hs.is_empty();
                    let mean = if disabled {
                        0
                    } else {
                        (hs.iter().map(|h| *h as f32).sum::<f32>() / hs.len() as f32).round() as u8
                    };
                    let sel = c.sel == Some(m0);
                    let now = m0.month() == c.today.month() && m0.year() == c.today.year();
                    let (h, ring) = focus("month", m0, window, cx);
                    let pick = c.on_pick.clone();
                    let hover = (p.wash_strong, p.text_strong);
                    cell(
                        nid(&format!("{}-month", c.id), m0.to_julian_day() as u64),
                        w,
                        px(44.),
                    )
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .aria_label(format!("{} {}", m0.month(), m0.year()))
                    .aria_selected(sel)
                    .pb(px(6.))
                    .rounded(radius::SM)
                    .font_weight(if sel {
                        theme::W_SEMIBOLD
                    } else {
                        theme::W_MEDIUM
                    })
                    .text_color(if sel {
                        p.text_strong
                    } else if disabled {
                        p.text_faint
                    } else {
                        p.text
                    })
                    .when(sel, |el| el.bg(wash14))
                    .when(!disabled, |el| {
                        el.track_focus(&h)
                            .cursor_pointer()
                            .when(!sel, |el| {
                                el.hover(move |s| s.bg(hover.0).text_color(hover.1))
                            })
                            .on_click(move |_, window, cx| pick(m0, window, cx))
                    })
                    .child(number(
                        m0.month().to_string()[..3].to_string(),
                        now && !sel,
                        &p,
                    ))
                    .children(dot(mean, sel.then_some(p.accent), &p))
                    .when(ring, |el| el.child(inset_ring(&p)))
                });
                div()
                    .flex()
                    .flex_wrap()
                    .w(px(MONTHS_W))
                    .gap(px(MONTHS_GAP))
                    .children(cells)
            }
        };

        let nav = |k: i32, disabled: bool| {
            let f = c.on_nav.clone();
            let (glyph, dir) = if k < 0 {
                (IconName::ChevronLeft, "Previous")
            } else {
                (IconName::ChevronRight, "Next")
            };
            let unit = if months { "year" } else { "month" };
            let step = if months { k * 12 } else { k };
            Button::icon_only(
                nid(&format!("{}-nav", c.id), (k + 1) as u64),
                glyph.path(),
                format!("{dir} {unit}"),
            )
            .ghost()
            .sm()
            .disabled(disabled)
            .on_click(move |_, window, cx| f(step, window, cx))
        };
        let (title, prev_off, next_off) = if months {
            (
                vfirst.year().to_string(),
                vfirst.year() <= min.year(),
                vfirst.year() >= max.year(),
            )
        } else {
            (
                format!("{} {}", vfirst.month(), vfirst.year()),
                vfirst <= month_of(min, 0).0,
                vfirst >= month_of(max, 0).0,
            )
        };
        let wd = |label: &'static str, w: f32| {
            div()
                .w(px(w))
                .h(px(22.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .font_family(FONT_MONO)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::XS)
                .tracking(0.06)
                .text_color(p.text_subtle)
                .child(label)
        };
        let weekdays = (!months).then(|| {
            div()
                .flex()
                .when(c.body == Body::Weeks, |el| el.child(wd("wk", WN_W)))
                .children(WEEKDAYS.iter().map(|w| wd(w, f32::from(size::CAL_CELL_W))))
        });
        let legend_dot = |s: f32, c: Hsla| div().size(px(s)).rounded_full().bg(c);
        let jumper = matches!(c.foot, Foot::Legend);
        let reset_pick = c.on_pick.clone();
        let tail = match c.foot {
            Foot::Reset { label, disabled } => Some(
                Button::new(format!("{}-reset", c.id), label)
                    .ghost()
                    .sm()
                    .disabled(disabled)
                    .on_click(move |_, window, cx| reset_pick(max, window, cx))
                    .into_any_element(),
            ),
            Foot::Hint(h) => Some(div().child(h).into_any_element()),
            Foot::Legend => None,
        };
        div()
            .flex()
            .flex_col()
            .pt(px(if jumper { 10. } else { 8. }))
            .px(px(10.))
            .pb(px(if jumper { 8. } else { 6. }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pb(px(8.))
                    .pl(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::MD)
                            .tracking(-0.01)
                            .line_height(gpui_kit::relative(1.))
                            .text_color(p.text_strong)
                            .child(title),
                    )
                    .child(nav(-1, prev_off))
                    .child(nav(1, next_off)),
            )
            .children(weekdays)
            .child(body)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .min_h(px(32.))
                    .mt(px(6.))
                    .pt(px(6.))
                    .pl(px(4.))
                    .border_t_1()
                    .border_color(p.hairline)
                    .font_family(FONT_MONO)
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .mr_auto()
                            .child(legend_dot(3., p.chart_grey_2))
                            .child(legend_dot(4., p.chart_grey_3))
                            .child(legend_dot(5., p.chart_grey_5).mr(px(3.)))
                            .child("captured"),
                    )
                    .children(tail),
            )
    }
}
