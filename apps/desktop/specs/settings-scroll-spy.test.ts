import { describe, expect, test } from "bun:test";
import {
  activeSectionAt,
  isAtScrollTarget,
  isScrolledToBottom,
} from "../src/lib/settings/scroll-spy";

describe("scroll-spy: activeSectionAt", () => {
  // Capture's four sections, tops in viewport px; the line sits at 100.
  const at = (tops: number[]) =>
    (["capture", "video", "audio", "privacy"] as const).map((id, i) => ({ id, top: tops[i] }));

  test("first section until any top crosses the line", () => {
    expect(activeSectionAt(at([180, 900, 1500, 2100]), 100, false)).toBe("capture");
  });

  test("the next section wins as soon as its top crosses — not when the previous one leaves", () => {
    // The old observer kept the previous (taller, still-visible) section lit here.
    expect(activeSectionAt(at([-700, 99, 700, 1300]), 100, false)).toBe("video");
    expect(activeSectionAt(at([-700, 100, 700, 1300]), 100, false)).toBe("video");
    expect(activeSectionAt(at([-700, 101, 700, 1300]), 100, false)).toBe("capture");
  });

  test("the last section wins at the bottom even if its top can't reach the line", () => {
    expect(activeSectionAt(at([-1900, -1200, -300, 400]), 100, true)).toBe("privacy");
    expect(activeSectionAt(at([-1900, -1200, -300, 400]), 100, false)).toBe("audio");
  });

  test("null with no sections", () => {
    expect(activeSectionAt([], 100, true)).toBeNull();
  });
});

describe("scroll-spy: isScrolledToBottom", () => {
  test("true when scrollTop reaches the exact bottom", () => {
    expect(
      isScrolledToBottom({ scrollHeight: 1000, scrollTop: 600, clientHeight: 400 }),
    ).toBe(true);
  });

  test("true within the 2px sub-pixel tolerance", () => {
    // 1000 - 599 - 400 = 1 (≤ 2) → still counts as bottomed out.
    expect(
      isScrolledToBottom({ scrollHeight: 1000, scrollTop: 599, clientHeight: 400 }),
    ).toBe(true);
    // Exactly 2px of remaining slack is still "bottom".
    expect(
      isScrolledToBottom({ scrollHeight: 1000, scrollTop: 598, clientHeight: 400 }),
    ).toBe(true);
  });

  test("false when more than 2px remain below the fold", () => {
    // 1000 - 500 - 400 = 100 remaining → not bottomed out.
    expect(
      isScrolledToBottom({ scrollHeight: 1000, scrollTop: 500, clientHeight: 400 }),
    ).toBe(false);
    // Just past the tolerance edge (3px remaining) → not bottom.
    expect(
      isScrolledToBottom({ scrollHeight: 1000, scrollTop: 597, clientHeight: 400 }),
    ).toBe(false);
  });

  test("true at the top when content fits without scrolling", () => {
    // Content shorter than the viewport: there's nothing to scroll, so the
    // region is already at its (only) bottom.
    expect(
      isScrolledToBottom({ scrollHeight: 300, scrollTop: 0, clientHeight: 400 }),
    ).toBe(true);
  });

  test("accepts a real-element-shaped object (structural compatibility)", () => {
    // The shell passes its scroll-region HTMLElement directly; an element is
    // structurally a {scrollHeight, scrollTop, clientHeight} carrier.
    const elementLike = Object.assign(Object.create(null), {
      scrollHeight: 800,
      scrollTop: 400,
      clientHeight: 400,
    });
    expect(isScrolledToBottom(elementLike)).toBe(true);
  });
});

describe("scroll-spy: isAtScrollTarget", () => {
  test("false while no programmatic scroll is in flight (null target)", () => {
    // With no target the scroll-spy must NOT treat any scroll as settled —
    // suppression stays raised until a real target arrives or the timer fires.
    expect(isAtScrollTarget(0, null)).toBe(false);
    expect(isAtScrollTarget(1234, null)).toBe(false);
  });

  test("false mid-animation while still far from the target", () => {
    // A long smooth jump: scrollTop is nowhere near the target yet, so the
    // spy must stay suppressed (this is the flicker the fix prevents).
    expect(isAtScrollTarget(120, 980)).toBe(false);
  });

  test("true once scrollTop reaches the target exactly", () => {
    expect(isAtScrollTarget(980, 980)).toBe(true);
  });

  test("true within the default 4px settle tolerance (either direction)", () => {
    // Smooth scrolls land a hair short or long of the exact offset; absorb it.
    expect(isAtScrollTarget(976, 980)).toBe(true); // 4px short
    expect(isAtScrollTarget(984, 980)).toBe(true); // 4px over
  });

  test("false just past the settle tolerance", () => {
    expect(isAtScrollTarget(975, 980)).toBe(false); // 5px short
    expect(isAtScrollTarget(985, 980)).toBe(false); // 5px over
  });

  test("honors a caller-supplied tolerance", () => {
    expect(isAtScrollTarget(970, 980, 10)).toBe(true);
    expect(isAtScrollTarget(969, 980, 10)).toBe(false);
  });
});
