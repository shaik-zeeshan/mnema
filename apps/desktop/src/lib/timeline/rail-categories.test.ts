// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the svelte-check tsconfig.
import { describe, expect, it } from "bun:test";
import { categoryColor, categoryForSpan } from "./rail-categories";

const ep = (startedAtMs, endedAtMs, category) => ({ startedAtMs, endedAtMs, category });

describe("categoryForSpan", () => {
  it("returns null when no episode overlaps the run", () => {
    expect(categoryForSpan(100, 200, [])).toBeNull();
    expect(categoryForSpan(100, 200, [ep(0, 99, "research"), ep(201, 300, "creating")])).toBeNull();
  });

  it("gives a run spanning two episodes the one it overlaps most", () => {
    const eps = [ep(0, 130, "research"), ep(130, 400, "meetings")];
    expect(categoryForSpan(100, 200, eps)).toBe("meetings"); // 70 vs 30
    expect(categoryForSpan(100, 140, eps)).toBe("research"); // 30 vs 10
  });

  it("breaks an overlap tie toward the earlier episode, whatever the input order", () => {
    const eps = [ep(150, 300, "meetings"), ep(0, 150, "research")];
    expect(categoryForSpan(100, 200, eps)).toBe("research");
  });

  it("colours a single-frame run inside an episode, and an uncategorized winner stays neutral", () => {
    expect(categoryForSpan(50, 50, [ep(0, 100, "learning")])).toBe("learning");
    expect(categoryForSpan(50, 60, [ep(0, 100, null), ep(55, 56, "learning")])).toBeNull();
    expect(categoryColor(null)).toBe("var(--chart-grey-3)");
    expect(categoryColor("learning")).toBe("var(--cat-learning)");
  });
});
