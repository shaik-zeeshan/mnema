// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-view.test.ts).
import { describe, expect, it } from "bun:test";
import { placeSpeakerActionsPopover } from "./speaker-popover-position";

describe("placeSpeakerActionsPopover", () => {
  it("anchors at the chip's left on a wide window (no clamp)", () => {
    // width = min(42*16, 2000-64) = 672; right limit = 2000-672-12 = 1316
    const pos = placeSpeakerActionsPopover(
      { left: 500, top: 700 },
      2000,
      900,
      16,
    );
    expect(pos.left).toBe(500);
    expect(pos.bottom).toBe(900 - 700 + 6);
  });

  it("clamps a left-edge chip to the 12px floor", () => {
    const pos = placeSpeakerActionsPopover({ left: 3, top: 700 }, 2000, 900, 16);
    expect(pos.left).toBe(12);
  });

  it("clamps a right-edge chip so the popover stays fully on-screen", () => {
    const pos = placeSpeakerActionsPopover(
      { left: 1900, top: 700 },
      2000,
      900,
      16,
    );
    // left + width + 12 must not exceed the viewport
    expect(pos.left).toBe(2000 - 672 - 12);
    expect(pos.left + 672 + 12).toBeLessThanOrEqual(2000);
  });

  it("uses the narrow-window width branch when 42rem does not fit", () => {
    // innerWidth 600 < 42*16+64 = 736, so width = 600-64 = 536
    const pos = placeSpeakerActionsPopover(
      { left: 300, top: 400 },
      600,
      500,
      16,
    );
    expect(pos.left).toBe(600 - 536 - 12); // 52 — clamped, still ≥ 12
    expect(pos.left).toBeGreaterThanOrEqual(12);
  });

  it("scales the width mirror with a non-default root font size", () => {
    // rem 20 → width = min(840, 2000-64) = 840; right limit = 2000-840-12
    const pos = placeSpeakerActionsPopover(
      { left: 1500, top: 700 },
      2000,
      900,
      20,
    );
    expect(pos.left).toBe(2000 - 840 - 12);
  });

  it("keeps the popover on-screen at every chip position on a narrow window", () => {
    const innerWidth = 400; // width = 336
    for (const left of [0, 12, 50, 200, 380, 400]) {
      const pos = placeSpeakerActionsPopover(
        { left, top: 300 },
        innerWidth,
        500,
        16,
      );
      expect(pos.left).toBeGreaterThanOrEqual(12);
      expect(pos.left + 336 + 12).toBeLessThanOrEqual(innerWidth);
    }
  });
});
