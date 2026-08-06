// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";

import { compactCount, plural, relativeAge, statementStamp, wipeMessage } from "./data";

const NOW = 1_700_000_000_000;
const H = 3_600_000;

describe("relativeAge", () => {
  it("rounds coarsely and never goes minute-precise past an hour", () => {
    expect(relativeAge(NOW - 30_000, NOW)).toBe("just now");
    expect(relativeAge(NOW - 12 * 60_000, NOW)).toBe("12m ago");
    expect(relativeAge(NOW - 3 * H, NOW)).toBe("3h ago");
    expect(relativeAge(NOW - 12 * 24 * H, NOW)).toBe("12d ago");
    expect(relativeAge(0, NOW)).toBe("—");
  });
});

describe("statementStamp", () => {
  it("says edited only when the update really followed creation", () => {
    const created = NOW - 9 * 24 * H;
    expect(statementStamp({ createdAtMs: created, updatedAtMs: created }, NOW)).toBe("added 9d ago");
    expect(statementStamp({ createdAtMs: created, updatedAtMs: NOW - 3 * 24 * H }, NOW)).toBe(
      "edited 3d ago",
    );
  });
});

describe("compactCount / plural", () => {
  it("compacts magnitudes and agrees with its noun", () => {
    expect(compactCount(1_900_000)).toBe("1.9M");
    expect(compactCount(214)).toBe("214");
    expect(plural(1, "conclusion", "conclusions")).toBe("1 conclusion");
    expect(plural(0, "dismissal", "dismissals")).toBe("0 dismissals");
  });
});

describe("wipeMessage", () => {
  const status = {
    activityCount: 412,
    conclusionCount: 38,
    subjectCount: 12,
    dismissedCount: 7,
  };

  it("names every category `wipe_user_context` really clears, with real counts", () => {
    const msg = wipeMessage(status, 6);
    expect(msg).toContain("412 activities and 38 conclusions across 12 subjects");
    // The two categories the shipped settings confirmation forgets.
    expect(msg).toContain("Your 6 standing statements");
    expect(msg).toContain("All Quick Access and Chat ask history");
    expect(msg).toContain("Your 7 dismissals");
    expect(msg).toContain("turns AI features off");
    // …and what survives it.
    expect(msg).toContain("Every recording, frame and audio segment on disk");
  });

  it("degrades to zeroes rather than inventing numbers when status is missing", () => {
    expect(wipeMessage(null, 0)).toContain("0 activities and 0 conclusions across 0 subjects");
  });
});
