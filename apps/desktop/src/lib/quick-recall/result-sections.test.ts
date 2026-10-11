// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { mergeByRank, momentTime, moreRowLabel, visibleCount } from "./result-sections";

const frame = (id, rank, at = "2026-10-06T10:00:00Z") => ({ groupKey: `f${id}`, rank, groupStartAt: at });
const clip = (id, rank, at = "2026-10-06T10:00:00Z") => ({ groupKey: `a${id}`, rank, absoluteStartAt: at });
const keys = (list) => list.map((r) => (r.kind === "frame" ? r.frame.groupKey : r.audio.groupKey));

describe("mergeByRank", () => {
  test("lower rank first across kinds, each list's order kept", () => {
    const merged = mergeByRank([frame(1, -9), frame(2, -3), frame(3, -1)], [clip(1, -7), clip(2, -2)]);
    expect(keys(merged)).toEqual(["f1", "a1", "f2", "a2", "f3"]);
  });

  test("a rank tie goes to the newer start", () => {
    expect(keys(mergeByRank([frame(1, -5, "2026-10-06T09:00:00Z")], [clip(1, -5, "2026-10-06T11:00:00Z")]))).toEqual(["a1", "f1"]);
    expect(keys(mergeByRank([frame(1, -5, "2026-10-06T11:00:00Z")], [clip(1, -5, "2026-10-06T09:00:00Z")]))).toEqual(["f1", "a1"]);
  });

  test("a full tie goes to the frame", () => {
    expect(keys(mergeByRank([frame(1, -5)], [clip(1, -5)]))).toEqual(["f1", "a1"]);
  });

  test("one kind empty passes the other through", () => {
    expect(keys(mergeByRank([], [clip(1, 1), clip(2, 2)]))).toEqual(["a1", "a2"]);
    expect(keys(mergeByRank([frame(1, 1)], []))).toEqual(["f1"]);
    expect(mergeByRank([], [])).toEqual([]);
  });
});

describe("show more", () => {
  test("one cap of 8, expanded shows everything", () => {
    expect(visibleCount(5, false)).toBe(5);
    expect(visibleCount(36, false)).toBe(8);
    expect(visibleCount(36, true)).toBe(36);
  });

  test("label only when the list overflows", () => {
    expect(moreRowLabel(8, false)).toBeNull();
    expect(moreRowLabel(36, false)).toBe("↓ show 28 more");
    expect(moreRowLabel(36, true)).toBe("↑ show less");
  });
});

describe("momentTime", () => {
  const now = new Date(2026, 9, 9, 18, 0);
  const time = (d) => d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

  test("today is just the time", () => {
    const at = new Date(2026, 9, 9, 14, 12);
    expect(momentTime(at, now)).toBe(time(at));
  });

  test("this week adds the weekday", () => {
    const at = new Date(2026, 9, 6, 14, 12);
    expect(momentTime(at, now)).toBe(`${at.toLocaleDateString(undefined, { weekday: "short" })} ${time(at)}`);
  });

  test("older is the date", () => {
    const at = new Date(2026, 8, 20, 9, 0);
    expect(momentTime(at, now)).toBe(at.toLocaleDateString(undefined, { month: "short", day: "numeric" }));
  });
});
