// @ts-nocheck — exercised by `bun test` (see activity-helpers.test.ts).
import { describe, expect, it } from "bun:test";
import { retentionVerdict } from "./retention";

const DAY = 86_400_000;
const NOW = 100 * DAY;

describe("retentionVerdict", () => {
  it("removed only when the span ends before the cutoff", () => {
    expect(retentionVerdict(80 * DAY, 85 * DAY, "days_14", NOW)).toEqual({ kind: "removed", days: 14 });
  });
  it("partly when the span straddles the cutoff", () => {
    expect(retentionVerdict(85 * DAY, 87 * DAY, "days_14", NOW)).toEqual({ kind: "partly", days: 14 });
  });
  it("null inside the window or with Never", () => {
    expect(retentionVerdict(95 * DAY, 96 * DAY, "days_14", NOW)).toBeNull();
    expect(retentionVerdict(0, DAY, "never", NOW)).toBeNull();
    expect(retentionVerdict(0, DAY, null, NOW)).toBeNull();
  });
});
