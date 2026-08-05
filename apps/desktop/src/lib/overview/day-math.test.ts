// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  busiestBar,
  capturedLabel,
  dayKey,
  dayWindow,
  heroHours,
  hourCells,
  indexDays,
  storageGauge,
  weekBars,
  weekTotalMs,
} from "./day-math";

const HOUR = 3_600_000;

describe("hourCells", () => {
  it("lights exactly the captured hours", () => {
    const cells = hourCells([0, 9, 23]);
    expect(cells.length).toBe(24);
    expect(cells.filter(Boolean).length).toBe(3);
    expect(cells[0] && cells[9] && cells[23]).toBe(true);
  });

  it("survives missing and out-of-range hours", () => {
    expect(hourCells(undefined).some(Boolean)).toBe(false);
    expect(hourCells([-1, 24, 99]).some(Boolean)).toBe(false);
  });
});

describe("hero + captured labels", () => {
  it("formats hours:minutes", () => {
    expect(heroHours(6 * HOUR + 42 * 60_000)).toBe("6:42");
    expect(capturedLabel(6 * HOUR + 42 * 60_000)).toBe("6h 42m");
    expect(capturedLabel(47 * 60_000)).toBe("47m");
  });

  it("renders NO number rather than a zero (G8)", () => {
    expect(heroHours(0)).toBeNull();
    expect(heroHours(undefined)).toBeNull();
    expect(capturedLabel(0)).toBeNull();
  });
});

describe("weekBars", () => {
  const today = new Date(2026, 7, 5); // Wed 5 Aug 2026, local
  const index = indexDays([
    { day: dayKey(today), coveredMs: 2 * HOUR, hours: [9, 10] },
    { day: "2026-08-03", coveredMs: 8 * HOUR, hours: [9] },
  ]);

  it("returns seven days oldest-first, ending today", () => {
    const bars = weekBars(index, today);
    expect(bars.length).toBe(7);
    expect(bars[6]!.isToday).toBe(true);
    expect(bars[6]!.key).toBe("2026-08-05");
    expect(bars[0]!.key).toBe("2026-07-30");
  });

  it("scales against the busiest day and treats absent days as real zeroes", () => {
    const bars = weekBars(index, today);
    expect(bars[6]!.fraction).toBeCloseTo(0.25);
    expect(busiestBar(bars)!.key).toBe("2026-08-03");
    expect(weekTotalMs(bars)).toBe(10 * HOUR);
    expect(bars.find((b) => b.key === "2026-08-01")!.coveredMs).toBe(0);
  });

  it("has no peak to divide by when the week is empty", () => {
    const bars = weekBars(new Map(), today);
    expect(bars.every((b) => b.fraction === 0)).toBe(true);
    expect(busiestBar(bars)).toBeNull();
  });
});

describe("storageGauge", () => {
  const facts = {
    capturePath: "/x",
    diskFreeBytes: 200_000_000_000,
    totalRamBytes: null,
    measuredBytesPerDay: 1_000_000_000,
    measuredDays: 7,
    screenFrameRate: 1,
    ocrBacklog: null,
    transcriptionBacklog: null,
    semanticVectorCount: null,
    semanticPendingCount: null,
    semanticVectorBytes: 768,
    databaseBytes: null,
  };

  it("reads a month of capture against the free space, notched at the measured week", () => {
    const gauge = storageGauge(facts)!;
    expect(gauge.windowBytes).toBe(7_000_000_000);
    expect(gauge.horizonBytes).toBe(30_000_000_000);
    expect(gauge.fillPct).toBeCloseTo(15, 1);
    expect(gauge.notchPct).toBeCloseTo(3.5, 1);
    expect(gauge.tight).toBe(false);
  });

  it("keeps a sliver visible, caps at full, and flags a tight disk", () => {
    const tight = storageGauge({ ...facts, diskFreeBytes: 5_000_000_000 })!;
    expect(tight.tight).toBe(true);
    expect(tight.fillPct).toBe(100);
    const tiny = storageGauge({ ...facts, measuredBytesPerDay: 1, measuredDays: 1 })!;
    expect(tiny.fillPct).toBe(1.5);
  });

  it("renders nothing without a measured day or a free-space reading (G8)", () => {
    expect(storageGauge(null)).toBeNull();
    expect(storageGauge({ ...facts, measuredBytesPerDay: null })).toBeNull();
    expect(storageGauge({ ...facts, diskFreeBytes: null })).toBeNull();
    expect(storageGauge({ ...facts, diskFreeBytes: 0 })).toBeNull();
  });
});

describe("dayWindow", () => {
  it("is local midnight to midnight", () => {
    const { startMs, endMs } = dayWindow(new Date(2026, 7, 5, 14, 30));
    expect(new Date(startMs).getHours()).toBe(0);
    expect(endMs - startMs).toBe(24 * HOUR);
  });
});
