// --- Date resolution (ADR 0019, A3) ---
//
// All date operators resolve to frozen concrete instants at parse time in the
// LOCAL timezone, both bounds inclusive at day granularity. The sound local
// offset is obtained per-calendar-date via chrono::Local (mirroring
// capture_retention), avoiding the `time` crate's unsound `local-offset`
// feature. Week start defaults to Monday (locale first weekday).

use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

/// The current local calendar date, used as the anchor for relative tokens.
pub(super) fn local_today_date() -> time::Date {
    use chrono::Datelike;
    let now = chrono::Local::now().date_naive();
    time::Date::from_calendar_date(
        now.year(),
        time::Month::try_from(now.month() as u8).unwrap_or(time::Month::January),
        now.day() as u8,
    )
    .unwrap_or_else(|_| OffsetDateTime::now_utc().date())
}

/// Resolves an `after:`/`before:` point to a single calendar date. Accepts an
/// absolute `YYYY-MM-DD`, or a relative point: `today`, `yesterday`, `Nd`
/// (N days ago), `Nh` (N hours ago, resolved to that day).
pub(super) fn resolve_point_date(value: &str, today: time::Date) -> Option<time::Date> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match value.to_lowercase().as_str() {
        "today" => return Some(today),
        "yesterday" => return today.previous_day(),
        _ => {}
    }

    if let Ok(date) = time::Date::parse(value, &time::format_description::well_known::Iso8601::DATE)
    {
        return Some(date);
    }

    if let Some(days) = parse_relative_count(value, 'd') {
        return today.checked_sub(time::Duration::days(days));
    }
    if let Some(hours) = parse_relative_count(value, 'h') {
        // `Nh` resolves to the day N hours before local "now".
        let now = local_now_offset_datetime();
        let resolved = now.checked_sub(time::Duration::hours(hours))?;
        return Some(resolved.date());
    }

    None
}

/// Resolves a `date:` value to an inclusive `(start_date, end_date)` span:
/// a single day, or a named period (today, yesterday, last/this week/month).
pub(super) fn resolve_day_or_period(
    value: &str,
    today: time::Date,
) -> Option<(time::Date, time::Date)> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match value.to_lowercase().as_str() {
        "today" => return Some((today, today)),
        "yesterday" => {
            let yesterday = today.previous_day()?;
            return Some((yesterday, yesterday));
        }
        "this-week" => return Some(week_span(today, 0)),
        "last-week" => return Some(week_span(today, -1)),
        "this-month" => return Some(month_span(today, 0)),
        "last-month" => return Some(month_span(today, -1)),
        _ => {}
    }

    if let Ok(date) = time::Date::parse(value, &time::format_description::well_known::Iso8601::DATE)
    {
        return Some((date, date));
    }

    None
}

/// Returns the inclusive Monday..Sunday span for the week containing `today`,
/// shifted by `week_offset` weeks. Week start = Monday (locale default).
pub(super) fn week_span(today: time::Date, week_offset: i64) -> (time::Date, time::Date) {
    let weekday_from_monday = today.weekday().number_days_from_monday() as i64;
    let monday = today
        .checked_sub(time::Duration::days(weekday_from_monday))
        .unwrap_or(today);
    let start = monday
        .checked_add(time::Duration::weeks(week_offset))
        .unwrap_or(monday);
    let end = start.checked_add(time::Duration::days(6)).unwrap_or(start);
    (start, end)
}

/// Returns the inclusive first..last day span for the month containing `today`,
/// shifted by `month_offset` months.
pub(super) fn month_span(today: time::Date, month_offset: i64) -> (time::Date, time::Date) {
    let (mut year, mut month_index) =
        (today.year() as i64, today.month() as i64 - 1 + month_offset);
    year += month_index.div_euclid(12);
    month_index = month_index.rem_euclid(12);
    let month = time::Month::try_from((month_index + 1) as u8).unwrap_or(time::Month::January);
    let start = time::Date::from_calendar_date(year as i32, month, 1).unwrap_or(today);
    let last_day = days_in_month(year as i32, month);
    let end = time::Date::from_calendar_date(year as i32, month, last_day).unwrap_or(start);
    (start, end)
}

pub(super) fn days_in_month(year: i32, month: time::Month) -> u8 {
    match month {
        time::Month::January
        | time::Month::March
        | time::Month::May
        | time::Month::July
        | time::Month::August
        | time::Month::October
        | time::Month::December => 31,
        time::Month::April | time::Month::June | time::Month::September | time::Month::November => {
            30
        }
        time::Month::February => {
            if time::util::is_leap_year(year) {
                29
            } else {
                28
            }
        }
    }
}

/// Parses a relative count like `7d`/`1h`. Returns the numeric magnitude when
/// the value is digits followed by exactly the expected unit char.
pub(super) fn parse_relative_count(value: &str, unit: char) -> Option<i64> {
    let lower = value.to_lowercase();
    let stripped = lower.strip_suffix(unit)?;
    if stripped.is_empty() || !stripped.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    stripped.parse::<i64>().ok()
}

/// The local offset for a given calendar date, resolved soundly through
/// chrono::Local (see capture_retention::local_midnight_offset). Falls back to
/// the current local offset, then UTC.
pub(super) fn local_offset_for_date(date: time::Date) -> UtcOffset {
    use chrono::{Local, LocalResult, NaiveDate, Offset, TimeZone};
    let resolved = NaiveDate::from_ymd_opt(
        date.year(),
        u32::from(u8::from(date.month())),
        u32::from(date.day()),
    )
    .and_then(|local_date| local_date.and_hms_opt(0, 0, 0))
    .and_then(
        |local_midnight| match Local.from_local_datetime(&local_midnight) {
            LocalResult::Single(datetime) => Some(datetime.offset().fix().local_minus_utc()),
            LocalResult::Ambiguous(earliest, _) => Some(earliest.offset().fix().local_minus_utc()),
            LocalResult::None => None,
        },
    )
    .and_then(|offset_seconds| UtcOffset::from_whole_seconds(offset_seconds).ok());
    resolved.unwrap_or_else(|| local_now_offset_datetime().offset())
}

/// Current instant as an `OffsetDateTime` carrying the local offset, resolved
/// through chrono (the `time` crate's `now_local` is feature-gated/unsound).
pub(super) fn local_now_offset_datetime() -> OffsetDateTime {
    use chrono::Offset;
    let now = chrono::Local::now();
    let offset_seconds = now.offset().fix().local_minus_utc();
    let offset = UtcOffset::from_whole_seconds(offset_seconds).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::now_utc().to_offset(offset)
}

/// `after:D` → D 00:00:00 local, formatted as RFC3339 for the existing
/// `normalize_search_refinements` date path (which converts to UTC).
pub(super) fn start_of_day_rfc3339(date: time::Date) -> String {
    let offset = local_offset_for_date(date);
    date.with_hms_milli(0, 0, 0, 0)
        .expect("midnight is always valid")
        .assume_offset(offset)
        .format(&Rfc3339)
        .expect("RFC3339 formatting of a valid datetime should succeed")
}

/// `before:D` → D 23:59:59.999 local, formatted as RFC3339.
pub(super) fn end_of_day_rfc3339(date: time::Date) -> String {
    let offset = local_offset_for_date(date);
    date.with_hms_milli(23, 59, 59, 999)
        .expect("end-of-day is always valid")
        .assume_offset(offset)
        .format(&Rfc3339)
        .expect("RFC3339 formatting of a valid datetime should succeed")
}

/// A wide-open lower bound used when only an upper date bound is supplied.
pub(super) fn open_lower_bound_rfc3339() -> String {
    "0001-01-01T00:00:00Z".to_string()
}

/// A wide-open upper bound used when only a lower date bound is supplied.
pub(super) fn open_upper_bound_rfc3339() -> String {
    "9999-12-31T23:59:59.999Z".to_string()
}
