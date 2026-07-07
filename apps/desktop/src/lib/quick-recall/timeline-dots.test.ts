// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  computeTimelineDots,
  timelineAxisStartMs,
  timelineDayIndex,
  timelineDayLabels,
  timelineLegend,
  TIMELINE_DAY_SPAN,
  TIMELINE_MIN_GAP_PC,
} from "./timeline-dots";

const DAY_MS = 86_400_000;
// A fixed "now": Tue Jul 7 2026, 15:00 local.
const NOW = new Date(2026, 6, 7, 15, 0, 0);
const AXIS = timelineAxisStartMs(NOW);

describe("timelineAxisStartMs", () => {
  test("is local midnight seven days back", () => {
    const start = new Date(AXIS);
    expect(start.getFullYear()).toBe(2026);
    expect(start.getMonth()).toBe(5); // June
    expect(start.getDate()).toBe(30);
    expect(start.getHours()).toBe(0);
    expect(start.getMinutes()).toBe(0);
  });
});

describe("timelineDayLabels", () => {
  test("eight labels, oldest first, human last two", () => {
    const labels = timelineDayLabels(NOW);
    expect(labels).toHaveLength(TIMELINE_DAY_SPAN);
    expect(labels[6]).toBe("Yesterday");
    expect(labels[7]).toBe("Today");
    // Earlier days are short dates ("Jun 30" in en-US; assert the day number
    // so the test survives locale-dependent month formatting).
    expect(labels[0]).toContain("30");
    expect(labels[5]).toContain("5");
  });
});

describe("timelineDayIndex", () => {
  test("maps a time inside a slot to that day", () => {
    expect(timelineDayIndex(AXIS, AXIS)).toBe(0);
    expect(timelineDayIndex(AXIS + 3 * DAY_MS + 1, AXIS)).toBe(3);
    expect(timelineDayIndex(NOW.getTime(), AXIS)).toBe(7);
  });

  test("clamps outside the axis to the edges", () => {
    expect(timelineDayIndex(AXIS - 20 * DAY_MS, AXIS)).toBe(0);
    expect(timelineDayIndex(AXIS + 30 * DAY_MS, AXIS)).toBe(7);
  });
});

describe("computeTimelineDots", () => {
  test("honest time mapping: day+hour → percent of the 8-day span", () => {
    // Mockup formula: ((day + hour/24) / 8) * 100.
    const noonDay7 = AXIS + 7 * DAY_MS + 12 * 3_600_000;
    const dots = computeTimelineDots([{ key: "a", timeMs: noonDay7 }], AXIS);
    expect(dots[0].pc).toBeCloseTo(((7 + 12 / 24) / 8) * 100, 6);
  });

  test("returns dots sorted ascending regardless of input order", () => {
    const dots = computeTimelineDots(
      [
        { key: "late", timeMs: AXIS + 7 * DAY_MS },
        { key: "early", timeMs: AXIS + 1 * DAY_MS },
        { key: "mid", timeMs: AXIS + 4 * DAY_MS },
      ],
      AXIS,
    );
    expect(dots.map((d) => d.key)).toEqual(["early", "mid", "late"]);
    expect(dots[0].pc).toBeLessThan(dots[1].pc);
    expect(dots[1].pc).toBeLessThan(dots[2].pc);
  });

  test("clamps results older than the axis to the left edge", () => {
    const dots = computeTimelineDots(
      [{ key: "ancient", timeMs: AXIS - 30 * DAY_MS }],
      AXIS,
    );
    expect(dots[0].pc).toBe(0);
  });

  test("min-gap pass spreads a same-time cluster by 1.1% steps", () => {
    const t = AXIS + 2 * DAY_MS;
    const dots = computeTimelineDots(
      [
        { key: "a", timeMs: t },
        { key: "b", timeMs: t },
        { key: "c", timeMs: t },
      ],
      AXIS,
    );
    const base = ((2 * DAY_MS) / (8 * DAY_MS)) * 100;
    expect(dots[0].pc).toBeCloseTo(base, 6);
    expect(dots[1].pc).toBeCloseTo(base + TIMELINE_MIN_GAP_PC, 6);
    expect(dots[2].pc).toBeCloseTo(base + 2 * TIMELINE_MIN_GAP_PC, 6);
  });

  test("min-gap leaves already-spread dots at their honest positions", () => {
    const dots = computeTimelineDots(
      [
        { key: "a", timeMs: AXIS + 1 * DAY_MS },
        { key: "b", timeMs: AXIS + 5 * DAY_MS },
      ],
      AXIS,
    );
    expect(dots[0].pc).toBeCloseTo(12.5, 6);
    expect(dots[1].pc).toBeCloseTo(62.5, 6);
  });

  test("a right-edge cluster stays inside 100% and keeps the gap", () => {
    const now = NOW.getTime();
    const sources = Array.from({ length: 10 }, (_, i) => ({
      key: `k${i}`,
      timeMs: now - i * 60_000, // ten results within the last ten minutes
    }));
    const dots = computeTimelineDots(sources, AXIS);
    expect(dots[dots.length - 1].pc).toBeLessThanOrEqual(100);
    for (let k = 1; k < dots.length; k++) {
      expect(dots[k].pc - dots[k - 1].pc).toBeGreaterThanOrEqual(
        TIMELINE_MIN_GAP_PC - 1e-9,
      );
    }
  });

  test("empty input yields no dots", () => {
    expect(computeTimelineDots([], AXIS)).toEqual([]);
  });
});

describe("timelineLegend", () => {
  test("no chips → no legend", () => {
    expect(timelineLegend(17, [])).toBeNull();
  });

  test("shown count + chip scope", () => {
    expect(timelineLegend(4, ["app: Chrome"])).toBe("4 shown · app: Chrome");
    expect(timelineLegend(9, ["app: Chrome", "source: Microphone"])).toBe(
      "9 shown · app: Chrome · source: Microphone",
    );
  });
});
