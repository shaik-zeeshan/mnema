// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { windowFor, shiftAnchor, humanizeMs } from "./activity-helpers";

// A duration should read in the coarsest non-zero unit: sub-minute activities as
// seconds (never a rounded "0m"), minutes, then hours with a minute remainder.
describe("humanizeMs adaptive units (seconds / minutes / hours)", () => {
  it("shows seconds under a minute instead of rounding to 0m", () => {
    expect(humanizeMs(9_000)).toBe("9s");
    expect(humanizeMs(45_000)).toBe("45s");
    expect(humanizeMs(59_000)).toBe("59s");
  });
  it("shows minutes from a minute up to an hour", () => {
    expect(humanizeMs(60_000)).toBe("1m");
    expect(humanizeMs(90_000)).toBe("2m"); // rounds to nearest minute
    expect(humanizeMs(45 * 60_000)).toBe("45m");
  });
  it("shows hours with a minute remainder past an hour", () => {
    expect(humanizeMs(60 * 60_000)).toBe("1h");
    expect(humanizeMs(65 * 60_000)).toBe("1h 5m");
    expect(humanizeMs(150 * 60_000)).toBe("2h 30m");
  });
  it("guards zero / invalid input", () => {
    expect(humanizeMs(0)).toBe("0s");
    expect(humanizeMs(-5)).toBe("0s");
    expect(humanizeMs(Number.NaN)).toBe("0s");
  });
});

// The Overview range stepper steps `anchor` by one window unit and derives the
// previous window for the period-over-period delta. Stepping a MONTH must always
// land in the adjacent calendar month — never the same month — even when the
// anchor's day-of-month (29/30/31) doesn't exist in the target month.
describe("shiftAnchor month stepping (calendar-correct, no day overflow)", () => {
  // Anchor input at NOON local so a DST transition can't nudge the date across a
  // day boundary and make the assertion flaky.
  const noon = (y: number, m0: number, day: number) =>
    new Date(y, m0, day, 12, 0, 0, 0).getTime();
  // `windowFor(_, "month").startMs` is the local-midnight first-of-month.
  const firstOfMonth = (y: number, m0: number) => new Date(y, m0, 1).getTime();
  const monthStart = (anchorMs: number) => windowFor(anchorMs, "month").startMs;

  it("steps back from a 31-day month into the PREVIOUS month, not the same one", () => {
    // Mar 31 → the previous window must be February, not March again.
    const mar31 = noon(2026, 2, 31);
    const prev = shiftAnchor(mar31, "month", -1);
    expect(monthStart(prev)).toBe(firstOfMonth(2026, 1)); // Feb 1
    expect(monthStart(prev)).not.toBe(monthStart(mar31)); // not March
  });

  it("steps forward across a short month without skipping it", () => {
    // Jan 31 → the next window must be February, not March (no Feb-skip).
    const jan31 = noon(2026, 0, 31);
    const next = shiftAnchor(jan31, "month", 1);
    expect(monthStart(next)).toBe(firstOfMonth(2026, 1)); // Feb 1
  });

  it("handles other 31→shorter month boundaries (May→Apr)", () => {
    const may31 = noon(2026, 4, 31);
    const prev = shiftAnchor(may31, "month", -1);
    expect(monthStart(prev)).toBe(firstOfMonth(2026, 3)); // Apr 1
  });

  it("still steps correctly when no overflow is involved (Jan→Dec across year)", () => {
    const jan31 = noon(2026, 0, 31);
    const prev = shiftAnchor(jan31, "month", -1);
    expect(monthStart(prev)).toBe(firstOfMonth(2025, 11)); // Dec 1, 2025
  });
});
