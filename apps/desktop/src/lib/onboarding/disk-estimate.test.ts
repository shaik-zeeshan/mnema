// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  ANCHOR_INTERVAL_S,
  ANCHOR_MB_PER_DAY,
  estimateDailyStorageMb,
  estimateWindowStorageMb,
} from "./disk-estimate";
import {
  CAPTURE_INTERVAL_LADDER_S,
  DEFAULT_CAPTURE_INTERVAL_S,
} from "../components/capture-rate";

describe("estimateDailyStorageMb", () => {
  it("returns the measured anchor at the interval it was measured at", () => {
    expect(estimateDailyStorageMb(ANCHOR_INTERVAL_S)).toBe(ANCHOR_MB_PER_DAY);
  });

  it("returns ~400 MB/day at the 2s default", () => {
    const mb = estimateDailyStorageMb(DEFAULT_CAPTURE_INTERVAL_S);
    expect(mb).toBe(405);
    expect(Math.abs(mb - 400)).toBeLessThan(10);
  });

  it("scales linearly with frame rate across the whole ladder", () => {
    for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
      const expected = (ANCHOR_MB_PER_DAY * ANCHOR_INTERVAL_S) / intervalS;
      expect(estimateDailyStorageMb(intervalS)).toBeCloseTo(expected, 1);
    }
  });

  it("halving the interval doubles the daily cost", () => {
    for (const intervalS of [0.5, 1, 2, 3, 5, 15, 30]) {
      expect(estimateDailyStorageMb(intervalS / 2)).toBeCloseTo(
        estimateDailyStorageMb(intervalS) * 2,
        1,
      );
    }
  });

  it("is monotonically decreasing as the interval grows", () => {
    let previous = Number.POSITIVE_INFINITY;
    for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
      const mb = estimateDailyStorageMb(intervalS);
      expect(mb).toBeLessThan(previous);
      previous = mb;
    }
  });

  it("falls back to the slider default for a non-positive interval", () => {
    const fallback = estimateDailyStorageMb(DEFAULT_CAPTURE_INTERVAL_S);
    expect(estimateDailyStorageMb(0)).toBe(fallback);
    expect(estimateDailyStorageMb(-1)).toBe(fallback);
  });
});

describe("estimateWindowStorageMb", () => {
  it("is the daily figure held for the retention window", () => {
    // The anchor itself was measured over a complete 14-day window.
    expect(estimateWindowStorageMb(ANCHOR_INTERVAL_S, 14)).toBe(
      ANCHOR_MB_PER_DAY * 14,
    );
    expect(estimateWindowStorageMb(DEFAULT_CAPTURE_INTERVAL_S, 14)).toBe(405 * 14);
  });

  it("is zero for a zero- or negative-length window", () => {
    expect(estimateWindowStorageMb(2, 0)).toBe(0);
    expect(estimateWindowStorageMb(2, -3)).toBe(0);
  });
});
