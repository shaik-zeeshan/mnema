import { describe, expect, test } from "bun:test";
import { SETTINGS_GROUPS } from "../src/lib/settings/groups";
import {
  isScrolledToBottom,
  lastSectionOfGroup,
} from "../src/lib/settings/scroll-spy";

// The scroll-spy tail-selection + bottom-out predicate had two dedicated fix
// commits but no test. These pin the pure decisions both fixes hinge on.

describe("scroll-spy: lastSectionOfGroup", () => {
  test("returns the FINAL section of each group (the spy-unreachable tail)", () => {
    // The whole point of the helper: the tail is the last section in render
    // order, which the IntersectionObserver can't mark active on bottom-out.
    for (const group of SETTINGS_GROUPS) {
      const expected = group.sections[group.sections.length - 1].id;
      expect(lastSectionOfGroup(group.id)).toBe(expected);
    }
  });

  test("Capture's tail is Privacy (last of capture/video/audio/privacy)", () => {
    expect(lastSectionOfGroup("capture")).toBe("privacy");
  });

  test("Intelligence's tail is Semantic Search (the deepest scroll target)", () => {
    expect(lastSectionOfGroup("intelligence")).toBe("semanticSearch");
  });

  test("returns null for an unknown group id", () => {
    // @ts-expect-error — deliberately passing a non-group id.
    expect(lastSectionOfGroup("not-a-group")).toBeNull();
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
