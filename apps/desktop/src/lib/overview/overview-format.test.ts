// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  busiestDayPhrase,
  dayKeyOf,
  dayWindow,
  formatClock,
  formatHeroHours,
  formatLongHours,
  formatSpan,
  shiftDayKey,
  weekBars,
  weekTotalMs,
} from "./overview-format";

describe("day windows", () => {
  it("is half-open and exactly one local day wide", () => {
    const { startMs, endMs } = dayWindow("2026-08-03");
    expect(new Date(startMs).getDate()).toBe(3);
    expect(new Date(endMs).getDate()).toBe(4);
    expect(new Date(endMs).getHours()).toBe(0);
  });

  it("steps across a month boundary", () => {
    expect(shiftDayKey("2026-08-01", -1)).toBe("2026-07-31");
    expect(shiftDayKey("2026-12-31", 1)).toBe("2027-01-01");
  });

  it("round-trips a Date", () => {
    expect(dayKeyOf(new Date(2026, 7, 3))).toBe("2026-08-03");
  });
});

describe("honest durations (G8)", () => {
  it("renders no hero number below a minute of capture", () => {
    expect(formatHeroHours(null)).toBeNull();
    expect(formatHeroHours(0)).toBeNull();
    expect(formatHeroHours(30_000)).toBeNull();
  });

  it("reads hours:minutes", () => {
    expect(formatHeroHours(6 * 3_600_000 + 42 * 60_000)).toBe("6:42");
    expect(formatHeroHours(47 * 60_000)).toBe("0:47");
  });

  it("keeps spans coarse", () => {
    expect(formatSpan(20_000)).toBe("under a minute");
    expect(formatSpan(38 * 60_000)).toBe("38 min");
    expect(formatSpan(72 * 60_000)).toBe("1h 12m");
    expect(formatLongHours(32 * 3_600_000 + 8 * 60_000)).toBe("32h 08m");
  });
});

describe("week bars", () => {
  const days = [
    { day: "2026-08-03", coveredMs: 6 * 3_600_000, hours: [9, 10] },
    { day: "2026-07-31", coveredMs: 3 * 3_600_000, hours: [14] },
  ];

  it("returns seven bars ending on the anchor, oldest first", () => {
    const bars = weekBars(days, "2026-08-03");
    expect(bars).toHaveLength(7);
    expect(bars[0].key).toBe("2026-07-28");
    expect(bars[6].key).toBe("2026-08-03");
    expect(bars[6].isAnchor).toBe(true);
  });

  it("draws a zero bar for a day with no coverage row", () => {
    const bars = weekBars(days, "2026-08-03");
    expect(bars.find((b) => b.key === "2026-08-01")?.coveredMs).toBe(0);
    expect(bars.find((b) => b.key === "2026-08-01")?.ratio).toBe(0);
  });

  it("scales against the busiest day and totals the week", () => {
    const bars = weekBars(days, "2026-08-03");
    expect(bars[6].ratio).toBe(1);
    expect(bars.find((b) => b.key === "2026-07-31")?.ratio).toBeCloseTo(0.5);
    expect(weekTotalMs(bars)).toBe(9 * 3_600_000);
  });

  it("says nothing about a busiest day when the week is empty", () => {
    expect(busiestDayPhrase(weekBars([], "2026-08-03"))).toBeNull();
    expect(busiestDayPhrase(weekBars(days, "2026-08-03"))).toBe("busiest Mon");
  });
});

describe("formatClock", () => {
  // Regression: a record missing its timestamp rendered the literal "NaN:NaN"
  // in the digest tile header. G8 says a fact that isn't there yields no
  // number at all, so the absent cases must come back null.
  it("returns null rather than a number for an absent stamp", () => {
    expect(formatClock(null)).toBeNull();
    expect(formatClock(undefined)).toBeNull();
    expect(formatClock(Number.NaN)).toBeNull();
    expect(formatClock(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("still formats a real stamp as local wall clock", () => {
    const at = new Date(2026, 7, 3, 14, 32).getTime();
    expect(formatClock(at)).toBe("14:32");
  });
});
