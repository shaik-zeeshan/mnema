// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { biggestGap, coverageReadout, relativeTime } from "./day-read";

const clock = (ms: number) => new Date(ms).toISOString().slice(11, 16);
const gap = (startMs: number, endMs: number) => ({ startMs, endMs });

describe("coverageReadout", () => {
  it("renders nothing when no hour holds frames (G8: no zero stand-in)", () => {
    expect(coverageReadout(0, [], clock)).toBeNull();
  });

  it("states hours alone when the day has no away-gap", () => {
    expect(coverageReadout(1, [], clock)).toBe("1 hour hold frames");
    expect(coverageReadout(6, [], clock)).toBe("6 hours hold frames");
  });

  it("names the single gap at its start", () => {
    expect(coverageReadout(6, [gap(3_600_000, 5_400_000)], clock)).toBe(
      "6 hours hold frames · one away-gap at 01:00",
    );
  });

  it("counts many gaps and points at the LONGEST, not the first", () => {
    const gaps = [gap(0, 600_000), gap(7_200_000, 14_400_000), gap(20_000_000, 20_600_000)];
    expect(coverageReadout(6, gaps, clock)).toBe(
      "6 hours hold frames · 3 away-gaps, the longest at 02:00",
    );
    expect(biggestGap(gaps)).toEqual(gaps[1]);
    expect(biggestGap([])).toBeNull();
  });
});

describe("relativeTime", () => {
  const now = 10 * 86_400_000;
  it("rounds coarsely and never prints a stamp it doesn't have", () => {
    expect(relativeTime(0, now)).toBe("");
    expect(relativeTime(now - 30_000, now)).toBe("just now");
    expect(relativeTime(now - 120_000, now)).toBe("2 min ago");
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe("3h ago");
    expect(relativeTime(now - 2 * 86_400_000, now)).toBe("2d ago");
  });
});
