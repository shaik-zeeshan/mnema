//! The date math behind DateInput (`lib/components/date-input.ts`); the
//! `.mx-cal` popover body is `calendar_view.rs`.

use time::{Date, Duration, Month};

pub use super::calendar_view::{calendar, Body, CalSpec, Foot, GridKeys, OnPickDay, RangeHooks};

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

/// The range capsule's small line: a preset's name, else `October` /
/// `week of Oct 5` / `N days` (`relativeLabel`).
pub fn span_relative(a: Date, b: Date, today: Date) -> String {
    match presets(today)
        .into_iter()
        .find(|(_, s, e)| *s == a && *e == b)
    {
        Some((name, ..)) => name.into(),
        None => match unit_of(a, b) {
            Some(Unit::Month) => a.month().to_string(),
            Some(Unit::Week) => format!("week of {}", md(a)),
            _ => match (b - a).whole_days() + 1 {
                1 => "1 day".into(),
                n => format!("{n} days"),
            },
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

/// Which unit a span is (`unitOf`): `None` = a custom span.
pub fn unit_of(s: Date, e: Date) -> Option<Unit> {
    if s == e {
        return Some(Unit::Day);
    }
    if s == monday(s) && (e - s).whole_days() == 6 {
        return Some(Unit::Week);
    }
    (month_of(s, 0) == (s, e)).then_some(Unit::Month)
}

/// Step a range-mode span `k` of its own units (`stepSpan`): a custom span
/// moves by its length; `None` when it would leave `[min, max]`.
pub fn step_span(a: Date, b: Date, k: i32, min: Date, max: Date) -> Option<(Date, Date)> {
    match unit_of(a, b) {
        Some(u) => step(a, u, k, min, max),
        None => {
            let len = Duration::days(((b - a).whole_days() + 1) * k as i64);
            let (na, nb) = (a + len, b + len);
            (!(na > max || nb < min)).then_some((na, nb))
        }
    }
}

/// Home/End in range mode (`latestSpan`): the newest span of the same unit;
/// a custom span keeps its length.
pub fn latest_span(a: Date, b: Date, max: Date) -> (Date, Date) {
    match unit_of(a, b) {
        Some(u) => period_of(max, u),
        None => (max - (b - a), max),
    }
}

/// The month's pickable days (`getSelectableCells`: in the month, inside `[min, max]`).
fn pickable(first: Date, min: Date, max: Date) -> Vec<Date> {
    let (f, l) = month_of(first, 0);
    (0..=(l - f).whole_days())
        .map(|i| f + Duration::days(i))
        .filter(|d| *d >= min && *d <= max)
        .collect()
}

/// bits-ui's `shiftCalendarFocus` (the Timeline jumper's arrow keys): move the
/// keyboard cursor `add` places (±1 a day, ±7 a week) through the month's
/// pickable days, rolling the overflow into the month before / after unless
/// that month is out of `[min, max]`. `(cursor, first of the month to show)`.
pub fn shift_cursor(
    cursor: Date,
    add: i32,
    view: Date,
    min: Date,
    max: Date,
) -> Option<(Date, Date)> {
    let first = month_of(view, 0).0;
    let here = pickable(first, min, max);
    let at = |days: &[Date], i: i32| (i >= 0).then(|| days.get(i as usize).copied()).flatten();
    let next = here.iter().position(|d| *d == cursor)? as i32 + add;
    if let Some(d) = at(&here, next) {
        return Some((d, first));
    }
    let (other, i) = if next < 0 {
        if first <= month_of(min, 0).0 {
            return None;
        }
        let prev = month_of(first, -1).0;
        (prev, pickable(prev, min, max).len() as i32 + next)
    } else {
        if first >= month_of(max, 0).0 {
            return None;
        }
        (month_of(first, 1).0, next - here.len() as i32)
    };
    at(&pickable(other, min, max), i).map(|d| (d, other))
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
        // Arrow keys: a week down from Oct 28 rolls into November at the same offset.
        let (min, view) = (date!(2026 - 06 - 01), date!(2026 - 10 - 01));
        assert_eq!(
            shift_cursor(date!(2026 - 10 - 05), 7, view, min, today),
            None,
            "October ends at today (Oct 9) and November is past max"
        );
        let max = date!(2026 - 12 - 31);
        assert_eq!(
            shift_cursor(date!(2026 - 10 - 28), 7, view, min, max),
            Some((date!(2026 - 11 - 04), date!(2026 - 11 - 01)))
        );
        assert_eq!(
            shift_cursor(date!(2026 - 10 - 01), -1, view, min, max),
            Some((date!(2026 - 09 - 30), date!(2026 - 09 - 01)))
        );
        assert_eq!(
            shift_cursor(date!(2026 - 10 - 05), 1, view, min, max),
            Some((date!(2026 - 10 - 06), view))
        );
    }
}
