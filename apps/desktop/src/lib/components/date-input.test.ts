// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  densityFromHeatmap,
  isoWeek,
  parseIso,
  periodOf,
  relativeLabel,
  stepSpan,
  switchUnit,
  toIso,
} from "./date-input";

const today = parseIso("2026-10-09"); // a Friday
const min = parseIso("2026-06-01");
const isos = (span) => span && span.map(toIso);

describe("period stepping", () => {
  it("steps exactly one period and never past the current one", () => {
    const [a, b] = periodOf(today, "week");
    expect(isos(stepSpan(a, b, "week", 1, min, today))).toBeNull();
    expect(isos(stepSpan(a, b, "week", -1, min, today))).toEqual(["2026-09-28", "2026-10-04"]);
    const [m0, m1] = periodOf(today, "month");
    expect(stepSpan(m0, m1, "month", 1, min, today)).toBeNull();
    expect(isos(stepSpan(m0, m1, "month", -1, min, today))).toEqual(["2026-09-01", "2026-09-30"]);
    expect(stepSpan(today, today, "day", 1, min, today)).toBeNull();
  });

  it("refuses to step wholly before min", () => {
    const [a, b] = periodOf(min, "month");
    expect(stepSpan(a, b, "month", -1, min, today)).toBeNull();
    expect(stepSpan(min, min, "day", -1, min, today)).toBeNull();
  });

  it("steps month ends without overflowing (Mar 31 → Feb)", () => {
    const [a, b] = periodOf(parseIso("2026-03-31"), "month");
    expect(isos(stepSpan(a, b, "month", -1, parseIso("2025-01-01"), today))).toEqual(["2026-02-01", "2026-02-28"]);
  });
});

describe("isoWeek", () => {
  it("numbers ISO-8601 weeks, including year edges", () => {
    expect(isoWeek(parseIso("2026-10-09"))).toBe(41);
    expect(isoWeek(parseIso("2026-10-05"))).toBe(41);
    expect(isoWeek(parseIso("2021-01-01"))).toBe(53); // belongs to 2020's last week
    expect(isoWeek(parseIso("2024-12-30"))).toBe(1); // belongs to 2025's first week
  });
});

describe("unit switch", () => {
  it("lands on now when the current period was showing", () => {
    const [a] = periodOf(today, "week");
    expect(isos(switchUnit(a, "week", "day", today, min))).toEqual(["2026-10-09", "2026-10-09"]);
    expect(isos(switchUnit(a, "week", "month", today, min))).toEqual(["2026-10-01", "2026-10-31"]);
  });

  it("keeps a past period's start otherwise", () => {
    const [a] = periodOf(parseIso("2026-09-16"), "week");
    expect(isos(switchUnit(a, "week", "month", today, min))).toEqual(["2026-09-01", "2026-09-30"]);
    expect(isos(switchUnit(a, "week", "day", today, min))).toEqual(["2026-09-14", "2026-09-14"]);
  });

  it("relative line counts whole periods", () => {
    const [a, b] = periodOf(parseIso("2026-07-20"), "month");
    expect(relativeLabel("period", "month", a, b, today)).toBe("3 months ago");
    const [w0, w1] = periodOf(parseIso("2026-10-01"), "week");
    expect(relativeLabel("period", "week", w0, w1, today)).toBe("last week");
  });
});

describe("densityFromHeatmap", () => {
  it("levels a local day by its captured hours", () => {
    const h = (iso, hour, n = 3) => ({ bucketStartMs: new Date(`${iso}T${String(hour).padStart(2, "0")}:00:00`).getTime(), intensityCount: n });
    const d = densityFromHeatmap([
      h("2026-10-07", 9),
      ...[9, 10, 11].map((x) => h("2026-10-08", x)),
      ...[8, 9, 10, 11, 14, 15].map((x) => h("2026-10-09", x)),
      h("2026-10-06", 9, 0),
    ]);
    expect(d).toEqual({ "2026-10-07": 1, "2026-10-08": 2, "2026-10-09": 3 });
  });
});
