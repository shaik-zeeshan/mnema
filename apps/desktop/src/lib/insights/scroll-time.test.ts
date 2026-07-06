// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-view.test.ts).
import { describe, expect, it } from "bun:test";
import { scrollFraction, dragToScrollTop, rowAtViewportY } from "./scroll-time";

describe("scrollFraction", () => {
  it("clamps to [0, 1]", () => {
    expect(scrollFraction(-10, 1000, 500)).toBe(0);
    expect(scrollFraction(250, 1000, 500)).toBe(0.5);
    expect(scrollFraction(9999, 1000, 500)).toBe(1);
  });
  it("is 0 when the content doesn't scroll", () => {
    expect(scrollFraction(0, 500, 500)).toBe(0);
    expect(scrollFraction(0, 300, 500)).toBe(0);
  });
});

describe("dragToScrollTop", () => {
  // track 100..300, content 1000, viewport 500 → maxScroll 500
  it("maps the track ends to 0 and maxScroll, midpoint to the middle", () => {
    expect(dragToScrollTop(100, 100, 200, 1000, 500)).toBe(0);
    expect(dragToScrollTop(200, 100, 200, 1000, 500)).toBe(250);
    expect(dragToScrollTop(300, 100, 200, 1000, 500)).toBe(500);
  });
  it("clamps pointer positions past either track end", () => {
    expect(dragToScrollTop(-50, 100, 200, 1000, 500)).toBe(0);
    expect(dragToScrollTop(9999, 100, 200, 1000, 500)).toBe(500);
  });
  it("is 0 when the content doesn't scroll or the track is degenerate", () => {
    expect(dragToScrollTop(200, 100, 200, 400, 500)).toBe(0);
    expect(dragToScrollTop(200, 100, 0, 1000, 500)).toBe(0);
  });
});

describe("rowAtViewportY", () => {
  const rows = [
    { atMs: 1, top: 0, bottom: 40 },
    { atMs: 2, top: 40, bottom: 80 },
    { atMs: 3, top: 80, bottom: 120 },
  ];
  it("picks the first row whose bottom is past line y", () => {
    expect(rowAtViewportY(rows, 50)).toBe(2);
  });
  it("picks the first row when y is above all rows", () => {
    expect(rowAtViewportY(rows, -100)).toBe(1);
  });
  it("treats a row ending exactly at line y as scrolled past", () => {
    expect(rowAtViewportY(rows, 40)).toBe(2);
  });
  it("returns null when y is past all rows or with no rows", () => {
    expect(rowAtViewportY(rows, 120)).toBe(null);
    expect(rowAtViewportY([], 0)).toBe(null);
  });
});
