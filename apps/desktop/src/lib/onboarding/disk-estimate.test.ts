// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  ANCHOR_INTERVAL_S,
  ANCHOR_MB_PER_DAY,
  ANCHOR_VIDEO_MB,
  ANCHOR_VIDEO_PIXELS,
  draftVideoPixels,
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

  it("scales only the video share with resolution", () => {
    // 1080p is 2.25× the anchor's 720p pixels; the OCR/audio shares don't move.
    const at1080 = estimateDailyStorageMb(ANCHOR_INTERVAL_S, 1920 * 1080);
    expect(at1080).toBeCloseTo(
      ANCHOR_MB_PER_DAY - ANCHOR_VIDEO_MB + ANCHOR_VIDEO_MB * 2.25,
      1,
    );
    // Omitted pixels price the anchor (= default 720p) resolution.
    expect(estimateDailyStorageMb(ANCHOR_INTERVAL_S, ANCHOR_VIDEO_PIXELS)).toBe(
      estimateDailyStorageMb(ANCHOR_INTERVAL_S),
    );
  });
});

describe("draftVideoPixels", () => {
  const draft = (over = {}) => ({
    resolutionMode: "preset",
    resolutionPreset: "720p",
    customWidth: null,
    customHeight: null,
    ...over,
  });

  it("maps presets to their pixel counts", () => {
    expect(draftVideoPixels(draft(), null)).toBe(1280 * 720);
    expect(draftVideoPixels(draft({ resolutionPreset: "1080p" }), null)).toBe(1920 * 1080);
    expect(draftVideoPixels(draft({ resolutionPreset: "540p" }), null)).toBe(960 * 540);
  });

  it("prices original at the display's backing pixels, 1080p when unknown", () => {
    expect(draftVideoPixels(draft({ resolutionMode: "original" }), 3024 * 1964)).toBe(3024 * 1964);
    expect(draftVideoPixels(draft({ resolutionMode: "original" }), null)).toBe(1920 * 1080);
  });

  it("prices a custom size at width × height, falling back while mid-edit", () => {
    expect(
      draftVideoPixels(draft({ resolutionMode: "custom", customWidth: 1600, customHeight: 900 }), null),
    ).toBe(1600 * 900);
    expect(draftVideoPixels(draft({ resolutionMode: "custom" }), null)).toBe(1920 * 1080);
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
