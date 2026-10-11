// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { formatClock, formatClockRange, formatHour, hourCycle, setTimeFormat, uses24h } from "./time-format";

const at = (h: number, m: number, s = 0) => new Date(2026, 9, 11, h, m, s);

describe("formatClock", () => {
  test("24-hour pads the hour", () => {
    expect(formatClock(at(9, 5), { h24: true })).toBe("09:05");
    expect(formatClock(at(14, 5, 9), { h24: true, seconds: true })).toBe("14:05:09");
  });

  test("12-hour adds AM/PM", () => {
    expect(formatClock(at(9, 5), { h24: false })).toBe("9:05 AM");
    expect(formatClock(at(14, 5, 9), { h24: false, seconds: true })).toBe("2:05:09 PM");
  });

  test("midnight and noon", () => {
    expect(formatClock(at(0, 0), { h24: true })).toBe("00:00");
    expect(formatClock(at(0, 0), { h24: false })).toBe("12:00 AM");
    expect(formatClock(at(12, 0), { h24: true })).toBe("12:00");
    expect(formatClock(at(12, 0), { h24: false })).toBe("12:00 PM");
  });

  test("accepts epoch ms; invalid reads empty", () => {
    expect(formatClock(at(9, 5).getTime(), { h24: true })).toBe("09:05");
    expect(formatClock(NaN, { h24: true })).toBe("");
  });
});

describe("formatClockRange", () => {
  test("24-hour", () => {
    expect(formatClockRange(at(9, 20), at(10, 50), { h24: true })).toBe("09:20–10:50");
  });

  test("12-hour says a shared AM/PM once", () => {
    expect(formatClockRange(at(9, 20), at(10, 50), { h24: false })).toBe("9:20–10:50 AM");
    expect(formatClockRange(at(11, 30), at(13, 5), { h24: false })).toBe("11:30 AM–1:05 PM");
    expect(formatClockRange(at(23, 30), at(0, 15), { h24: false, sep: " – " })).toBe("11:30 PM – 12:15 AM");
  });
});

describe("formatHour", () => {
  test("both cycles, incl. midnight/noon/24", () => {
    expect([0, 8, 12, 13, 24].map((h) => formatHour(h, { h24: true }))).toEqual(["00", "08", "12", "13", "00"]);
    expect([0, 8, 12, 13, 24].map((h) => formatHour(h, { h24: false }))).toEqual([
      "12 AM",
      "8 AM",
      "12 PM",
      "1 PM",
      "12 AM",
    ]);
  });
});

describe("setting", () => {
  test("explicit h12/h24 override the system cycle", () => {
    setTimeFormat("h24");
    expect(uses24h()).toBe(true);
    expect(hourCycle()).toBe("h23");
    expect(formatClock(at(15, 0))).toBe("15:00");
    setTimeFormat("h12");
    expect(uses24h()).toBe(false);
    expect(formatClock(at(15, 0))).toBe("3:00 PM");
    setTimeFormat(undefined);
  });
});
