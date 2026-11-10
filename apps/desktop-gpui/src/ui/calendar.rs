//! The date math behind DateInput (`lib/components/date-input.ts`) and the
//! `.mx-cal` day grid: Monday-first weeks, density dots, today underline,
//! selected / in-period fills.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, App, Div, Hsla, SharedString, Window};
use time::{Date, Duration, Month};

use super::{alpha, mix, nid, Button};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
}

impl Unit {
    pub fn name(self) -> &'static str {
        match self {
            Unit::Day => "day",
            Unit::Week => "week",
            Unit::Month => "month",
        }
    }
}

pub fn monday(d: Date) -> Date {
    d - Duration::days(d.weekday().number_days_from_monday() as i64)
}

/// First and last day of the month `k` months from `d`'s.
pub fn month_of(d: Date, k: i32) -> (Date, Date) {
    let idx = d.year() * 12 + d.month() as i32 - 1 + k;
    let (y, m) = (
        idx.div_euclid(12),
        Month::try_from((idx.rem_euclid(12) + 1) as u8).unwrap(),
    );
    let first = Date::from_calendar_date(y, m, 1).unwrap();
    (first, first.replace_day(first.month().length(y)).unwrap())
}

pub fn period_of(d: Date, unit: Unit) -> (Date, Date) {
    match unit {
        Unit::Day => (d, d),
        Unit::Week => (monday(d), monday(d) + Duration::days(6)),
        Unit::Month => month_of(d, 0),
    }
}

/// Step one period `k` units; `None` when it would leave `[min, max]`.
pub fn step(a: Date, unit: Unit, k: i32, min: Date, max: Date) -> Option<(Date, Date)> {
    let (na, nb) = match unit {
        Unit::Day => (a + Duration::days(k as i64), a + Duration::days(k as i64)),
        Unit::Week => {
            let s = monday(a) + Duration::weeks(k as i64);
            (s, s + Duration::days(6))
        }
        Unit::Month => month_of(a, k),
    };
    let out = match unit {
        Unit::Day => na < min || na > max,
        _ => na > max || nb < min,
    };
    (!out).then_some((na, nb))
}

pub fn periods_ago(a: Date, unit: Unit, today: Date) -> i64 {
    match unit {
        Unit::Month => {
            (today.year() - a.year()) as i64 * 12 + today.month() as i64 - a.month() as i64
        }
        Unit::Week => (monday(today) - monday(a)).whole_days() / 7,
        Unit::Day => (today - a).whole_days(),
    }
}

fn short_month(d: Date) -> String {
    d.month().to_string()[..3].to_string()
}

fn md(d: Date) -> String {
    format!("{} {}", short_month(d), d.day())
}

/// The capsule's bold line: `Tue, Oct 7` (day mode) · `Oct 9` · `Oct 5 – 11` · `October 2026`.
pub fn capsule_label(period: Option<Unit>, a: Date, b: Date, today: Date) -> String {
    match period {
        None => format!("{}, {}", &a.weekday().to_string()[..3], md(a)),
        Some(Unit::Week) if a.month() == b.month() => format!("{} – {}", md(a), b.day()),
        Some(Unit::Week) => format!("{} – {}", md(a), md(b)),
        Some(Unit::Month) => format!("{} {}", a.month(), a.year()),
        Some(Unit::Day) if a.year() != today.year() => format!("{}, {}", md(a), a.year()),
        Some(Unit::Day) => md(a),
    }
}

/// The capsule's small line: `today` · `yesterday` · `3 days ago` · `last week` · `this month`.
pub fn relative_label(period: Option<Unit>, a: Date, today: Date) -> String {
    let unit = period.unwrap_or(Unit::Day);
    let k = periods_ago(a, unit, today);
    match (k, unit) {
        (0, Unit::Day) => "today".into(),
        (0, u) => format!("this {}", u.name()),
        (1, Unit::Day) => "yesterday".into(),
        (1, u) => format!("last {}", u.name()),
        (k, Unit::Day) => format!("{k} days ago"),
        (k, u) => format!("{k} {}s ago", u.name()),
    }
}

/// The range presets (`presets(today)`), label + inclusive days.
pub fn presets(today: Date) -> Vec<(&'static str, Date, Date)> {
    let m0 = monday(today);
    let d = Duration::days;
    let (ms, me) = month_of(today, 0);
    let (ls, le) = month_of(today, -1);
    vec![
        ("Today", today, today),
        ("Yesterday", today - d(1), today - d(1)),
        ("This week", m0, m0 + d(6)),
        ("Last week", m0 - d(7), m0 - d(1)),
        ("Last 7 days", today - d(6), today),
        ("Last 30 days", today - d(29), today),
        ("This month", ms, me),
        ("Last month", ls, le),
    ]
}

/// "Oct 3" / "Oct 3 – 9" / "Sep 28 – Oct 4" (`spanLabel`).
pub fn span_label(a: Date, b: Date) -> String {
    if a == b {
        md(a)
    } else if a.month() == b.month() {
        format!("{} – {}", md(a), b.day())
    } else {
        format!("{} – {}", md(a), md(b))
    }
}

/// The range capsule's small line: a preset's name, else `N days`.
pub fn span_relative(a: Date, b: Date, today: Date) -> String {
    match presets(today)
        .into_iter()
        .find(|(_, s, e)| *s == a && *e == b)
    {
        Some((name, ..)) => name.into(),
        None => match (b - a).whole_days() + 1 {
            1 => "1 day".into(),
            n => format!("{n} days"),
        },
    }
}

/// The month grid's days: Monday of the 1st's week through Sunday of the last's.
pub fn month_cells(view: Date) -> Vec<Date> {
    let (first, last) = month_of(view, 0);
    let (start, end) = (monday(first), monday(last) + Duration::days(6));
    let n = (end - start).whole_days() + 1;
    (0..n).map(|i| start + Duration::days(i)).collect()
}

pub const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

/// Density dot colour/size for 0..=3 captured (0 = no dot).
fn dot(h: u8, p: &Palette) -> Option<(f32, Hsla)> {
    match h {
        0 => None,
        1 => Some((3., p.chart_grey_2)),
        2 => Some((4., p.chart_grey_3)),
        _ => Some((5., p.chart_grey_5)),
    }
}

pub type OnPickDay = Rc<dyn Fn(Date, &mut Window, &mut App)>;

/// Everything one `.mx-cal` render needs.
pub struct CalSpec {
    pub id: SharedString,
    pub view: Date,
    pub today: Date,
    pub min: Date,
    pub max: Date,
    /// The selected span (`a == b` for a single day).
    pub sel: (Date, Date),
    pub density: Rc<dyn Fn(Date) -> u8>,
    pub reset_label: SharedString,
    /// The selection already holds the latest day / the current period.
    pub at_now: bool,
    pub on_pick: OnPickDay,
    pub on_nav: Rc<dyn Fn(i32, &mut Window, &mut App)>,
}

pub fn calendar(c: CalSpec, p: &Palette) -> Div {
    let (vfirst, _) = month_of(c.view, 0);
    let single = c.sel.0 == c.sel.1;
    let nav = |k: i32, disabled: bool| {
        let f = c.on_nav.clone();
        let (glyph, label) = if k < 0 {
            (IconName::ChevronLeft, "Previous month")
        } else {
            (IconName::ChevronRight, "Next month")
        };
        Button::icon_only(
            nid(&format!("{}-nav", c.id), (k + 1) as u64),
            glyph.path(),
            label,
        )
        .ghost()
        .sm()
        .disabled(disabled)
        .on_click(move |_, window, cx| f(k, window, cx))
    };
    let days = month_cells(c.view).into_iter().enumerate().map(|(i, d)| {
        let off = d < c.min || d > c.max;
        let out = d.month() != vfirst.month();
        let sel = single && d == c.sel.0;
        let within = !single && d >= c.sel.0 && d <= c.sel.1;
        let (fg, bg) = if sel {
            (p.accent_contrast, Some(p.accent))
        } else if off {
            (p.text_faint, None)
        } else if within {
            (
                p.text_strong,
                Some(mix(p.accent, gpui_kit::transparent_black(), 0.11)),
            )
        } else if out {
            (p.text_subtle, None)
        } else {
            (p.text, None)
        };
        let col = i % 7;
        let r = radius::SM;
        let pick = c.on_pick.clone();
        let hover = (p.wash_strong, p.text_strong);
        let dot = (!off).then(|| dot((c.density)(d), p)).flatten();
        div()
            .id(nid(&format!("{}-day", c.id), d.to_julian_day() as u64))
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(format!("{}, {} {}", d.weekday(), d.month(), d.day()))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w(size::CAL_CELL_W)
            .h(size::CAL_CELL_H)
            .pb(px(4.))
            .text_size(text::BASE)
            .font_weight(if sel {
                theme::W_SEMIBOLD
            } else {
                theme::W_MEDIUM
            })
            .line_height(gpui_kit::relative(1.))
            .text_color(fg)
            .when_some(bg, |el, bg| el.bg(bg))
            .map(|el| {
                if within {
                    let start = d == c.sel.0 || col == 0;
                    let end = d == c.sel.1 || col == 6;
                    el.when(start, |el| el.rounded_l(r))
                        .when(end, |el| el.rounded_r(r))
                } else {
                    el.rounded(r)
                }
            })
            .when(!off && !sel && !within, |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover.0).text_color(hover.1))
            })
            .when(!off, |el| {
                el.on_click(move |_, window, cx| pick(d, window, cx))
            })
            .child(
                div()
                    .when(d == c.today && !sel, |el| {
                        el.border_b(px(1.5)).border_color(p.accent)
                    })
                    .child(d.day().to_string()),
            )
            .when_some(dot, |el, (s, color)| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(5.))
                        .left(size::CAL_CELL_W / 2. - px(s / 2.))
                        .size(px(s))
                        .rounded_full()
                        .bg(if sel {
                            alpha(p.accent_contrast, 0.7)
                        } else {
                            color
                        }),
                )
            })
    });
    let legend_dot = |s: f32, c: Hsla| div().size(px(s)).rounded_full().bg(c);
    let reset_pick = c.on_pick.clone();
    let max = c.max;
    div()
        .flex()
        .flex_col()
        .pt(px(8.))
        .px(px(10.))
        .pb(px(6.))
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
                        .line_height(gpui_kit::relative(1.))
                        .text_color(p.text_strong)
                        .child(format!("{} {}", vfirst.month(), vfirst.year())),
                )
                .child(nav(-1, vfirst <= month_of(c.min, 0).0))
                .child(nav(1, vfirst >= month_of(c.max, 0).0)),
        )
        .child(div().flex().children(WEEKDAYS.iter().map(|w| {
            div()
                .w(size::CAL_CELL_W)
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .font_family(FONT_MONO)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(*w)
        })))
        .child(
            div()
                .flex()
                .flex_wrap()
                .w(size::CAL_CELL_W * 7.)
                .gap_y(px(2.))
                .children(days),
        )
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
                .child(
                    Button::new(format!("{}-reset", c.id), c.reset_label.clone())
                        .ghost()
                        .sm()
                        .disabled(c.at_now)
                        .on_click(move |_, window, cx| reset_pick(max, window, cx)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    #[test]
    fn labels_and_periods_match_date_input_ts() {
        let today = date!(2026 - 10 - 09);
        assert_eq!(
            capsule_label(None, date!(2026 - 10 - 07), date!(2026 - 10 - 07), today),
            "Wed, Oct 7"
        );
        assert_eq!(
            relative_label(None, date!(2026 - 10 - 07), today),
            "2 days ago"
        );
        assert_eq!(relative_label(None, today, today), "today");
        let (a, b) = period_of(today, Unit::Week);
        assert_eq!((a, b), (date!(2026 - 10 - 05), date!(2026 - 10 - 11)));
        assert_eq!(capsule_label(Some(Unit::Week), a, b, today), "Oct 5 – 11");
        assert_eq!(relative_label(Some(Unit::Week), a, today), "this week");
        let (m, _) = month_of(today, -1);
        assert_eq!(
            capsule_label(Some(Unit::Month), m, m, today),
            "September 2026"
        );
        assert_eq!(relative_label(Some(Unit::Month), m, today), "last month");
        assert_eq!(month_of(date!(2026 - 01 - 15), -1).0, date!(2025 - 12 - 01));
        assert_eq!(
            step(today, Unit::Day, 1, date!(2026 - 08 - 20), today),
            None
        );
        assert_eq!(
            step(today, Unit::Day, -1, date!(2026 - 08 - 20), today).map(|s| s.0),
            Some(date!(2026 - 10 - 08))
        );
        let cells = month_cells(today);
        assert_eq!(cells.first(), Some(&date!(2026 - 09 - 28)));
        assert_eq!(cells.len() % 7, 0);
        assert_eq!(cells.last(), Some(&date!(2026 - 11 - 01)));
    }
}
