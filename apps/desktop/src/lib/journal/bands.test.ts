// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig (same as journal-view.test.ts).
import { describe, expect, it } from "bun:test";
import { buildBands, bandRowKey } from "./bands";

const at = (h, m = 0) => new Date(2026, 6, 3, Math.floor(h), m + (h % 1) * 60, 0, 0).getTime();
const slot = (id, startH, endH) => ({
  activity: { id, startedAtMs: at(startH), endedAtMs: at(endH) },
  frameCount: 4,
  expired: false,
});
const pending = (h) => ({ active: true, sinceMs: at(h), reason: { kind: "summarizing" } });
const noPending = { active: false, sinceMs: null, reason: null };

describe("buildBands", () => {
  it("counts activities and spans the band, gaps excluded from the count", () => {
    const bands = buildBands(
      [slot(1, 9, 10), slot(2, 11, 11.5)],
      [{ startMs: at(10, 10), endMs: at(10, 40) }],
      noPending,
    );
    expect(bands).toHaveLength(1);
    expect(bands[0].label).toBe("Morning");
    expect(bands[0].count).toBe(2);
    expect(bands[0].startMs).toBe(at(9));
    expect(bands[0].endMs).toBe(at(11, 30));
    // A trailing gap into the next band never extends this band's hours.
    const trailing = buildBands([slot(1, 9, 10)], [{ startMs: at(10), endMs: at(13) }], noPending);
    expect(trailing[0].endMs).toBe(at(10));
    expect(bands[0].rows.map(bandRowKey)).toEqual(["card1", `gap${at(10, 10)}`, "card2"]);
  });

  it("folds the pending row into its own band and makes the band run to now", () => {
    const bands = buildBands([slot(1, 13, 14)], [], pending(15));
    expect(bands).toHaveLength(1);
    expect(bands[0].rows.at(-1).kind).toBe("pending");
    expect(bands[0].endMs).toBeNull();
  });

  it("opens a new band when the pending watermark falls past the last band", () => {
    const bands = buildBands([slot(1, 9, 10)], [], pending(18));
    expect(bands.map((b) => b.label)).toEqual(["Morning", "Evening"]);
    expect(bands[0].endMs).toBe(at(10));
    expect(bands[1].endMs).toBeNull();
    expect(bands[1].count).toBe(0);
  });
});
