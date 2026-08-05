// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  formatSpan,
  frameIntervalMs,
  slotWidthForZoom,
  visibleSpanMs,
} from "./zoom";

describe("frameIntervalMs", () => {
  it("falls back to the 0.5 fps default for missing or bogus rates", () => {
    expect(frameIntervalMs(0.5)).toBe(2_000);
    expect(frameIntervalMs(1 / 60)).toBe(60_000);
    expect(frameIntervalMs(undefined)).toBe(2_000);
    expect(frameIntervalMs(0)).toBe(2_000);
    expect(frameIntervalMs(-3)).toBe(2_000);
  });
});

describe("slotWidthForZoom", () => {
  it("keeps the shipped 8 px density at Day and stays monotonic", () => {
    const wide = 1_200;
    const hour = slotWidthForZoom("hour", wide, 2_000);
    const day = slotWidthForZoom("day", wide, 2_000);
    const week = slotWidthForZoom("week", wide, 2_000);
    expect(day).toBe(8);
    // Wider slot = less time visible, so Hour > Day > Week in px per frame.
    expect(hour).toBeGreaterThan(day);
    expect(day).toBeGreaterThan(week);
  });

  it("honours the requested span once the cadence is slow enough to allow it", () => {
    // 1 frame/minute: an hour is 60 frames, which fits 1200 px at 20 px each.
    expect(slotWidthForZoom("hour", 1_200, 60_000)).toBe(20);
  });

  it("never exceeds the max slot width", () => {
    expect(slotWidthForZoom("hour", 4_000, 3_600_000)).toBe(48);
  });
});

describe("visibleSpanMs", () => {
  const stamps = [0, 1, 2, 3, 4, 5, 6].map((i) => 1_000_000 + i * 2_000);

  it("measures the real elapsed time across the visible window", () => {
    // Active in the middle, 5 slots visible -> indices 1..5 -> 4 gaps of 2 s.
    expect(visibleSpanMs(stamps, 3, 5)).toBe(8_000);
  });

  it("clamps at the ends of the loaded list", () => {
    expect(visibleSpanMs(stamps, 0, 5)).toBe(4_000);
    expect(visibleSpanMs(stamps, 6, 5)).toBe(4_000);
  });

  it("returns null when there is nothing to measure", () => {
    expect(visibleSpanMs([], 0, 10)).toBeNull();
    expect(visibleSpanMs([1_000], 0, 10)).toBeNull();
    expect(visibleSpanMs(stamps, 3, 1)).toBeNull();
  });

  it("reports the real gap, not the assumed cadence", () => {
    const overnight = [0, 2_000, 40_000_000, 40_002_000];
    expect(visibleSpanMs(overnight, 1, 4)).toBe(40_002_000);
  });
});

describe("formatSpan", () => {
  it("reads coarsely", () => {
    expect(formatSpan(0)).toBe("0s");
    expect(formatSpan(42_000)).toBe("42s");
    expect(formatSpan(18 * 60_000)).toBe("18m");
    expect(formatSpan(4 * 3_600_000 + 12 * 60_000)).toBe("4h 12m");
  });
});
