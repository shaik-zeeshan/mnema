import { describe, expect, test } from "bun:test";
import {
  clampNumber,
  clampToRange,
  parseStepperRaw,
  stepRaw,
} from "../src/lib/components/stepper-clamp";

describe("parseStepperRaw", () => {
  test("parses a plain integer", () => {
    expect(parseStepperRaw("1920")).toBe(1920);
  });

  test("trims surrounding whitespace", () => {
    expect(parseStepperRaw("  42  ")).toBe(42);
  });

  test("blank and whitespace-only are null", () => {
    expect(parseStepperRaw("")).toBeNull();
    expect(parseStepperRaw("   ")).toBeNull();
  });

  test("non-numeric is null", () => {
    expect(parseStepperRaw("abc")).toBeNull();
    expect(parseStepperRaw("12.5")).toBeNull();
    expect(parseStepperRaw("1920px")).toBeNull();
  });

  test("parses a negative integer", () => {
    expect(parseStepperRaw("-5")).toBe(-5);
  });
});

describe("clampNumber", () => {
  test("clamps above max", () => {
    expect(clampNumber(100, 1, 40)).toBe(40);
  });

  test("clamps below min", () => {
    expect(clampNumber(0, 16, 8192)).toBe(16);
  });

  test("passes a value already in range through", () => {
    expect(clampNumber(1080, 16, 8192)).toBe(1080);
  });

  test("respects an open-ended (min-only) range", () => {
    expect(clampNumber(1000, 1, undefined)).toBe(1000);
    expect(clampNumber(0, 1, undefined)).toBe(1);
  });
});

describe("clampToRange", () => {
  test("clamps above max", () => {
    expect(clampToRange("9999", { min: 16, max: 8192 })).toBe("8192");
  });

  test("clamps below min", () => {
    expect(clampToRange("8", { min: 16, max: 8192 })).toBe("16");
  });

  test("passes a valid in-range value through unchanged", () => {
    expect(clampToRange("1920", { min: 16, max: 8192 })).toBe("1920");
  });

  test("blank stays blank (preserves blank = unset)", () => {
    expect(clampToRange("", { min: 16, max: 8192 })).toBe("");
    expect(clampToRange("   ", { min: 16, max: 8192 })).toBe("");
  });

  test("non-numeric is left untouched for the upstream validator", () => {
    expect(clampToRange("abc", { min: 16, max: 8192 })).toBe("abc");
    expect(clampToRange("12.5", { min: 1, max: 40 })).toBe("12.5");
  });

  test("works with a min-only range (bitrate)", () => {
    expect(clampToRange("0", { min: 1 })).toBe("1");
    expect(clampToRange("12", { min: 1 })).toBe("12");
  });
});

describe("stepRaw", () => {
  test("increments a valid value, clamped to max", () => {
    expect(stepRaw("39", 1, 1, { min: 1, max: 40 })).toBe("40");
    expect(stepRaw("40", 1, 1, { min: 1, max: 40 })).toBe("40");
  });

  test("decrements a valid value, clamped to min", () => {
    expect(stepRaw("17", -1, 1, { min: 16, max: 8192 })).toBe("16");
    expect(stepRaw("16", -1, 1, { min: 16, max: 8192 })).toBe("16");
  });

  test("seeds from min on the first click of a blank field", () => {
    expect(stepRaw("", 1, 1, { min: 16, max: 8192 })).toBe("16");
    expect(stepRaw("", -1, 1, { min: 16, max: 8192 })).toBe("16");
  });

  test("seeds from 0 when there is no min", () => {
    expect(stepRaw("", 1, 5)).toBe("0");
  });

  test("seeds from min when the current value is non-numeric", () => {
    expect(stepRaw("oops", 1, 1, { min: 1, max: 40 })).toBe("1");
  });

  test("respects a custom step", () => {
    expect(stepRaw("100", 1, 10, { min: 0, max: 1000 })).toBe("110");
    expect(stepRaw("100", -1, 10, { min: 0, max: 1000 })).toBe("90");
  });
});
