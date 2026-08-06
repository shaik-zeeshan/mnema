// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-view.test.ts).
import { describe, expect, it } from "bun:test";
import { areaPaths, spanLabel, sparkPoints, sparkY } from "./subjects-format";

describe("spanLabel", () => {
  it("rounds coarsely and drops sub-hour spans", () => {
    expect(spanLabel(0)).toBeNull();
    expect(spanLabel(59 * 60_000)).toBe("over 1 hour"); // 59 min rounds to 1h
    expect(spanLabel(20 * 60_000)).toBeNull();
    expect(spanLabel(6 * 3_600_000)).toBe("over 6 hours");
    expect(spanLabel(21 * 24 * 3_600_000)).toBe("over 21 days");
  });
});

describe("sparkY", () => {
  it("puts the 0.15 display floor where the mockup draws it", () => {
    // Hero box is 260x52; the mockup's dashed floor line sits at y = 42.8.
    expect(sparkY(0.15, 52)).toBeCloseTo(42.8, 1);
    // 0.78 is the mockup's top row figure; its last point sits at y = 12.6.
    expect(sparkY(0.78, 52)).toBeCloseTo(12.6, 1);
  });
});

describe("sparkPoints", () => {
  it("spreads points across the full width by INDEX, not time", () => {
    const p = sparkPoints([0.5, 0.5, 0.5], 100, 20).split(" ");
    expect(p.map((s) => s.split(",")[0])).toEqual(["0.0", "50.0", "100.0"]);
  });
  it("flattens a single snapshot into a drawable line", () => {
    expect(sparkPoints([0.4], 100, 20).split(" ").length).toBe(2);
  });
  it("draws nothing for no history", () => {
    expect(sparkPoints([], 100, 20)).toBe("");
  });
});

describe("areaPaths", () => {
  it("refuses to draw a trajectory from one point", () => {
    expect(areaPaths([0.4], 300, 74)).toBeNull();
  });
  it("closes the fill down to the baseline", () => {
    const a = areaPaths([0.2, 0.8], 300, 74);
    expect(a.line.startsWith("M0.0,")).toBe(true);
    expect(a.fill.endsWith("L300,74L0,74Z")).toBe(true);
  });
});
