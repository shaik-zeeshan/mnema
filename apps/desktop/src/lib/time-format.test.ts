// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { formatClock, formatClockRange, formatHour, hourCycle, localizeClockTimes, setTimeFormat, uses24h } from "./time-format";

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

describe("localizeClockTimes", () => {
  test("12-hour: 24-hour clocks gain AM/PM", () => {
    expect(localizeClockTimes("You were in Zed from 14:05 to 15:30 before the 16:00 call.", false)).toBe(
      "You were in Zed from 2:05 PM to 3:30 PM before the 4:00 PM call.",
    );
    expect(localizeClockTimes("09:20–10:50, then 00:15 and 9:05.", false)).toBe("9:20 AM–10:50 AM, then 12:15 AM and 9:05 AM.");
  });

  test("12-hour: times already carrying AM/PM are kept", () => {
    expect(localizeClockTimes("At 2:05 PM and 11:30 am, then 9:00 a.m.", false)).toBe("At 2:05 PM and 11:30 am, then 9:00 a.m.");
  });

  test("12-hour: non-times are not mangled", () => {
    for (const s of ["v1:31", "build 2.1:30", "3/1:31", "1:31:05 elapsed", "10:75", "25:00", "a 1:1 call", "x14:05", "14:051"]) {
      expect(localizeClockTimes(s, false)).toBe(s);
    }
  });

  test("12-hour: a bare H:MM is indistinguishable from a time (documented ceiling)", () => {
    expect(localizeClockTimes("Genesis 1:31", false)).toBe("Genesis 1:31 AM");
  });

  test("24-hour: AM/PM clocks become 24-hour", () => {
    expect(localizeClockTimes("From 2 PM to 2:05 pm, then 9 a.m., 12 AM and 12:30 PM.", true)).toBe(
      "From 14:00 to 14:05, then 09:00., 00:00 and 12:30.",
    );
    expect(localizeClockTimes("at 11am", true)).toBe("at 11:00");
  });

  test("24-hour: 24-hour clocks and non-times are left alone", () => {
    for (const s of ["14:05 to 15:30", "13 PM", "0 AM", "v2 PM", "1:31", "a 3 pmf"]) {
      expect(localizeClockTimes(s, true)).toBe(s);
    }
  });
});
