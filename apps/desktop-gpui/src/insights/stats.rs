//! The Insights figures, pure: `lede-stats.ts` + `activity-helpers.ts`. Same
//! label, same computation on Overview and Journal. Local-calendar math runs in
//! the clock's offset (frozen in goldens), like `new Date()` in the webview.

use capture_types::{Activity, ActivityCategory, FocusLevel, HeatmapBucket};
use gpui_kit::Hsla;
use time::{Date, Duration, OffsetDateTime, Time, UtcOffset};

use crate::theme::Palette;
use crate::ui::calendar::{period_of, Unit};

pub const DAY_MS: i64 = 86_400_000;
pub const HOUR_MS: i64 = 3_600_000;

/// insights.html's `CAT` order: the legend and the stack.
pub const CATEGORY_ORDER: [ActivityCategory; 8] = [
    ActivityCategory::Creating,
    ActivityCategory::Meetings,
    ActivityCategory::Research,
    ActivityCategory::Communication,
    ActivityCategory::Learning,
    ActivityCategory::Entertainment,
    ActivityCategory::Organizing,
    ActivityCategory::Personal,
];

pub fn category_label(c: Option<ActivityCategory>) -> &'static str {
    match c {
        None => "Uncategorized",
        Some(ActivityCategory::Creating) => "Creating",
        Some(ActivityCategory::Communication) => "Communication",
        Some(ActivityCategory::Meetings) => "Meetings",
        Some(ActivityCategory::Research) => "Research",
        Some(ActivityCategory::Learning) => "Learning",
        Some(ActivityCategory::Organizing) => "Organizing",
        Some(ActivityCategory::Personal) => "Personal",
        Some(ActivityCategory::Entertainment) => "Entertainment",
    }
}

/// `CATEGORY_COLOR` / `UNCATEGORIZED_COLOR`.
pub fn category_color(c: Option<ActivityCategory>, p: &Palette) -> Hsla {
    match c {
        None => p.chart_grey_3,
        Some(ActivityCategory::Creating) => p.cat_creating,
        Some(ActivityCategory::Communication) => p.cat_communication,
        Some(ActivityCategory::Meetings) => p.cat_meetings,
        Some(ActivityCategory::Research) => p.cat_research,
        Some(ActivityCategory::Learning) => p.cat_learning,
        Some(ActivityCategory::Organizing) => p.cat_organizing,
        Some(ActivityCategory::Personal) => p.cat_personal,
        Some(ActivityCategory::Entertainment) => p.cat_entertainment,
    }
}

pub fn focus_color(f: FocusLevel, p: &Palette) -> Hsla {
    match f {
        FocusLevel::Deep => p.focus_deep,
        FocusLevel::Mixed => p.focus_mid,
        FocusLevel::Distracted => p.focus_distracted,
    }
}

/// `focusHint`.
pub fn focus_label(f: FocusLevel) -> &'static str {
    match f {
        FocusLevel::Deep => "Deep",
        FocusLevel::Mixed => "Mixed",
        FocusLevel::Distracted => "Scattered",
    }
}

// ── Time ────────────────────────────────────────────────────────────────

pub fn local(ms: i64, off: UtcOffset) -> OffsetDateTime {
    crate::timeline::model::local(ms, off)
}

pub fn ms_of(t: OffsetDateTime) -> i64 {
    (t.unix_timestamp_nanos() / 1_000_000) as i64
}

/// Local midnight of `d`.
pub fn date_ms(d: Date, off: UtcOffset) -> i64 {
    ms_of(d.with_time(Time::MIDNIGHT).assume_offset(off))
}

pub fn start_of_day(ms: i64, off: UtcOffset) -> i64 {
    date_ms(local(ms, off).date(), off)
}

/// `windowFor(anchor, unit)`: the local-calendar period holding `d`, `[start, end)`.
pub fn window(d: Date, unit: Unit, off: UtcOffset) -> (i64, i64) {
    let (a, b) = period_of(d, unit);
    (date_ms(a, off), date_ms(b + Duration::days(1), off))
}

/// "2:32 PM" (12-hour, like the parity fixture).
pub fn clock(ms: i64, off: UtcOffset) -> String {
    crate::timeline::model::clock_label(local(ms, off), false)
}

/// `formatClockRange`: "9:20–10:50 AM" (a shared AM/PM said once).
pub fn clock_range(a: i64, b: i64, off: UtcOffset, sep: &str) -> String {
    let (x, y) = (clock(a, off), clock(b, off));
    match (
        x.strip_suffix(" AM"),
        y.ends_with(" AM"),
        x.strip_suffix(" PM"),
        y.ends_with(" PM"),
    ) {
        (Some(s), true, _, _) | (_, _, Some(s), true) => format!("{s}{sep}{y}"),
        _ => format!("{x}{sep}{y}"),
    }
}

/// `formatHour`: "8 AM".
pub fn hour_label(h: u8) -> String {
    let h = h % 24;
    let twelve = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{twelve} {}", if h < 12 { "AM" } else { "PM" })
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn weekday(ms: i64, off: UtcOffset) -> &'static str {
    WEEKDAYS[local(ms, off).weekday().number_days_from_monday() as usize]
}

/// "Nov 4".
pub fn month_day(ms: i64, off: UtcOffset) -> String {
    let t = local(ms, off);
    format!("{} {}", MONTHS[t.month() as usize - 1], t.day())
}

/// "Wed, Nov 4".
pub fn day_label(ms: i64, off: UtcOffset) -> String {
    format!("{}, {}", weekday(ms, off), month_day(ms, off))
}

pub fn month_name(d: Date) -> String {
    d.month().to_string()
}

// ── Durations ───────────────────────────────────────────────────────────

/// `fmtDuration`: "6h 09m" · "47m".
pub fn fmt_duration(ms: i64) -> String {
    let m = ((ms.max(0) as f64) / 60_000.).round() as i64;
    if m >= 60 {
        format!("{}h {:02}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

/// `humanizeMs`: "45s" · "12m" · "2h" · "2h 5m".
pub fn humanize_ms(ms: i64) -> String {
    if ms <= 0 {
        return "0s".into();
    }
    let sec = ((ms as f64) / 1000.).round() as i64;
    if sec < 60 {
        return format!("{sec}s");
    }
    let min = ((ms as f64) / 60_000.).round() as i64;
    let (h, m) = (min / 60, min % 60);
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// "+2h 10m" / "−4m".
pub fn signed_duration(ms: i64) -> String {
    format!(
        "{}{}",
        if ms < 0 { "−" } else { "+" },
        fmt_duration(ms.abs())
    )
}

/// Chart axis tops, in minutes.
const AXIS_STEPS_MIN: [i64; 9] = [20, 40, 60, 120, 240, 480, 720, 960, 1440];

/// `axisMax`.
pub fn axis_max(peak_ms: f64) -> f64 {
    let m = peak_ms / 60_000.;
    let step = AXIS_STEPS_MIN
        .iter()
        .find(|s| m <= **s as f64)
        .map(|s| *s as f64)
        .unwrap_or_else(|| (m / 480.).ceil() * 480.);
    step * 60_000.
}

/// `fmtAxisTick`: "15m" · "2h" · "1.5h".
pub fn fmt_axis_tick(ms: f64) -> String {
    let m = (ms / 60_000.).round() as i64;
    if m < 60 {
        format!("{m}m")
    } else {
        let h = m as f64 / 60.;
        let s = format!("{:.1}", h);
        format!("{}h", s.trim_end_matches(".0"))
    }
}

// ── Activity figures ────────────────────────────────────────────────────

fn overlap(a: &Activity, s: i64, e: i64) -> i64 {
    (a.ended_at_ms.min(e) - a.started_at_ms.max(s)).max(0)
}

/// Activities overlapping `[start, end)` (start inclusive, like Overview's filter).
pub fn in_range(acts: &[Activity], start: i64, end: i64) -> Vec<Activity> {
    acts.iter()
        .filter(|a| a.started_at_ms < end && a.ended_at_ms >= start)
        .cloned()
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryRow {
    pub category: Option<ActivityCategory>,
    pub ms: i64,
    pub pct: i64,
}

/// Whole percent of `v` in `sum` (0 when empty).
pub fn fmt_pct(v: i64, sum: i64) -> i64 {
    round_pct(v, sum)
}

fn round_pct(v: i64, sum: i64) -> i64 {
    if sum > 0 {
        ((v as f64 / sum as f64) * 100.).round() as i64
    } else {
        0
    }
}

/// `categoryRows`: activity time per category inside `[s, e)`, largest first.
pub fn category_rows(acts: &[Activity], s: i64, e: i64) -> Vec<CategoryRow> {
    let keys: Vec<Option<ActivityCategory>> = CATEGORY_ORDER
        .iter()
        .map(|c| Some(*c))
        .chain([None])
        .collect();
    let totals: Vec<i64> = keys
        .iter()
        .map(|k| {
            acts.iter()
                .filter(|a| a.category == *k)
                .map(|a| overlap(a, s, e))
                .sum()
        })
        .collect();
    let sum: i64 = totals.iter().sum();
    let mut rows: Vec<CategoryRow> = keys
        .into_iter()
        .zip(totals)
        .filter(|(_, ms)| *ms > 0)
        .map(|(category, ms)| CategoryRow {
            category,
            ms,
            pct: round_pct(ms, sum),
        })
        .collect();
    rows.sort_by(|a, b| b.ms.cmp(&a.ms));
    rows
}

/// The busiest category (`computeTopCategory`): named categories win ties in
/// `CATEGORY_ORDER`; Uncategorized only when strictly larger.
pub fn top_category(acts: &[Activity], s: i64, e: i64) -> Option<CategoryRow> {
    let rows = category_rows(acts, s, e);
    let best = CATEGORY_ORDER
        .iter()
        .filter_map(|c| rows.iter().find(|r| r.category == Some(*c)))
        .fold(None::<&CategoryRow>, |b, r| match b {
            Some(b) if b.ms >= r.ms => Some(b),
            _ => Some(r),
        });
    let uncat = rows.iter().find(|r| r.category.is_none());
    match (best, uncat) {
        (Some(b), Some(u)) if u.ms > b.ms => Some(u.clone()),
        (None, Some(u)) => Some(u.clone()),
        (b, _) => b.cloned(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FocusCounts {
    pub deep: usize,
    pub mixed: usize,
    pub distracted: usize,
    pub n: usize,
}

pub fn focus_counts(acts: &[Activity]) -> FocusCounts {
    let mut c = FocusCounts::default();
    for f in acts.iter().filter_map(|a| a.focus) {
        match f {
            FocusLevel::Deep => c.deep += 1,
            FocusLevel::Mixed => c.mixed += 1,
            FocusLevel::Distracted => c.distracted += 1,
        }
        c.n += 1;
    }
    c
}

fn deep_share(c: FocusCounts) -> Option<i64> {
    (c.n > 0).then(|| ((c.deep as f64 / c.n as f64) * 100.).round() as i64)
}

/// `deepFocusDelta`, in points.
pub fn deep_focus_delta(cur: &[Activity], prev: &[Activity]) -> Option<i64> {
    Some(deep_share(focus_counts(cur))? - deep_share(focus_counts(prev))?)
}

pub fn deep_pct(acts: &[Activity]) -> Option<i64> {
    deep_share(focus_counts(acts))
}

// ── Columns ("Where the <unit> went") ───────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub start: i64,
    pub end: i64,
    pub label: String,
    pub live: bool,
    pub future: bool,
    /// Stacked bottom-up in `CATEGORY_ORDER` (+ uncategorized last); `None` key
    /// with `app` = capture-only grey.
    pub parts: Vec<(Option<ActivityCategory>, i64)>,
    pub app_time: bool,
    pub total: i64,
}

fn slots(start: i64, end: i64, unit: Unit, now: i64, off: UtcOffset) -> Vec<Column> {
    let day0 = local(start, off).date();
    let mut out = Vec::new();
    for i in 0.. {
        let (s, e, label) = if unit == Unit::Day {
            let s = date_ms(day0, off) + i * HOUR_MS;
            (s, s + HOUR_MS, hour_label(i as u8))
        } else {
            let d = day0 + Duration::days(i);
            let label = if unit == Unit::Week {
                weekday(date_ms(d, off), off).to_string()
            } else {
                d.day().to_string()
            };
            (date_ms(d, off), date_ms(d + Duration::days(1), off), label)
        };
        if s >= end {
            break;
        }
        out.push(Column {
            start: s,
            end: e,
            label,
            live: now >= s && now < e,
            future: s > now,
            parts: Vec::new(),
            app_time: false,
            total: 0,
        });
    }
    out
}

/// Day mode keeps the blocks from the first to the last with anything (through "now").
fn trim(cols: Vec<Column>, unit: Unit) -> Vec<Column> {
    if unit != Unit::Day {
        return cols;
    }
    let hit: Vec<usize> = (0..cols.len())
        .filter(|i| cols[*i].total > 0 || cols[*i].live)
        .collect();
    match (hit.first(), hit.last()) {
        (Some(a), Some(b)) => cols[*a..=*b].to_vec(),
        _ => Vec::new(),
    }
}

/// `activityColumns`: category-stacked activity time per slot.
pub fn activity_columns(
    acts: &[Activity],
    (start, end): (i64, i64),
    unit: Unit,
    now: i64,
    off: UtcOffset,
) -> Vec<Column> {
    let cols = slots(start, end, unit, now, off)
        .into_iter()
        .map(|mut c| {
            let rows = category_rows(acts, c.start, c.end);
            c.parts = CATEGORY_ORDER
                .iter()
                .map(|k| Some(*k))
                .chain([None])
                .filter_map(|k| rows.iter().find(|r| r.category == k).map(|r| (k, r.ms)))
                .collect();
            c.total = c.parts.iter().map(|(_, ms)| ms).sum();
            c
        })
        .collect();
    trim(cols, unit)
}

/// `appTimeColumns`: the range's app time spread by heatmap intensity.
// shortcut: a share of Σ app time, not a per-day measurement (the Svelte
// ponytail); a per-day usage query if the capture-only bars must be exact.
pub fn app_time_columns(
    heat: &[HeatmapBucket],
    tracked: i64,
    (start, end): (i64, i64),
    unit: Unit,
    now: i64,
    off: UtcOffset,
) -> Vec<Column> {
    let total: i64 = heat.iter().map(|b| b.intensity_count.max(0)).sum();
    let cols = slots(start, end, unit, now, off)
        .into_iter()
        .map(|mut c| {
            let w: i64 = heat
                .iter()
                .filter(|b| b.bucket_start_ms >= c.start && b.bucket_start_ms < c.end)
                .map(|b| b.intensity_count.max(0))
                .sum();
            let ms = if total > 0 {
                (tracked as f64 * w as f64 / total as f64) as i64
            } else {
                0
            };
            c.app_time = true;
            c.parts = if ms > 0 { vec![(None, ms)] } else { Vec::new() };
            c.total = ms;
            c
        })
        .collect();
    trim(cols, unit)
}

// ── Focus heat ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct HeatRow {
    pub label: String,
    /// 12 two-hour slots; `Some((band, mean minutes per day))`.
    pub cells: Vec<Option<(FocusLevel, f64)>>,
}

fn focus_weight(f: FocusLevel) -> f64 {
    match f {
        FocusLevel::Deep => 1.,
        FocusLevel::Mixed => 0.5,
        FocusLevel::Distracted => 0.,
    }
}

/// `focusHeat`: rows = captured days (day / week) or ISO weeks (month).
pub fn focus_heat(
    acts: &[Activity],
    (start, end): (i64, i64),
    unit: Unit,
    off: UtcOffset,
) -> Vec<HeatRow> {
    let day_unit = if unit == Unit::Day { Unit::Week } else { unit };
    let days: Vec<(Column, Vec<&Activity>)> = slots(start, end, day_unit, 0, off)
        .into_iter()
        .map(|d| {
            let a = acts
                .iter()
                .filter(|a| a.focus.is_some() && overlap(a, d.start, d.end) > 0)
                .collect::<Vec<_>>();
            (d, a)
        })
        .filter(|(_, a)| !a.is_empty())
        .collect();
    let mut groups: Vec<(String, Vec<&(Column, Vec<&Activity>)>)> = Vec::new();
    for x in &days {
        let date = local(x.0.start, off).date();
        let key = if unit == Unit::Month {
            format!("W{}", date.iso_week())
        } else {
            weekday(x.0.start, off).to_string()
        };
        let gk = if unit == Unit::Month {
            key.clone()
        } else {
            x.0.start.to_string()
        };
        match groups.iter_mut().find(|(k, _)| *k == gk) {
            Some((_, g)) => g.push(x),
            None => groups.push((gk, vec![x])),
        }
        let _ = key;
    }
    groups
        .into_iter()
        .map(|(_, g)| {
            let first = local(g[0].0.start, off).date();
            let label = if unit == Unit::Month {
                format!("W{}", first.iso_week())
            } else {
                weekday(g[0].0.start, off).to_string()
            };
            let cells = (0..12)
                .map(|i| {
                    let (mut w, mut m) = (0., 0.);
                    for (d, da) in &g {
                        let s = d.start + i * 2 * HOUR_MS;
                        let e = s + 2 * HOUR_MS;
                        for a in da {
                            let o = overlap(a, s, e) as f64 / 60_000.;
                            w += focus_weight(a.focus.unwrap()) * o;
                            m += o;
                        }
                    }
                    (m > 0.).then(|| {
                        let mean = w / m;
                        let band = if mean >= 0.7 {
                            FocusLevel::Deep
                        } else if mean >= 0.35 {
                            FocusLevel::Mixed
                        } else {
                            FocusLevel::Distracted
                        };
                        (band, m / g.len() as f64)
                    })
                })
                .collect();
            HeatRow { label, cells }
        })
        .collect()
}

/// `listedAppRows`: no rows that would print "0m".
pub fn listed_apps(rows: &[capture_types::AppUsage]) -> Vec<capture_types::AppUsage> {
    rows.iter()
        .filter(|r| (r.active_ms as f64 / 60_000.).round() > 0.)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, offset};

    fn act(
        id: i64,
        s: i64,
        e: i64,
        c: Option<ActivityCategory>,
        f: Option<FocusLevel>,
    ) -> Activity {
        Activity {
            id,
            title: format!("a{id}"),
            summary: String::new(),
            category: c,
            focus: f,
            started_at_ms: s,
            ended_at_ms: e,
            created_at_ms: e,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn durations_match_the_svelte_voice() {
        assert_eq!(fmt_duration(369 * 60_000), "6h 09m");
        assert_eq!(fmt_duration(47 * 60_000), "47m");
        assert_eq!(humanize_ms(45_000), "45s");
        assert_eq!(humanize_ms(125 * 60_000), "2h 5m");
        assert_eq!(humanize_ms(120 * 60_000), "2h");
        assert_eq!(fmt_axis_tick(90. * 60_000.), "1.5h");
        assert_eq!(fmt_axis_tick(120. * 60_000.), "2h");
        assert_eq!(axis_max(5. * 60_000.), 20. * 60_000.);
        assert_eq!(axis_max(300. * 60_000.), 480. * 60_000.);
    }

    #[test]
    fn week_columns_stack_categories_in_order() {
        let off = offset!(+5:30);
        let (s, e) = window(date!(2026 - 11 - 04), Unit::Week, off);
        assert_eq!(local(s, off).date(), date!(2026 - 11 - 02));
        let tue = s + DAY_MS + 9 * HOUR_MS;
        let acts = [
            act(
                1,
                tue,
                tue + HOUR_MS,
                Some(ActivityCategory::Research),
                None,
            ),
            act(
                2,
                tue + HOUR_MS,
                tue + 3 * HOUR_MS,
                Some(ActivityCategory::Creating),
                None,
            ),
        ];
        let cols = activity_columns(&acts, (s, e), Unit::Week, e, off);
        assert_eq!(cols.len(), 7);
        assert_eq!(cols[1].label, "Tue");
        assert_eq!(
            cols[1].parts,
            vec![
                (Some(ActivityCategory::Creating), 2 * HOUR_MS),
                (Some(ActivityCategory::Research), HOUR_MS)
            ]
        );
        let top = top_category(&acts, s, e).unwrap();
        assert_eq!(
            (top.category, top.pct),
            (Some(ActivityCategory::Creating), 67)
        );
    }

    #[test]
    fn focus_heat_bands_by_weighted_mean() {
        let off = offset!(+5:30);
        let (s, e) = window(date!(2026 - 11 - 04), Unit::Week, off);
        let mon8 = s + 8 * HOUR_MS;
        let acts = [act(1, mon8, mon8 + HOUR_MS, None, Some(FocusLevel::Deep))];
        let rows = focus_heat(&acts, (s, e), Unit::Week, off);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Mon");
        assert_eq!(rows[0].cells[4], Some((FocusLevel::Deep, 60.)));
        assert_eq!(clock_range(mon8, mon8 + HOUR_MS, off, "–"), "8:00–9:00 AM");
    }
}

/// `toLocaleString()` for a count: "12,408".
pub fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}
