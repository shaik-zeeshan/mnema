// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { matchTurnIndex, segmentDurationMs } from "./detail-view";

describe("matchTurnIndex", () => {
  const turns = [
    { startMs: 0, endMs: 5000 },
    { startMs: 5000, endMs: 12_000 },
    { startMs: 15_000, endMs: 20_000 },
  ];

  test("returns the containing turn", () => {
    expect(matchTurnIndex(turns, 6000)).toBe(1);
  });

  test("a span on a shared boundary belongs to the turn that starts there", () => {
    expect(matchTurnIndex(turns, 5000)).toBe(1);
  });

  test("falls back to the nearest turn start when no turn contains the span", () => {
    expect(matchTurnIndex(turns, 13_900)).toBe(2);
    expect(matchTurnIndex(turns, 12_100)).toBe(1);
  });

  test("empty list yields -1", () => {
    expect(matchTurnIndex([], 1000)).toBe(-1);
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
