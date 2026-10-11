// @ts-nocheck — run under `bun test` (bun:test types aren't in svelte-check's tsconfig).
import { describe, expect, it } from "bun:test";
import { activityColumns, axisMax, deepFocusDelta, fmtAxisTick, focusHeat } from "./lede-stats";

describe("axisMax / fmtAxisTick", () => {
  const min = (m) => m * 60000;
  it("rounds the peak up to the next nice step, so a light week isn't drawn on an 8h scale", () => {
    expect(axisMax(min(43))).toBe(min(60));
    expect(axisMax(min(61))).toBe(min(120));
    expect(axisMax(min(5 * 60))).toBe(min(8 * 60));
    expect(axisMax(min(8 * 60))).toBe(min(8 * 60));
    expect(axisMax(min(13 * 60))).toBe(min(16 * 60));
  });
  it("keeps a 20m floor so a few seconds aren't drawn full-height", () => {
    expect(axisMax(0)).toBe(min(20));
    expect(axisMax(5000)).toBe(min(20));
  });
  it("grows past a day in 8h steps (overlapping activities)", () => {
    expect(axisMax(min(25 * 60))).toBe(min(32 * 60));
  });
  it("labels every quarter cleanly", () => {
    for (const top of [20, 40, 60, 120, 240, 480, 720, 960, 1440])
      for (const q of [1, 2, 3, 4]) expect(fmtAxisTick((min(top) * q) / 4)).toMatch(/^\d+(\.5)?[mh]$/);
    expect(fmtAxisTick(min(15))).toBe("15m");
    expect(fmtAxisTick(min(90))).toBe("1.5h");
    expect(fmtAxisTick(min(360))).toBe("6h");
  });
});

const at = (d, h, m = 0) => new Date(2026, 9, d, h, m).getTime();
const act = (d, h0, h1, focus, category = "creating") => ({ startedAtMs: at(d, h0), endedAtMs: at(d, h1), focus, category });

describe("deepFocusDelta", () => {
  it("is the change in deep share by activity count, not minutes", () => {
    // cur: 1 deep of 2 (50%) even though the deep one is 6h long; prev: 1 of 4 (25%).
    const cur = [act(5, 9, 15, "deep"), act(5, 15, 16, "mixed")];
    const prev = [act(1, 9, 10, "deep"), act(1, 10, 11, "mixed"), act(1, 11, 12, "mixed"), act(1, 12, 13, "distracted")];
    expect(deepFocusDelta(cur, prev)).toBe(25);
  });
  it("ignores untagged activities and is null with nothing to compare", () => {
    expect(deepFocusDelta([act(5, 9, 10, "deep"), act(5, 10, 11, null)], [act(1, 9, 10, "mixed")])).toBe(100);
    expect(deepFocusDelta([act(5, 9, 10, "deep")], [act(1, 9, 10, null)])).toBeNull();
  });
});

describe("activityColumns", () => {
  it("draws every day of a week, empty days as zero columns", () => {
    const cols = activityColumns([act(6, 9, 11, "deep")], at(5, 0), at(12, 0), "week", at(9, 15));
    expect(cols.map((c) => c.label)).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
    expect(cols.map((c) => c.totalMs / 36e5)).toEqual([0, 2, 0, 0, 0, 0, 0]);
    expect(cols[4].live && cols[5].future).toBe(true);
  });
  it("splits a day into hours, trimmed to the captured span", () => {
    const cols = activityColumns([act(5, 9, 13, "deep", "research")], at(5, 0), at(6, 0), "day", at(9, 0));
    expect(cols.map((c) => [c.label, c.totalMs / 6e4])).toEqual([["09", 60], ["10", 60], ["11", 60], ["12", 60]]);
  });
});

describe("focusHeat", () => {
  it("labels month rows by ISO week, not weekday", () => {
    const rows = focusHeat([act(5, 9, 10, "deep"), act(6, 9, 10, "deep"), act(13, 9, 10, "mixed")], at(1, 0), at(32, 0), "month");
    expect(rows.map((r) => r.label)).toEqual(["W41", "W42"]);
    expect(rows[0].cells[4]).toEqual({ band: "deep", minutes: 60 });
  });
});
