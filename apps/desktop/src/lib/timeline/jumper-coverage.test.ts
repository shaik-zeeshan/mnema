// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  buildDayRows,
  coverageCells,
  dayHasCoverage,
  formatCapturedHours,
  hasCoverageInHours,
  indexCoverage,
} from "./jumper-coverage";

const coverage = indexCoverage([
  { day: "2026-08-03", coveredMs: 24_120_000, hours: [9, 10, 11, 14] },
  { day: "2026-08-01", coveredMs: 4_140_000, hours: [1, 2] },
]);

describe("formatCapturedHours", () => {
  it("reads hours and minutes, or 'nothing' when the day is empty", () => {
    expect(formatCapturedHours(0)).toBe("nothing");
    expect(formatCapturedHours(59_000)).toBe("nothing");
    expect(formatCapturedHours(47 * 60_000)).toBe("47m");
    expect(formatCapturedHours(6 * 3_600_000 + 42 * 60_000)).toBe("6h 42m");
    expect(formatCapturedHours(2 * 3_600_000 + 5 * 60_000)).toBe("2h 05m");
  });
});

describe("coverageCells", () => {
  it("lights the 3-hour cell each captured hour falls in", () => {
    expect(coverageCells([])).toEqual(new Array(8).fill(false));
    // 9,10,11 -> cell 3; 14 -> cell 4.
    expect(coverageCells([9, 10, 11, 14])).toEqual([
      false, false, false, true, true, false, false, false,
    ]);
    expect(coverageCells([0, 23])).toEqual([
      true, false, false, false, false, false, false, true,
    ]);
  });
});

describe("buildDayRows", () => {
  const rows = buildDayRows(coverage, new Date(2026, 7, 3, 15, 0, 0));

  it("returns seven days newest-first with relative names for the first two", () => {
    expect(rows).toHaveLength(7);
    expect(rows[0].key).toBe("2026-08-03");
    expect(rows[0].label).toBe("Today");
    expect(rows[1].label).toBe("Yesterday");
    expect(rows[2].label).toContain("Aug 1");
    expect(rows[6].key).toBe("2026-07-28");
  });

  it("disables days with no recording and reads their hours as 'nothing'", () => {
    // Aug 2 has no coverage row at all.
    expect(rows[1].disabled).toBe(true);
    expect(rows[1].hoursLabel).toBe("nothing");
    expect(rows[0].disabled).toBe(false);
    expect(rows[0].hoursLabel).toBe("6h 42m");
    expect(rows[2].disabled).toBe(false);
  });

  it("crosses a month boundary without gaps", () => {
    expect(rows.map((r) => r.key)).toEqual([
      "2026-08-03",
      "2026-08-02",
      "2026-08-01",
      "2026-07-31",
      "2026-07-30",
      "2026-07-29",
      "2026-07-28",
    ]);
  });
});

describe("month-grid predicates", () => {
  it("enables only days that hold capture", () => {
    expect(dayHasCoverage(coverage, { year: 2026, month: 8, day: 3 })).toBe(true);
    expect(dayHasCoverage(coverage, { year: 2026, month: 8, day: 2 })).toBe(false);
    // Zero-padding: month 8 day 1 must match the "2026-08-01" key.
    expect(dayHasCoverage(coverage, { year: 2026, month: 8, day: 1 })).toBe(true);
  });

  it("answers 'is there anything this morning'", () => {
    const today = { year: 2026, month: 8, day: 3 };
    expect(hasCoverageInHours(coverage, today, 0, 11)).toBe(true);
    expect(hasCoverageInHours(coverage, today, 15, 23)).toBe(false);
    expect(
      hasCoverageInHours(coverage, { year: 2026, month: 8, day: 2 }, 0, 11),
    ).toBe(false);
  });
});
