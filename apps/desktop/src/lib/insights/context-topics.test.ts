// @ts-nocheck — exercised by `bun test` (see activity-helpers.test.ts).
import { describe, expect, it } from "bun:test";
import { topicCounts } from "./context-topics";

describe("topicCounts", () => {
  it("counts null and unknown topics under Other", () => {
    const rows = topicCounts([{ topic: "Focus" }, { topic: null }, { topic: "Work" }]);
    expect(rows.map((r) => [r.name, r.count])).toEqual([
      ["Work", 1],
      ["Preferences", 0],
      ["People", 0],
      ["Goals", 0],
      ["Other", 2],
    ]);
  });
  it("omits Other when every statement has a known topic", () => {
    expect(topicCounts([{ topic: "Goals" }]).map((r) => r.name)).not.toContain("Other");
  });
});
