// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { computeTipPosition } from "./tooltip";

// GAP=6, MARGIN=4 in tooltip.ts.
describe("computeTipPosition", () => {
  it("sits above the trigger when there's room", () => {
    const { top } = computeTipPosition(
      { top: 100, bottom: 120, left: 200, width: 40 },
      80,
      20,
      1000,
    );
    expect(top).toBe(100 - 20 - 6); // above: top - tipH - GAP
  });

  it("flips below when there's no room above", () => {
    const { top } = computeTipPosition(
      { top: 2, bottom: 22, left: 200, width: 40 },
      80,
      20,
      1000,
    );
    expect(top).toBe(22 + 6); // below: bottom + GAP
  });

  it("centers horizontally over the trigger", () => {
    const { left } = computeTipPosition(
      { top: 100, bottom: 120, left: 200, width: 40 },
      80,
      20,
      1000,
    );
    expect(left).toBe(200 + 20 - 40); // triggerCenter(220) - tipW/2(40)
  });

  it("clamps to the left edge", () => {
    const { left } = computeTipPosition(
      { top: 100, bottom: 120, left: 0, width: 10 },
      80,
      20,
      1000,
    );
    expect(left).toBe(4); // MARGIN, not negative
  });

  it("clamps to the right edge", () => {
    const { left } = computeTipPosition(
      { top: 100, bottom: 120, left: 990, width: 10 },
      80,
      20,
      1000,
    );
    expect(left).toBe(1000 - 80 - 4); // viewportW - tipW - MARGIN
  });
});
