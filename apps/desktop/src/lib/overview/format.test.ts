// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";

import {
  busiestDay,
  convictionDots,
  coveredOn,
  formatClock,
  formatElapsed,
  formatHoursColon,
  formatHoursMinutes,
  formatMinutes,
  localDayKey,
  retentionDays,
  weekBars,
} from "./format";

const H = 3_600_000;

describe("overview formatters", () => {
  it("keys local days without locale reordering", () => {
    expect(localDayKey(new Date(2026, 7, 3))).toBe("2026-08-03");
    expect(localDayKey(new Date(2026, 0, 9))).toBe("2026-01-09");
  });

  it("formats the header and hero durations", () => {
    expect(formatHoursMinutes(6 * H + 42 * 60_000)).toBe("6h 42m");
    expect(formatHoursMinutes(42 * 60_000)).toBe("42m");
    expect(formatHoursColon(6 * H + 42 * 60_000)).toBe("6:42");
    expect(formatHoursColon(0)).toBe("0:00");
    expect(formatElapsed(2 * H + 14 * 60_000 + 7_000)).toBe("2:14:07");
  });

  it("rounds conversation length coarsely and never to zero", () => {
    expect(formatMinutes(38 * 60_000)).toBe("38 min");
    expect(formatMinutes(20_000)).toBe("1 min");
  });

  it("renders a local wall clock", () => {
    expect(formatClock(new Date(2026, 7, 3, 9, 14).getTime())).toBe("09:14");
  });

  it("lights at least one conviction dot for any live belief", () => {
    expect(convictionDots(0)).toBe(0);
    expect(convictionDots(0.05)).toBe(1);
    expect(convictionDots(0.8)).toBe(4);
    expect(convictionDots(1)).toBe(5);
  });

  it("builds seven days ending today, zero-filling absent days", () => {
    const now = new Date(2026, 7, 3, 14, 40); // Monday
    const bars = weekBars(
      [
        { day: "2026-08-03", coveredMs: 6 * H, hours: [] },
        { day: "2026-07-31", coveredMs: 8 * H, hours: [] },
      ],
      now,
    );
    expect(bars).toHaveLength(7);
    expect(bars[0].key).toBe("2026-07-28");
    expect(bars[6].key).toBe("2026-08-03");
    expect(bars[6].isToday).toBe(true);
    expect(bars[6].label).toBe("Mo");
    expect(bars[2].coveredMs).toBe(0);
    expect(busiestDay(bars)).toBe("Fr");
    expect(busiestDay(weekBars([], now))).toBeNull();
  });

  it("reads a day's coverage and the retention window", () => {
    expect(coveredOn([{ day: "2026-08-03", coveredMs: 5, hours: [] }], "2026-08-03")).toBe(5);
    expect(coveredOn([], "2026-08-03")).toBe(0);
    expect(retentionDays("days_30")).toBe(30);
    expect(retentionDays("never")).toBeNull();
  });
});
