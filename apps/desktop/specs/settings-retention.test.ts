import { describe, expect, test } from "bun:test";
import {
  retentionLabel,
  retentionPresets,
  retentionToDays,
} from "../src/lib/components/retention";
import type { RetentionPolicy } from "../src/lib/types";

describe("retentionPresets", () => {
  test("offers exactly the four supported RetentionPolicy values", () => {
    const values = retentionPresets().map((p) => p.value);
    expect(values).toEqual(["days_7", "days_14", "days_30", "never"]);
  });

  test("orders by ascending duration with Forever last", () => {
    const days = retentionPresets().map((p) => retentionToDays(p.value));
    expect(days).toEqual([7, 14, 30, null]);
  });

  test("uses friendly duration labels and maps never to Forever", () => {
    const byValue = Object.fromEntries(
      retentionPresets().map((p) => [p.value, p.label]),
    );
    expect(byValue.days_7).toBe("7 days");
    expect(byValue.days_14).toBe("14 days");
    expect(byValue.days_30).toBe("30 days");
    expect(byValue.never).toBe("Forever");
  });

  test("does not expose unsupported (non-persistable) presets", () => {
    const labels = retentionPresets().map((p) => p.label);
    expect(labels).not.toContain("90 days");
    expect(labels).not.toContain("1 year");
    // Every exposed value is a member of the closed enum.
    const allowed: RetentionPolicy[] = ["never", "days_7", "days_14", "days_30"];
    for (const preset of retentionPresets()) {
      expect(allowed).toContain(preset.value);
    }
  });
});

describe("retentionToDays", () => {
  test("maps day policies to their day count", () => {
    expect(retentionToDays("days_7")).toBe(7);
    expect(retentionToDays("days_14")).toBe(14);
    expect(retentionToDays("days_30")).toBe(30);
  });

  test("maps never to null (unbounded)", () => {
    expect(retentionToDays("never")).toBeNull();
  });
});

describe("retentionLabel", () => {
  test("returns the friendly label for each policy", () => {
    expect(retentionLabel("days_7")).toBe("7 days");
    expect(retentionLabel("days_14")).toBe("14 days");
    expect(retentionLabel("days_30")).toBe("30 days");
    expect(retentionLabel("never")).toBe("Forever");
  });
});
