import { describe, expect, test } from "bun:test";
import {
  dateKeyOf,
  monthKeyOf,
  pad2,
} from "../src/lib/timeline/jumper-cache-core";

// The jumper cache keys every map/set entry on a string built from these
// helpers. The whole disabled-date map, the drop-by-month-prefix swap, and the
// "is this day in a loaded month" gate all hinge on (a) zero-padding being
// stable and (b) the date key being unambiguous across padding boundaries and
// nesting under its month key. These are pure and rune-free, so they're tested
// directly (no extraction needed).

describe("pad2", () => {
  test("zero-pads single digits to two chars", () => {
    expect(pad2(0)).toBe("00");
    expect(pad2(1)).toBe("01");
    expect(pad2(9)).toBe("09");
  });

  test("leaves two-digit values intact", () => {
    expect(pad2(10)).toBe("10");
    expect(pad2(12)).toBe("12");
    expect(pad2(31)).toBe("31");
  });

  test("never truncates a value already wider than two chars (year)", () => {
    expect(pad2(2026)).toBe("2026");
  });
});

describe("monthKeyOf / dateKeyOf", () => {
  test("monthKeyOf is YYYY-MM with a padded month", () => {
    expect(monthKeyOf({ year: 2026, month: 6 })).toBe("2026-06");
    expect(monthKeyOf({ year: 2026, month: 12 })).toBe("2026-12");
  });

  test("dateKeyOf is YYYY-MM-DD with padded month and day", () => {
    expect(dateKeyOf({ year: 2026, month: 6, day: 3 })).toBe("2026-06-03");
    expect(dateKeyOf({ year: 2026, month: 12, day: 31 })).toBe("2026-12-31");
    expect(dateKeyOf({ year: 2026, month: 1, day: 9 })).toBe("2026-01-09");
  });

  test("dateKeyOf always nests under its own monthKeyOf prefix", () => {
    // The atomic month swap drops prior rows via `key.startsWith(`${monthKey}-`)`,
    // so a day key MUST begin with its month key + "-" or stale rows leak.
    for (const d of [
      { year: 2026, month: 1, day: 1 },
      { year: 2026, month: 6, day: 15 },
      { year: 2026, month: 12, day: 31 },
    ]) {
      expect(dateKeyOf(d).startsWith(`${monthKeyOf(d)}-`)).toBe(true);
    }
  });

  test("padding makes day/month keys unambiguous across boundaries", () => {
    // Without zero-pad, (month=1,day=23) and (month=12,day=3) could both render
    // as "...-1-..."/"...-...-3" style collisions. Padding keeps every distinct
    // (y,m,d) a distinct string.
    const a = dateKeyOf({ year: 2026, month: 1, day: 23 });
    const b = dateKeyOf({ year: 2026, month: 12, day: 3 });
    expect(a).toBe("2026-01-23");
    expect(b).toBe("2026-12-03");
    expect(a).not.toBe(b);
    // Month-key boundary: Jan vs Dec never share a prefix.
    expect(monthKeyOf({ year: 2026, month: 1 })).not.toBe(
      monthKeyOf({ year: 2026, month: 12 }),
    );
  });
});
