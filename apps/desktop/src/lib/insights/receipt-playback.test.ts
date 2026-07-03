// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  LruCache,
  clampIndex,
  stepIndex,
  desiredWindow,
  initialPosterIndex,
  countCaptureSegments,
} from "./receipt-playback";

describe("LruCache eviction order", () => {
  it("evicts the least-recently-used key past capacity", () => {
    const c = new LruCache<string>(2);
    c.set(1, "a");
    c.set(2, "b");
    c.set(3, "c"); // capacity 2 → key 1 (LRU) evicted
    expect(c.has(1)).toBe(false);
    expect(c.keys()).toEqual([2, 3]);
  });

  it("get() touches a key so it survives the next eviction", () => {
    const c = new LruCache<string>(2);
    c.set(1, "a");
    c.set(2, "b");
    c.get(1); // 1 becomes most-recent → 2 is now the LRU
    c.set(3, "c");
    expect(c.has(2)).toBe(false);
    expect(c.keys()).toEqual([1, 3]);
  });

  it("peek() does NOT reorder", () => {
    const c = new LruCache<string>(2);
    c.set(1, "a");
    c.set(2, "b");
    c.peek(1); // no touch → 1 stays LRU
    c.set(3, "c");
    expect(c.has(1)).toBe(false);
    expect(c.keys()).toEqual([2, 3]);
  });

  it("re-setting an existing key refreshes its recency without growing size", () => {
    const c = new LruCache<string>(2);
    c.set(1, "a");
    c.set(2, "b");
    c.set(1, "a2"); // 1 refreshed → 2 is LRU
    c.set(3, "c");
    expect(c.size).toBe(2);
    expect(c.has(2)).toBe(false);
    expect(c.peek(1)).toBe("a2");
  });
});

describe("index stepping bounds", () => {
  it("clamps into [0, count-1]", () => {
    expect(clampIndex(-3, 5)).toBe(0);
    expect(clampIndex(9, 5)).toBe(4);
    expect(clampIndex(2, 5)).toBe(2);
  });
  it("clamps to 0 when the strip is empty", () => {
    expect(clampIndex(4, 0)).toBe(0);
  });
  it("steps and clamps at both ends", () => {
    expect(stepIndex(0, -1, 5)).toBe(0); // can't go below 0
    expect(stepIndex(4, 1, 5)).toBe(4); // can't exceed last
    expect(stepIndex(2, 1, 5)).toBe(3);
  });
});

describe("initialPosterIndex", () => {
  it("uses the headline frame when present in the strip", () => {
    expect(initialPosterIndex([10, 20, 30, 40], 30)).toBe(2);
  });
  it("falls back to the middle when no headline / not found", () => {
    expect(initialPosterIndex([10, 20, 30, 40, 50], null)).toBe(2);
    expect(initialPosterIndex([10, 20, 30], 999)).toBe(1);
  });
  it("is 0 for an empty strip", () => {
    expect(initialPosterIndex([], 5)).toBe(0);
  });
});

describe("desiredWindow", () => {
  it("orders current first, then interleaves ahead and behind", () => {
    // index 2, lookahead 2, behind 1 → current, +1, -1, +2
    expect(desiredWindow([10, 20, 30, 40, 50], 2, 2, 1)).toEqual([30, 40, 20, 50]);
  });
  it("clips at both strip edges", () => {
    expect(desiredWindow([10, 20, 30], 0, 2, 1)).toEqual([10, 20, 30]);
    expect(desiredWindow([10, 20, 30], 2, 2, 1)).toEqual([30, 20]);
  });
  it("is empty for an empty strip", () => {
    expect(desiredWindow([], 0, 2, 1)).toEqual([]);
  });
});

describe("countCaptureSegments", () => {
  it("counts one segment for a dense run", () => {
    expect(countCaptureSegments([0, 1000, 2000, 3000])).toBe(1);
  });
  it("splits on a gap larger than the threshold", () => {
    // two dense runs separated by a 5-minute gap
    expect(countCaptureSegments([0, 1000, 2000, 300_000, 301_000])).toBe(2);
  });
  it("is 0 for no frames", () => {
    expect(countCaptureSegments([])).toBe(0);
  });
});
