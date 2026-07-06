//! Pure **Recurrence Digest** builder (derivation-INPUT only).
//!
//! Renders a deterministic, token-frugal text block that counts recurring
//! routines across the user's stored [`Activity`]s over a trailing window. It is
//! a pure function of its inputs — no DB, no clock, no ai-runtime — so the
//! desktop slice can feed it straight into a derivation prompt. Every timestamp
//! is shifted to the user's local wall clock (`local_offset_ms`) before
//! bucketing; the caller prepends the "Times are local, UTC+HH:MM" line.

use std::collections::BTreeMap;

use capture_types::{Activity, ActivityCategory, FocusLevel};
use time::OffsetDateTime;

/// Digest window length — a tuning constant (starting point, same stance as the
/// subject-candidate knobs in ADR 0042).
pub const RECURRENCE_DIGEST_WINDOW_DAYS: i64 = 21;

/// Total char budget for the rendered digest block (KNOWN_SUBJECTS-cap pattern).
const RECURRENCE_DIGEST_CHAR_CAP: usize = 4_000;

const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// Window start for a digest ending at `now_ms`.
pub fn recurrence_digest_window_start_ms(now_ms: i64) -> i64 {
    now_ms - RECURRENCE_DIGEST_WINDOW_DAYS * MS_PER_DAY
}

fn category_label(c: ActivityCategory) -> &'static str {
    match c {
        ActivityCategory::Creating => "creating",
        ActivityCategory::Communication => "communication",
        ActivityCategory::Meetings => "meetings",
        ActivityCategory::Research => "research",
        ActivityCategory::Learning => "learning",
        ActivityCategory::Organizing => "organizing",
        ActivityCategory::Personal => "personal",
        ActivityCategory::Entertainment => "entertainment",
    }
}

fn focus_label(f: FocusLevel) -> &'static str {
    match f {
        FocusLevel::Deep => "deep",
        FocusLevel::Mixed => "mixed",
        FocusLevel::Distracted => "distracted",
    }
}

/// Local wall-clock view of a UTC-ms instant, shifted by `local_offset_ms`.
fn local_dt(ms: i64, local_offset_ms: i64) -> OffsetDateTime {
    let nanos = (ms + local_offset_ms) as i128 * 1_000_000;
    // Treat the shifted instant as UTC so its hour/date read as local wall clock.
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Render the Recurrence Digest block from the in-window Activities.
///
/// `local_offset_ms` shifts every timestamp to the user's local wall clock
/// before bucketing (0 = UTC fallback when the frontend never stamped an
/// offset). Returns "" when `activities` is empty. The returned block is
/// char-capped at [`RECURRENCE_DIGEST_CHAR_CAP`].
pub fn build_recurrence_digest(
    activities: &[Activity],
    local_offset_ms: i64,
    now_ms: i64,
) -> String {
    if activities.is_empty() {
        return String::new();
    }
    let window_start = recurrence_digest_window_start_ms(now_ms);
    let in_window: Vec<&Activity> = activities
        .iter()
        .filter(|a| a.started_at_ms >= window_start && a.started_at_ms <= now_ms)
        .collect();
    if in_window.is_empty() {
        return String::new();
    }

    // hour -> category -> count
    let mut hour_cat: BTreeMap<u8, BTreeMap<&'static str, u32>> = BTreeMap::new();
    // hour -> focus -> count
    let mut hour_focus: BTreeMap<u8, BTreeMap<&'static str, u32>> = BTreeMap::new();
    // local day (unix-day number) -> earliest activity for that day
    let mut first_of_day: BTreeMap<i64, &Activity> = BTreeMap::new();

    for a in &in_window {
        let dt = local_dt(a.started_at_ms, local_offset_ms);
        let hour = dt.hour();
        if let Some(cat) = a.category {
            *hour_cat.entry(hour).or_default().entry(category_label(cat)).or_default() += 1;
        }
        if let Some(f) = a.focus {
            *hour_focus.entry(hour).or_default().entry(focus_label(f)).or_default() += 1;
        }
        let day = (a.started_at_ms + local_offset_ms).div_euclid(MS_PER_DAY);
        first_of_day
            .entry(day)
            .and_modify(|cur| {
                if a.started_at_ms < cur.started_at_ms {
                    *cur = a;
                }
            })
            .or_insert(a);
    }

    let header = format!(
        "RECURRENCE DIGEST (last {} days, patterns in the user's stored activity):",
        RECURRENCE_DIGEST_WINDOW_DAYS
    );

    // Section 1: per-hour category counts.
    let mut sec_cat = String::from("By hour-of-day (category counts):\n");
    for (hour, cats) in &hour_cat {
        let body: Vec<String> = cats.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        sec_cat.push_str(&format!("{:02}h {}\n", hour, body.join(" ")));
    }

    // Section 2: per-hour focus counts.
    let mut sec_focus = String::from("By hour-of-day (focus counts):\n");
    for (hour, foci) in &hour_focus {
        let body: Vec<String> = foci.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        sec_focus.push_str(&format!("{:02}h {}\n", hour, body.join(" ")));
    }

    // Section 3: first activity of each local day.
    let mut sec_first = String::from("First activity each day (local time, category, title):\n");
    for a in first_of_day.values() {
        let dt = local_dt(a.started_at_ms, local_offset_ms);
        let cat = a.category.map(category_label).unwrap_or("uncategorized");
        sec_first.push_str(&format!(
            "{:04}-{:02}-{:02} {:02}:{:02} {} {}\n",
            dt.year(),
            dt.month() as u8,
            dt.day(),
            dt.hour(),
            dt.minute(),
            cat,
            a.title.trim(),
        ));
    }

    // Assemble, then enforce the cap by dropping sections tail-first (first-of-day
    // lines are the longest, so they go first).
    let mut block = format!("{header}\n{sec_cat}\n{sec_focus}\n{sec_first}");
    if block.chars().count() > RECURRENCE_DIGEST_CHAR_CAP {
        block = format!("{header}\n{sec_cat}\n{sec_focus}");
    }
    if block.chars().count() > RECURRENCE_DIGEST_CHAR_CAP {
        block = format!("{header}\n{sec_cat}");
    }
    if block.chars().count() > RECURRENCE_DIGEST_CHAR_CAP {
        // Last resort: hard-truncate on a char boundary.
        block = block.chars().take(RECURRENCE_DIGEST_CHAR_CAP).collect();
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::{Activity, ActivityCategory, FocusLevel};

    const IST_OFFSET_MS: i64 = 19_800_000; // +05:30
    const PST_OFFSET_MS: i64 = -28_800_000; // -08:00

    fn activity(id: i64, started_at_ms: i64, cat: Option<ActivityCategory>, focus: Option<FocusLevel>, title: &str) -> Activity {
        Activity {
            id,
            title: title.to_string(),
            summary: String::new(),
            category: cat,
            focus,
            started_at_ms,
            ended_at_ms: started_at_ms + 60_000,
            created_at_ms: started_at_ms,
            evidence: Vec::new(),
        }
    }

    // 2026-07-01T00:00:00Z as unix ms.
    fn base() -> i64 {
        OffsetDateTime::parse("2026-07-01T00:00:00Z", &time::format_description::well_known::Rfc3339)
            .unwrap()
            .unix_timestamp()
            * 1000
    }

    #[test]
    fn empty_activities_yields_empty_string() {
        assert_eq!(build_recurrence_digest(&[], IST_OFFSET_MS, base()), "");
    }

    #[test]
    fn hour_bucketing_crosses_local_midnight_with_ist_offset() {
        // 22:00 UTC -> 03:30 IST next day. Hour bucket must be 03, not 22.
        let ts = base() + 22 * 3600 * 1000;
        let acts = vec![activity(1, ts, Some(ActivityCategory::Creating), None, "x")];
        let out = build_recurrence_digest(&acts, IST_OFFSET_MS, ts + 1000);
        assert!(out.contains("03h creating:1"), "block was:\n{out}");
        assert!(!out.contains("22h"), "should not bucket at UTC hour: {out}");
        // And at UTC (offset 0) it lands at 22h.
        let out_utc = build_recurrence_digest(&acts, 0, ts + 1000);
        assert!(out_utc.contains("22h creating:1"), "block was:\n{out_utc}");
    }

    #[test]
    fn negative_offset_buckets_hour_and_day_into_previous_local_day() {
        // 02:00 UTC -> 18:00 PST the PREVIOUS local day. Hour bucket must be 18,
        // and the day key (div_euclid) must land on 2026-06-30, not 2026-07-01 —
        // a regression to plain `/` division yields day 0 and passes the
        // positive-offset tests.
        let ts = base() + 2 * 3600 * 1000;
        let acts = vec![activity(1, ts, Some(ActivityCategory::Creating), None, "predawn")];
        let out = build_recurrence_digest(&acts, PST_OFFSET_MS, ts + 1000);
        assert!(out.contains("18h creating:1"), "block was:\n{out}");
        assert!(!out.contains("02h"), "should not bucket at UTC hour:\n{out}");
        assert!(out.contains("2026-06-30 18:00"), "first-of-day must use the local day:\n{out}");

        // Half-hour negative offset (-09:30): 02:00 UTC -> 16:30 previous day.
        let out = build_recurrence_digest(&acts, -34_200_000, ts + 1000);
        assert!(out.contains("16h creating:1"), "block was:\n{out}");
        assert!(out.contains("2026-06-30 16:30"), "half-hour shift wrong:\n{out}");
    }

    #[test]
    fn first_of_day_groups_by_local_day_not_utc_day() {
        // Both activities share UTC day 2026-07-01 but fall on two different PST
        // local days -> two first-of-day lines. A regression dropping the offset
        // from the day key would merge them into one UTC day.
        let acts = vec![
            activity(1, base() + 2 * 3600 * 1000, Some(ActivityCategory::Creating), None, "late evening"),
            activity(2, base() + 15 * 3600 * 1000, Some(ActivityCategory::Research), None, "morning"),
        ];
        let out = build_recurrence_digest(&acts, PST_OFFSET_MS, base() + 16 * 3600 * 1000);
        assert!(out.contains("2026-06-30 18:00 creating late evening"), "block was:\n{out}");
        assert!(out.contains("2026-07-01 07:00 research morning"), "block was:\n{out}");
    }

    #[test]
    fn out_of_window_activities_are_filtered() {
        let now = base();
        let stale = || {
            activity(
                1,
                recurrence_digest_window_start_ms(now) - 1000,
                Some(ActivityCategory::Creating),
                None,
                "stale",
            )
        };
        // Non-empty input entirely outside the window -> empty digest (the
        // in_window.is_empty() early return, distinct from the &[] path).
        assert_eq!(build_recurrence_digest(&[stale()], 0, now), "");
        // Mixed: only the in-window activity is counted.
        let fresh = activity(2, now - 3600 * 1000, Some(ActivityCategory::Research), None, "fresh");
        let out = build_recurrence_digest(&[stale(), fresh], 0, now);
        assert!(out.contains("research:1"), "block was:\n{out}");
        assert!(!out.contains("creating"), "stale activity leaked into digest:\n{out}");
        assert!(!out.contains("stale"), "stale title leaked:\n{out}");
    }

    #[test]
    fn category_and_focus_histograms_count_correctly() {
        let h9 = base() + 9 * 3600 * 1000; // 09:00 UTC
        let acts = vec![
            activity(1, h9, Some(ActivityCategory::Creating), Some(FocusLevel::Deep), "a"),
            activity(2, h9 + 60_000, Some(ActivityCategory::Creating), Some(FocusLevel::Deep), "b"),
            activity(3, h9 + 120_000, Some(ActivityCategory::Meetings), Some(FocusLevel::Mixed), "c"),
        ];
        let out = build_recurrence_digest(&acts, 0, h9 + 200_000);
        assert!(out.contains("09h creating:2 meetings:1"), "cat line wrong:\n{out}");
        assert!(out.contains("09h deep:2 mixed:1"), "focus line wrong:\n{out}");
    }

    #[test]
    fn first_activity_of_day_picks_earliest() {
        let day = base();
        let acts = vec![
            activity(1, day + 15 * 3600 * 1000, Some(ActivityCategory::Research), None, "afternoon"),
            activity(2, day + 8 * 3600 * 1000, Some(ActivityCategory::Creating), None, "morning"),
            activity(3, day + 12 * 3600 * 1000, Some(ActivityCategory::Meetings), None, "noon"),
        ];
        let out = build_recurrence_digest(&acts, 0, day + 20 * 3600 * 1000);
        assert!(out.contains("creating morning"), "first-of-day wrong:\n{out}");
        assert!(!out.contains("afternoon"), "should only show earliest:\n{out}");
    }

    #[test]
    fn char_cap_drops_first_of_day_section_first() {
        // 21 days x one long-titled activity: sec_first alone (~21 x ~250 chars)
        // pushes the full block past the cap while the histogram sections stay
        // tiny. Rung 1 must drop the first-of-day section and keep BOTH
        // histograms. Rungs 2/3 (dropping the focus histogram, hard truncation)
        // are practically unreachable once titles are gone: hour lines are
        // bounded at 24 and category/focus labels at 8/3, so header+histograms
        // stay well under the cap for any realistic counts.
        let title = "t".repeat(220);
        let mut acts = Vec::new();
        for d in 0..RECURRENCE_DIGEST_WINDOW_DAYS {
            acts.push(activity(
                d,
                base() + d * MS_PER_DAY + 9 * 3600 * 1000,
                Some(ActivityCategory::Creating),
                Some(FocusLevel::Deep),
                &title,
            ));
        }
        let now = base() + RECURRENCE_DIGEST_WINDOW_DAYS * MS_PER_DAY;
        let out = build_recurrence_digest(&acts, 0, now);
        assert!(out.chars().count() <= RECURRENCE_DIGEST_CHAR_CAP, "cap breached: {}", out.chars().count());
        assert!(out.contains("By hour-of-day (category counts):"));
        assert!(out.contains("By hour-of-day (focus counts):"));
        assert!(!out.contains("First activity each day"), "sec_first must be dropped:\n{out}");
        assert!(!out.contains(&title), "titles must not survive the cap:\n{out}");
    }
}
