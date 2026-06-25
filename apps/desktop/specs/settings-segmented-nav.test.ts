import { describe, expect, test } from "bun:test";
import {
  focusableIndex,
  navTargetIndex,
  nextEnabledIndex,
  type SegmentedNavOption,
} from "../src/lib/components/segmented-nav";

const OPTS: SegmentedNavOption[] = [
  { value: "a" },
  { value: "b" },
  { value: "c" },
  { value: "d" },
];

describe("nextEnabledIndex", () => {
  test("steps to the adjacent index when nothing is disabled", () => {
    expect(nextEnabledIndex(OPTS, [], 1, 1)).toBe(2);
    expect(nextEnabledIndex(OPTS, [], 1, -1)).toBe(0);
  });

  test("skips over disabled options in the step direction", () => {
    // From a (0) forward, b and c are disabled -> lands on d (3).
    expect(nextEnabledIndex(OPTS, ["b", "c"], 0, 1)).toBe(3);
    // From d (3) backward, c and b disabled -> lands on a (0).
    expect(nextEnabledIndex(OPTS, ["b", "c"], 3, -1)).toBe(0);
  });

  test("wraps around the ends", () => {
    // From d (3) forward wraps to a (0).
    expect(nextEnabledIndex(OPTS, [], 3, 1)).toBe(0);
    // From a (0) backward wraps to d (3).
    expect(nextEnabledIndex(OPTS, [], 0, -1)).toBe(3);
  });

  test("wraps past a disabled edge", () => {
    // From c (2) forward, d disabled -> wraps past d to a (0).
    expect(nextEnabledIndex(OPTS, ["d"], 2, 1)).toBe(0);
    // From b (1) backward, a disabled -> wraps to d (3).
    expect(nextEnabledIndex(OPTS, ["a"], 1, -1)).toBe(3);
  });

  test("returns null when every option is disabled", () => {
    expect(nextEnabledIndex(OPTS, ["a", "b", "c", "d"], 0, 1)).toBeNull();
    expect(nextEnabledIndex(OPTS, ["a", "b", "c", "d"], 2, -1)).toBeNull();
  });
});

describe("focusableIndex (roving tabindex)", () => {
  test("is the active value's index when it is enabled", () => {
    expect(focusableIndex(OPTS, [], "c")).toBe(2);
  });

  test("falls back to the first enabled segment when the active value is disabled", () => {
    // Regression for the keyboard-unreachable bug: the selected value is itself
    // in disabledValues, so the roving tabindex must move to the first ENABLED
    // segment rather than stranding focus on the disabled active one.
    expect(focusableIndex(OPTS, ["a"], "a")).toBe(1);
    // First two disabled, active is the disabled "b" -> first enabled is c (2).
    expect(focusableIndex(OPTS, ["a", "b"], "b")).toBe(2);
  });

  test("falls back to the first enabled segment when there is no active value", () => {
    expect(focusableIndex(OPTS, [], "")).toBe(0);
    expect(focusableIndex(OPTS, ["a"], "")).toBe(1);
  });

  test("returns -1 when every option is disabled (nothing focusable)", () => {
    expect(focusableIndex(OPTS, ["a", "b", "c", "d"], "a")).toBe(-1);
    expect(focusableIndex(OPTS, ["a", "b", "c", "d"], "")).toBe(-1);
  });
});

describe("navTargetIndex", () => {
  test("ArrowRight / ArrowDown step forward skipping disabled", () => {
    expect(navTargetIndex(OPTS, [], 0, "ArrowRight")).toBe(1);
    expect(navTargetIndex(OPTS, [], 0, "ArrowDown")).toBe(1);
    expect(navTargetIndex(OPTS, ["b"], 0, "ArrowRight")).toBe(2);
  });

  test("ArrowLeft / ArrowUp step backward skipping disabled", () => {
    expect(navTargetIndex(OPTS, [], 2, "ArrowLeft")).toBe(1);
    expect(navTargetIndex(OPTS, [], 2, "ArrowUp")).toBe(1);
    expect(navTargetIndex(OPTS, ["b"], 2, "ArrowLeft")).toBe(0);
  });

  test("Home lands on the first ENABLED segment", () => {
    expect(navTargetIndex(OPTS, [], 3, "Home")).toBe(0);
    // First two disabled -> Home falls inward to the first enabled (c = 2).
    expect(navTargetIndex(OPTS, ["a", "b"], 3, "Home")).toBe(2);
  });

  test("End lands on the last ENABLED segment", () => {
    expect(navTargetIndex(OPTS, [], 0, "End")).toBe(3);
    // Last two disabled -> End falls inward to the last enabled (b = 1).
    expect(navTargetIndex(OPTS, ["c", "d"], 0, "End")).toBe(1);
  });

  test("returns null for a non-nav key", () => {
    expect(navTargetIndex(OPTS, [], 0, "Enter")).toBeNull();
    expect(navTargetIndex(OPTS, [], 0, " ")).toBeNull();
    expect(navTargetIndex(OPTS, [], 0, "a")).toBeNull();
  });

  test("returns null when no enabled target exists (all disabled)", () => {
    expect(navTargetIndex(OPTS, ["a", "b", "c", "d"], 0, "ArrowRight")).toBeNull();
    expect(navTargetIndex(OPTS, ["a", "b", "c", "d"], 0, "Home")).toBeNull();
    expect(navTargetIndex(OPTS, ["a", "b", "c", "d"], 0, "End")).toBeNull();
  });
});
