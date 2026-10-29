// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { windowFor, shiftAnchor, humanizeMs, historyDensity, historyWindow, pickerDensity } from "./activity-helpers";

// A duration should read in the coarsest non-zero unit: sub-minute activities as
// seconds (never a rounded "0m"), minutes, then hours with a minute remainder.
describe("humanizeMs adaptive units (seconds / minutes / hours)", () => {
  it("shows seconds under a minute instead of rounding to 0m", () => {
    expect(humanizeMs(9_000)).toBe("9s");
    expect(humanizeMs(45_000)).toBe("45s");
    expect(humanizeMs(59_000)).toBe("59s");
  });
  it("shows minutes from a minute up to an hour", () => {
    expect(humanizeMs(60_000)).toBe("1m");
    expect(humanizeMs(90_000)).toBe("2m"); // rounds to nearest minute
    expect(humanizeMs(45 * 60_000)).toBe("45m");
  });
  it("shows hours with a minute remainder past an hour", () => {
    expect(humanizeMs(60 * 60_000)).toBe("1h");
    expect(humanizeMs(65 * 60_000)).toBe("1h 5m");
    expect(humanizeMs(150 * 60_000)).toBe("2h 30m");
  });
  it("guards zero / invalid input", () => {
    expect(humanizeMs(0)).toBe("0s");
    expect(humanizeMs(-5)).toBe("0s");
    expect(humanizeMs(Number.NaN)).toBe("0s");
  });
});

// The Overview range stepper steps `anchor` by one window unit and derives the
// previous window for the period-over-period delta. Stepping a MONTH must always
// land in the adjacent calendar month — never the same month — even when the
// anchor's day-of-month (29/30/31) doesn't exist in the target month.
describe("shiftAnchor month stepping (calendar-correct, no day overflow)", () => {
  // Anchor input at NOON local so a DST transition can't nudge the date across a
  // day boundary and make the assertion flaky.
  const noon = (y: number, m0: number, day: number) =>
    new Date(y, m0, day, 12, 0, 0, 0).getTime();
  // `windowFor(_, "month").startMs` is the local-midnight first-of-month.
  const firstOfMonth = (y: number, m0: number) => new Date(y, m0, 1).getTime();
  const monthStart = (anchorMs: number) => windowFor(anchorMs, "month").startMs;

  it("steps back from a 31-day month into the PREVIOUS month, not the same one", () => {
    // Mar 31 → the previous window must be February, not March again.
    const mar31 = noon(2026, 2, 31);
    const prev = shiftAnchor(mar31, "month", -1);
    expect(monthStart(prev)).toBe(firstOfMonth(2026, 1)); // Feb 1
    expect(monthStart(prev)).not.toBe(monthStart(mar31)); // not March
  });

  it("steps forward across a short month without skipping it", () => {
    // Jan 31 → the next window must be February, not March (no Feb-skip).
    const jan31 = noon(2026, 0, 31);
    const next = shiftAnchor(jan31, "month", 1);
    expect(monthStart(next)).toBe(firstOfMonth(2026, 1)); // Feb 1
  });

  it("handles other 31→shorter month boundaries (May→Apr)", () => {
    const may31 = noon(2026, 4, 31);
    const prev = shiftAnchor(may31, "month", -1);
    expect(monthStart(prev)).toBe(firstOfMonth(2026, 3)); // Apr 1
  });

  it("still steps correctly when no overflow is involved (Jan→Dec across year)", () => {
    const jan31 = noon(2026, 0, 31);
    const prev = shiftAnchor(jan31, "month", -1);
    expect(monthStart(prev)).toBe(firstOfMonth(2025, 11)); // Dec 1, 2025
  });
});

describe("date-picker density", () => {
  it("period_density_overrides_history_for_the_same_day", () => {
    const at = (h) => new Date(2026, 9, 9, h).getTime();
    // Three captured hours on Oct 9 → level 2, replacing history's stale 1.
    const activityHeatmap = [9, 10, 11].map((h) => ({ bucketStartMs: at(h), intensityCount: 1 }));
    const period = { rangeStartMs: at(0), rangeEndMs: new Date(2026, 9, 10).getTime(), activityHeatmap };
    expect(pickerDensity({ "2026-10-08": 3, "2026-10-09": 1 }, period)).toEqual({
      "2026-10-08": 3,
      "2026-10-09": 2,
    });
  });

  it("period_days_without_capture_read_empty_not_unknown", () => {
    const period = { rangeStartMs: new Date(2026, 0, 5).getTime(), rangeEndMs: new Date(2026, 0, 7).getTime(), activityHeatmap: [] };
    expect(pickerDensity({ "2026-01-05": -1, "2026-01-06": -1, "2026-01-07": -1 }, period)).toEqual({
      "2026-01-05": 0,
      "2026-01-06": 0,
      "2026-01-07": -1,
    });
  });

  it("history_before_the_window_reads_unknown_not_empty", () => {
    const span = { startMs: new Date(2026, 5, 11).getTime() };
    const hours = [new Date(2026, 5, 11, 9).getTime()];
    expect(historyDensity(hours, span, "2026-06-09")).toEqual({
      "2026-06-09": -1,
      "2026-06-10": -1,
      "2026-06-11": 1,
    });
  });

  it("history_window_is_clamped_to_min_day_and_120_days", () => {
    const now = new Date(2026, 9, 9, 15, 30).getTime();
    // 120 days back, from that day's local midnight — never mid-day.
    expect(historyWindow("2025-01-01", now)).toEqual({
      startMs: new Date(2026, 5, 11).getTime(),
      endMs: now,
    });
    // A later first capture wins, also from its local midnight.
    expect(historyWindow("2026-09-01", now).startMs).toBe(new Date(2026, 8, 1).getTime());
  });
});

// India is UTC+5:30, so UTC hour buckets start at :30 local and the one holding
// 00:00–00:30 starts at 23:30 the day before.
describe("date-picker density under a half-hour offset", () => {
  const saved = process.env.TZ;
  beforeAll(() => {
    process.env.TZ = "Asia/Kolkata";
  });
  afterAll(() => {
    process.env.TZ = saved;
  });

  it("period_bucket_starting_before_the_period_does_not_overwrite_history", () => {
    const oct9 = new Date(2026, 9, 9).getTime();
    expect(new Date(oct9).getTimezoneOffset()).toBe(-330);
    // A frame at 00:15 on Oct 9 lands in the bucket starting 23:30 on Oct 8.
    const activityHeatmap = [{ bucketStartMs: oct9 - 30 * 60_000, intensityCount: 4 }];
    const period = { rangeStartMs: oct9, rangeEndMs: new Date(2026, 9, 10).getTime(), activityHeatmap };
    expect(pickerDensity({ "2026-10-08": 3 }, period)).toEqual({ "2026-10-08": 3, "2026-10-09": 0 });
  });

  it("history_window_first_bucket_starting_before_it_stays_unknown", () => {
    const span = { startMs: new Date(2026, 5, 11).getTime() };
    const hours = [span.startMs - 30 * 60_000, span.startMs + 30 * 60_000];
    expect(historyDensity(hours, span, "2026-06-10")).toEqual({ "2026-06-10": -1, "2026-06-11": 1 });
  });
});
