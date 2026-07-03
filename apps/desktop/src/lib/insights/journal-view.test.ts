// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-day.test.ts).
import { describe, expect, it } from "bun:test";
import {
  SHORT_ACTIVITY_MAX_MS,
  buildRiver,
  bandOf,
  bandRiver,
  isShortActivity,
  pendingReasonCopy,
} from "./journal-view";

const DAY = new Date(2026, 6, 3, 0, 0, 0, 0).getTime();
const at = (h, m = 0) => new Date(2026, 6, 3, h, m, 0, 0).getTime();

const slot = (startMs) => ({
  activity: { id: 1, startedAtMs: startMs, endedAtMs: startMs + 60_000 },
  frameCount: 1,
  expired: false,
});
const gap = (startMs, endMs) => ({ startMs, endMs });

describe("buildRiver", () => {
  it("interleaves slots and gaps chronologically by start", () => {
    const rows = buildRiver([slot(at(9)), slot(at(13))], [gap(at(11), at(12))]);
    expect(rows.map((r) => r.kind)).toEqual(["card", "gap", "card"]);
    expect(rows.map((r) => r.atMs)).toEqual([at(9), at(11), at(13)]);
  });
});

describe("isShortActivity", () => {
  it("is short strictly below the threshold; exactly 5 minutes is not short", () => {
    const start = at(9);
    expect(isShortActivity({ startedAtMs: start, endedAtMs: start + SHORT_ACTIVITY_MAX_MS - 1 })).toBe(true);
    expect(isShortActivity({ startedAtMs: start, endedAtMs: start + SHORT_ACTIVITY_MAX_MS })).toBe(false);
  });
});

describe("bandOf", () => {
  it("splits the day at noon and 5pm (local hours)", () => {
    expect(bandOf(at(8))).toBe("Morning");
    expect(bandOf(at(11, 59))).toBe("Morning");
    expect(bandOf(at(12))).toBe("Afternoon");
    expect(bandOf(at(16, 59))).toBe("Afternoon");
    expect(bandOf(at(17))).toBe("Evening");
    expect(bandOf(at(23))).toBe("Evening");
  });
});

describe("bandRiver", () => {
  it("groups consecutive rows into bands, one band per contiguous run", () => {
    const rows = buildRiver(
      [slot(at(8)), slot(at(9)), slot(at(13)), slot(at(18))],
      [],
    );
    const bands = bandRiver(rows);
    expect(bands.map((b) => b.label)).toEqual(["Morning", "Afternoon", "Evening"]);
    expect(bands[0].rows).toHaveLength(2);
    expect(bands[1].rows).toHaveLength(1);
    expect(bands[2].rows).toHaveLength(1);
  });
});

describe("pendingReasonCopy", () => {
  it("maps known codes and prefixes to paused sentences", () => {
    expect(pendingReasonCopy("ai_runtime_disabled")).toContain("AI features are turned off");
    expect(pendingReasonCopy("no_provider_key:openai")).toContain("no API key");
    expect(pendingReasonCopy("provider_not_connected:x")).toContain("isn't connected");
  });
  it("falls back to a safe generic sentence for unknown codes", () => {
    expect(pendingReasonCopy("something_new")).toContain("isn't available right now");
    expect(pendingReasonCopy("")).toContain("isn't available right now");
  });
});
