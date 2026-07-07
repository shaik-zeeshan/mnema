// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  WAVE_BAR_COUNT,
  detailCacheKey,
  matchFraction,
  matchTurnIndex,
  segmentDurationMs,
  waveBars,
} from "./detail-view";

const frameSel = (representativeId, thumbnailId) => ({
  kind: "frame",
  frame: {
    representativeFrame: { id: representativeId },
    thumbnailFrameId: thumbnailId,
  },
});

const audioSel = (segmentId, spanStartMs) => ({
  kind: "audio",
  audio: { audioSegment: { id: segmentId }, spanStartMs },
});

describe("detailCacheKey", () => {
  test("frame keys on representative AND thumbnail frame", () => {
    expect(detailCacheKey(frameSel(10, 12))).toBe("frame:10:12");
    // Equivalent-reuse: same representative, different thumbnail → distinct.
    expect(detailCacheKey(frameSel(10, 13))).not.toBe(
      detailCacheKey(frameSel(10, 12)),
    );
  });

  test("audio keys on segment + span start (two matches in one segment differ)", () => {
    expect(detailCacheKey(audioSel(7, 4200))).toBe("audio:7:4200");
    expect(detailCacheKey(audioSel(7, 9000))).not.toBe(
      detailCacheKey(audioSel(7, 4200)),
    );
  });

  test("frame and audio keys never collide", () => {
    expect(detailCacheKey(frameSel(7, 4200))).not.toBe(
      detailCacheKey(audioSel(7, 4200)),
    );
  });
});

describe("matchFraction", () => {
  test("fraction of the segment, clamped to [0, 1]", () => {
    expect(matchFraction(30_000, 120_000)).toBe(0.25);
    expect(matchFraction(200_000, 120_000)).toBe(1);
    expect(matchFraction(-5, 120_000)).toBe(0);
  });

  test("zero / invalid duration yields 0, never NaN", () => {
    expect(matchFraction(30_000, 0)).toBe(0);
    expect(matchFraction(30_000, NaN)).toBe(0);
  });
});

describe("matchTurnIndex", () => {
  const turns = [
    { startMs: 0, endMs: 5000 },
    { startMs: 5000, endMs: 12_000 },
    { startMs: 15_000, endMs: 20_000 },
  ];

  test("returns the containing turn", () => {
    expect(matchTurnIndex(turns, 6000)).toBe(1);
  });

  test("falls back to the nearest turn start when no turn contains the span", () => {
    expect(matchTurnIndex(turns, 13_900)).toBe(2);
    expect(matchTurnIndex(turns, 12_100)).toBe(1);
  });

  test("empty list yields -1", () => {
    expect(matchTurnIndex([], 1000)).toBe(-1);
  });
});

describe("waveBars", () => {
  test("renders 64 bars with the on-cluster at the real match position", () => {
    const bars = waveBars("group-key", 0.5);
    expect(bars.length).toBe(WAVE_BAR_COUNT);
    const at = Math.round(0.5 * (WAVE_BAR_COUNT - 1));
    for (const [i, bar] of bars.entries()) {
      expect(bar.on).toBe(Math.abs(i - at) <= 2);
    }
  });

  test("is deterministic for the same key", () => {
    expect(waveBars("k", 0)).toEqual(waveBars("k", 0));
  });
});

describe("segmentDurationMs", () => {
  test("difference of the segment bounds, floor 0, tolerant of bad input", () => {
    expect(
      segmentDurationMs({
        startedAt: "2026-07-07 10:00:00",
        endedAt: "2026-07-07 10:02:30",
      }),
    ).toBe(150_000);
    expect(
      segmentDurationMs({ startedAt: "garbage", endedAt: "2026-07-07 10:02:30" }),
    ).toBe(0);
  });
});
