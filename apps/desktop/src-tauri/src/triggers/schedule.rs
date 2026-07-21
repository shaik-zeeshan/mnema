//! Pure Schedule-condition evaluation for the Triggers evaluator (issue #175).
//!
//! Everything here is integer arithmetic over unix milliseconds plus the user's
//! UTC offset — no clocks, no I/O — so the fire/miss rules are fully unit-tested
//! without waiting for wall-clock time.
//!
//! The rules (docs/triggers/CONTEXT.md):
//! - A Schedule fires daily or weekly at a chosen LOCAL time.
//! - The window is the natural period: the current local day (daily) or the
//!   current local week, Monday-start (weekly).
//! - Catch-up within the period: if the scheduled moment passed while the
//!   machine was asleep/off, the occurrence still fires later that same period.
//! - Expired occurrences are quietly missed: yesterday's (or last week's)
//!   occurrence is NEVER fired late — only the current period's occurrence is
//!   ever considered.

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

/// How often a Schedule condition recurs. Weekly requires a `weekday`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleCadence {
    Daily,
    Weekly,
}

/// Day of week for a weekly Schedule, serialized as its lowercase English name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleWeekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl ScheduleWeekday {
    /// Index within the Monday-start week (Monday = 0 … Sunday = 6).
    pub fn week_index(self) -> i64 {
        match self {
            ScheduleWeekday::Monday => 0,
            ScheduleWeekday::Tuesday => 1,
            ScheduleWeekday::Wednesday => 2,
            ScheduleWeekday::Thursday => 3,
            ScheduleWeekday::Friday => 4,
            ScheduleWeekday::Saturday => 5,
            ScheduleWeekday::Sunday => 6,
        }
    }
}

/// Parse a `"HH:MM"` local time-of-day into minutes since local midnight, or
/// `None` when malformed/out of range.
pub fn parse_time_minutes(time: &str) -> Option<i64> {
    let (hours, minutes) = time.trim().split_once(':')?;
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) {
        return None;
    }
    Some(hours * 60 + minutes)
}

/// The current period's scheduled occurrence that is DUE and UNFIRED, as a UTC
/// unix-ms instant — or `None` when there is nothing to fire right now.
///
/// - `time_minutes`: minutes since local midnight (from [`parse_time_minutes`]).
/// - `weekday`: required for `Weekly` (a weekly schedule without one never
///   fires); ignored for `Daily`.
/// - `offset_minutes`: minutes to ADD to UTC to reach the user's local wall
///   clock (the same convention as Ask AI's temporal grounding).
/// - `last_fired_ms`: the trigger's persisted last-fired instant (UTC ms).
///
/// Fires exactly when the occurrence's UTC instant is at-or-before `now_ms`
/// (due — including hours later after a missed-time wake, as long as the local
/// period has not rolled over) AND `last_fired_ms` predates it (unfired). A
/// previous period's missed occurrence is structurally unreachable: only the
/// CURRENT local day/week's occurrence is ever computed.
pub fn due_occurrence_ms(
    cadence: ScheduleCadence,
    time_minutes: i64,
    weekday: Option<ScheduleWeekday>,
    now_ms: i64,
    offset_minutes: i32,
    last_fired_ms: Option<i64>,
) -> Option<i64> {
    let offset_ms = i64::from(offset_minutes) * MINUTE_MS;
    // The user's local wall clock as "ms since epoch, local calendar".
    let local_now_ms = now_ms + offset_ms;
    // Whole local days since epoch (Euclidean so pre-1970 / negative offsets
    // near midnight still floor toward the correct local day).
    let local_day = local_now_ms.div_euclid(DAY_MS);

    let occurrence_local_ms = match cadence {
        ScheduleCadence::Daily => local_day * DAY_MS + time_minutes * MINUTE_MS,
        ScheduleCadence::Weekly => {
            let weekday = weekday?;
            // 1970-01-01 was a Thursday: index 3 in the Monday-start week.
            let day_of_week = (local_day + 3).rem_euclid(7);
            let week_start_day = local_day - day_of_week;
            (week_start_day + weekday.week_index()) * DAY_MS + time_minutes * MINUTE_MS
        }
    };
    let occurrence_utc_ms = occurrence_local_ms - offset_ms;

    // Not due yet this period (scheduled later today / later this week).
    if occurrence_utc_ms > now_ms {
        return None;
    }
    // Already fired at-or-after this occurrence.
    if last_fired_ms.is_some_and(|fired| fired >= occurrence_utc_ms) {
        return None;
    }
    Some(occurrence_utc_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-07-20 is a Monday. UTC midnight of that day in unix ms.
    const MON_2026_07_20_UTC: i64 = 1_784_505_600_000;
    const IST: i32 = 330; // UTC+05:30
    const PST: i32 = -480; // UTC-08:00

    fn at(day_start: i64, hours: i64, minutes: i64) -> i64 {
        day_start + (hours * 60 + minutes) * MINUTE_MS
    }

    #[test]
    fn parse_time_minutes_accepts_hh_mm_and_rejects_junk() {
        assert_eq!(parse_time_minutes("18:30"), Some(18 * 60 + 30));
        assert_eq!(parse_time_minutes("00:00"), Some(0));
        assert_eq!(parse_time_minutes("23:59"), Some(23 * 60 + 59));
        assert_eq!(parse_time_minutes(" 9:05 "), Some(9 * 60 + 5));
        for junk in ["24:00", "12:60", "-1:00", "noon", "12", "12:", ":30", ""] {
            assert_eq!(parse_time_minutes(junk), None, "{junk:?} must not parse");
        }
    }

    // ── Daily ────────────────────────────────────────────────────────────────

    #[test]
    fn daily_does_not_fire_before_its_local_time() {
        // Schedule 18:30 UTC-local; now is 18:29 UTC.
        let now = at(MON_2026_07_20_UTC, 18, 29);
        assert_eq!(
            due_occurrence_ms(ScheduleCadence::Daily, 18 * 60 + 30, None, now, 0, None),
            None
        );
    }

    #[test]
    fn daily_fires_at_its_local_time_and_reports_the_occurrence_instant() {
        let occurrence = at(MON_2026_07_20_UTC, 18, 30);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                18 * 60 + 30,
                None,
                occurrence,
                0,
                None
            ),
            Some(occurrence)
        );
    }

    #[test]
    fn daily_catches_up_after_a_missed_time_wake_within_the_same_day() {
        // The machine slept through 18:30; it wakes at 22:47 the same local day.
        let occurrence = at(MON_2026_07_20_UTC, 18, 30);
        let wake = at(MON_2026_07_20_UTC, 22, 47);
        assert_eq!(
            due_occurrence_ms(ScheduleCadence::Daily, 18 * 60 + 30, None, wake, 0, None),
            Some(occurrence)
        );
        // …including when the last fire was YESTERDAY's occurrence.
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                18 * 60 + 30,
                None,
                wake,
                0,
                Some(occurrence - DAY_MS)
            ),
            Some(occurrence)
        );
    }

    #[test]
    fn daily_never_double_fires_within_a_day() {
        let occurrence = at(MON_2026_07_20_UTC, 18, 30);
        let later = at(MON_2026_07_20_UTC, 23, 0);
        // Fired at (or after) the occurrence → nothing more today.
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                18 * 60 + 30,
                None,
                later,
                0,
                Some(occurrence)
            ),
            None
        );
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                18 * 60 + 30,
                None,
                later,
                0,
                Some(occurrence + 5 * MINUTE_MS)
            ),
            None
        );
    }

    #[test]
    fn daily_quietly_misses_an_expired_occurrence_from_a_previous_day() {
        // The machine was off across yesterday's 18:30 and boots today at 09:00:
        // today's occurrence (18:30) is not due yet, and yesterday's is never
        // considered → nothing fires.
        let tuesday_9am = at(MON_2026_07_20_UTC + DAY_MS, 9, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                18 * 60 + 30,
                None,
                tuesday_9am,
                0,
                None
            ),
            None
        );
    }

    #[test]
    fn daily_respects_the_local_offset_period_boundary() {
        // Schedule 23:30 IST. At 2026-07-20 18:30 UTC the IST wall clock reads
        // 2026-07-21 00:00 — a NEW local day whose 23:30 has not come yet.
        let now = at(MON_2026_07_20_UTC, 18, 30);
        assert_eq!(
            due_occurrence_ms(ScheduleCadence::Daily, 23 * 60 + 30, None, now, IST, None),
            None
        );
        // But the just-ended IST day's 23:30 (18:00 UTC) fired if unclaimed…
        let just_before_midnight = at(MON_2026_07_20_UTC, 18, 20);
        let occurrence_utc = at(MON_2026_07_20_UTC, 18, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                23 * 60 + 30,
                None,
                just_before_midnight,
                IST,
                None
            ),
            Some(occurrence_utc)
        );

        // Negative offset: 07:00 PST on Monday = 15:00 UTC.
        let now = at(MON_2026_07_20_UTC, 15, 0);
        assert_eq!(
            due_occurrence_ms(ScheduleCadence::Daily, 7 * 60, None, now, PST, None),
            Some(now)
        );
        // One minute earlier (UTC) it is 06:59 PST → not due.
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Daily,
                7 * 60,
                None,
                now - MINUTE_MS,
                PST,
                None
            ),
            None
        );
    }

    // ── Weekly ───────────────────────────────────────────────────────────────

    #[test]
    fn weekly_fires_on_its_weekday_and_catches_up_later_in_the_same_week() {
        // Friday 09:00 schedule; 2026-07-24 is the Friday of this week.
        let friday_9 = at(MON_2026_07_20_UTC + 4 * DAY_MS, 9, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Weekly,
                9 * 60,
                Some(ScheduleWeekday::Friday),
                friday_9,
                0,
                None
            ),
            Some(friday_9)
        );
        // Missed Friday morning, woke Sunday evening — still the same week.
        let sunday_20 = at(MON_2026_07_20_UTC + 6 * DAY_MS, 20, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Weekly,
                9 * 60,
                Some(ScheduleWeekday::Friday),
                sunday_20,
                0,
                None
            ),
            Some(friday_9)
        );
    }

    #[test]
    fn weekly_not_due_before_its_weekday_and_expired_after_the_week_rolls() {
        // Wednesday: this week's Friday occurrence is still ahead → None.
        let wednesday = at(MON_2026_07_20_UTC + 2 * DAY_MS, 12, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Weekly,
                9 * 60,
                Some(ScheduleWeekday::Friday),
                wednesday,
                0,
                None
            ),
            None
        );
        // Next Monday: last week's missed Friday is expired (new week's Friday
        // is ahead) → quietly missed.
        let next_monday = at(MON_2026_07_20_UTC + 7 * DAY_MS, 12, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Weekly,
                9 * 60,
                Some(ScheduleWeekday::Friday),
                next_monday,
                0,
                None
            ),
            None
        );
    }

    #[test]
    fn weekly_never_double_fires_within_a_week_and_needs_a_weekday() {
        let friday_9 = at(MON_2026_07_20_UTC + 4 * DAY_MS, 9, 0);
        let saturday = at(MON_2026_07_20_UTC + 5 * DAY_MS, 10, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Weekly,
                9 * 60,
                Some(ScheduleWeekday::Friday),
                saturday,
                0,
                Some(friday_9)
            ),
            None
        );
        // A weekly schedule without a weekday can never fire.
        assert_eq!(
            due_occurrence_ms(ScheduleCadence::Weekly, 9 * 60, None, saturday, 0, None),
            None
        );
    }

    #[test]
    fn weekly_sunday_belongs_to_the_monday_start_week() {
        // Sunday 2026-07-26 18:00 with a Sunday 17:00 schedule fires that day.
        let sunday_18 = at(MON_2026_07_20_UTC + 6 * DAY_MS, 18, 0);
        let sunday_17 = at(MON_2026_07_20_UTC + 6 * DAY_MS, 17, 0);
        assert_eq!(
            due_occurrence_ms(
                ScheduleCadence::Weekly,
                17 * 60,
                Some(ScheduleWeekday::Sunday),
                sunday_18,
                0,
                None
            ),
            Some(sunday_17)
        );
    }
}
