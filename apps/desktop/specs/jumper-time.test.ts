import { describe, expect, test } from "bun:test";
import {
  buildHourBuckets,
  dayRange,
  formatHourLabel,
  hourRange,
} from "../src/lib/timeline/jumper-time";
import type { FrameSummaryDto } from "../src/lib/types/app-infra";

// Pure time-list helpers extracted from `TimelineJumper.svelte`. `now` is
// injected so "today caps at the current hour" is deterministic, and ranges are
// asserted via local-Date getters so the suite holds in any TZ.

function localFrame(id: number, y: number, m: number, d: number, h: number): FrameSummaryDto {
  return { id, capturedAt: new Date(y, m - 1, d, h, 0, 0, 0).toISOString() };
}

describe("formatHourLabel", () => {
  test("maps the 24h clock onto a 12h AM/PM label (noon/midnight = 12)", () => {
    expect(formatHourLabel(0)).toBe("12:00 AM");
    expect(formatHourLabel(1)).toBe("1:00 AM");
    expect(formatHourLabel(11)).toBe("11:00 AM");
    expect(formatHourLabel(12)).toBe("12:00 PM");
    expect(formatHourLabel(13)).toBe("1:00 PM");
    expect(formatHourLabel(23)).toBe("11:00 PM");
  });
});

describe("buildHourBuckets", () => {
  const now = new Date(2026, 5, 15, 14, 30, 0, 0); // local 2026-06-15 14:30

  test("today caps the buckets at the current local hour (inclusive)", () => {
    const buckets = buildHourBuckets({ year: 2026, month: 6, day: 15 }, now, false, undefined);
    expect(buckets.length).toBe(15); // hours 0..14
    expect(buckets[0].hour).toBe(0);
    expect(buckets[14].hour).toBe(14);
  });

  test("a non-today date renders all 24 hours", () => {
    const buckets = buildHourBuckets({ year: 2026, month: 6, day: 10 }, now, true, []);
    expect(buckets.length).toBe(24);
    expect(buckets[23].hour).toBe(23);
  });

  test("pre-load (monthLoaded=false): nothing disabled, counts all zero", () => {
    const buckets = buildHourBuckets({ year: 2026, month: 6, day: 10 }, now, false, undefined);
    expect(buckets.every((b) => b.disabled === false)).toBe(true);
    expect(buckets.every((b) => b.count === 0)).toBe(true);
  });

  test("loaded month: an hour is disabled iff it has zero frames; counts tally", () => {
    const summaries = [
      localFrame(1, 2026, 6, 10, 9),
      localFrame(2, 2026, 6, 10, 9),
      localFrame(3, 2026, 6, 10, 18),
    ];
    const buckets = buildHourBuckets({ year: 2026, month: 6, day: 10 }, now, true, summaries);
    const byHour = new Map(buckets.map((b) => [b.hour, b]));
    expect(byHour.get(9)?.count).toBe(2);
    expect(byHour.get(9)?.disabled).toBe(false);
    expect(byHour.get(18)?.count).toBe(1);
    expect(byHour.get(10)?.count).toBe(0);
    expect(byHour.get(10)?.disabled).toBe(true); // loaded + empty → disabled
  });

  test("unparseable timestamps are skipped, not counted", () => {
    const summaries: FrameSummaryDto[] = [
      { id: 1, capturedAt: "not-a-date" },
      localFrame(2, 2026, 6, 10, 9),
    ];
    const buckets = buildHourBuckets({ year: 2026, month: 6, day: 10 }, now, true, summaries);
    const total = buckets.reduce((sum, b) => sum + b.count, 0);
    expect(total).toBe(1);
  });

  test("labels track the hour through the AM/PM boundary", () => {
    const buckets = buildHourBuckets({ year: 2026, month: 6, day: 10 }, now, false, undefined);
    expect(buckets[0].label).toBe("12:00 AM");
    expect(buckets[12].label).toBe("12:00 PM");
  });
});

describe("dayRange / hourRange", () => {
  test("dayRange spans the full local day [00:00:00.000 .. 23:59:59.999]", () => {
    const { start, end } = dayRange({ year: 2026, month: 6, day: 3 });
    expect([start.getFullYear(), start.getMonth(), start.getDate()]).toEqual([2026, 5, 3]);
    expect([start.getHours(), start.getMinutes(), start.getSeconds(), start.getMilliseconds()]).toEqual([0, 0, 0, 0]);
    expect([end.getFullYear(), end.getMonth(), end.getDate()]).toEqual([2026, 5, 3]);
    expect([end.getHours(), end.getMinutes(), end.getSeconds(), end.getMilliseconds()]).toEqual([23, 59, 59, 999]);
  });

  test("hourRange runs day-start through the last millisecond of the picked hour", () => {
    const { start, end } = hourRange({ year: 2026, month: 6, day: 3 }, 9);
    expect([start.getHours(), start.getMinutes(), start.getSeconds(), start.getMilliseconds()]).toEqual([0, 0, 0, 0]);
    expect([end.getHours(), end.getMinutes(), end.getSeconds(), end.getMilliseconds()]).toEqual([9, 59, 59, 999]);
    expect([end.getFullYear(), end.getMonth(), end.getDate()]).toEqual([2026, 5, 3]);
  });

  test("hour 0 yields [00:00:00.000 .. 00:59:59.999]", () => {
    const { end } = hourRange({ year: 2026, month: 6, day: 3 }, 0);
    expect([end.getHours(), end.getMinutes(), end.getSeconds(), end.getMilliseconds()]).toEqual([0, 59, 59, 999]);
  });
});
