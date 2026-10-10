// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-day.test.ts).
import { describe, expect, it } from "bun:test";
import {
  awayLabel,
  buildRiver,
  capturedHoursByDay,
  categoryMix,
  pendingReasonCopy,
  riverRowKey,
  spineAxis,
  stackLanes,
  weekDays,
} from "./journal-view";

const DAY = new Date(2026, 6, 3, 0, 0, 0, 0).getTime();
const at = (h, m = 0) => new Date(2026, 6, 3, h, m, 0, 0).getTime();

const slot = (startMs, endMs = startMs + 60_000) => ({
  activity: { id: 1, startedAtMs: startMs, endedAtMs: endMs },
  frameCount: 1,
  expired: false,
});
const gap = (startMs, endMs) => ({ startMs, endMs });
const act = (id, startMs, endMs, category = null) => ({ id, startedAtMs: startMs, endedAtMs: endMs, category });

describe("buildRiver", () => {
  it("is newest first, gaps interleaved by start", () => {
    const rows = buildRiver([slot(at(9)), slot(at(13))], [gap(at(11), at(12))]);
    expect(rows.map((r) => r.kind)).toEqual(["card", "gap", "card"]);
    expect(rows.map((r) => r.atMs)).toEqual([at(13), at(11), at(9)]);
  });

  it("puts the shorter of two same-start cards first", () => {
    const rows = buildRiver([slot(at(9), at(11)), slot(at(9), at(9, 20))], []);
    expect(rows.map((r) => r.slot.activity.endedAtMs)).toEqual([at(9, 20), at(11)]);
  });
});

describe("spineAxis", () => {
  it("spans the real day, padded and rounded out to whole hours", () => {
    const axis = spineAxis(at(8, 41), at(15, 20));
    expect(axis.startMs).toBe(at(8));
    expect(axis.endMs).toBe(at(16));
    expect(axis.ticks).toEqual([at(8), at(10), at(12), at(14), at(16)]);
  });

  it("pads a start right on the hour back an hour and keeps at least an hour", () => {
    const axis = spineAxis(at(10), at(10, 5));
    expect(axis.startMs).toBe(at(9));
    expect(axis.endMs).toBe(at(11));
    expect(axis.ticks).toEqual([at(9), at(10), at(11)]);
  });
});

describe("stackLanes", () => {
  it("stacks overlaps into sub-lanes and reuses a lane once it frees", () => {
    const { lane, lanes } = stackLanes([gap(at(9), at(11)), gap(at(10), at(10, 30)), gap(at(11), at(12))]);
    expect(lane).toEqual([0, 1, 0]);
    expect(lanes).toBe(2);
  });

  it("gives the longer of two same-start spans the top lane", () => {
    expect(stackLanes([gap(at(9), at(9, 10)), gap(at(9), at(10))]).lane).toEqual([1, 0]);
  });
});

describe("side column", () => {
  it("tallies category time largest first", () => {
    const mix = categoryMix([act(1, at(9), at(10), "research"), act(2, at(10), at(12), "creating"), act(3, at(13), at(13, 30), "research")]);
    expect(mix).toEqual([
      { category: "creating", ms: 2 * 3_600_000 },
      { category: "research", ms: 1.5 * 3_600_000 },
    ]);
  });

  it("builds the Mon–Sun week around the day, with captured hours as the fallback", () => {
    // 2026-07-03 is a Friday → week starts Mon Jun 29.
    const mon = new Date(2026, 5, 29).getTime();
    const hours = capturedHoursByDay([
      { bucketStartMs: mon + 9 * 3_600_000, intensityCount: 4 },
      { bucketStartMs: mon + 10 * 3_600_000, intensityCount: 0 },
      { bucketStartMs: mon + 11 * 3_600_000, intensityCount: 2 },
    ]);
    const week = weekDays(DAY, [act(1, at(9), at(10), "creating")], hours, at(12));
    expect(week).toHaveLength(7);
    expect(week[0].startMs).toBe(mon);
    expect(week[0].capturedHours).toBe(2);
    expect(week[4].activities).toBe(1);
    expect(week[4].activityMs).toBe(3_600_000);
    expect(week.map((d) => d.future)).toEqual([false, false, false, false, false, true, true]);
  });
});

describe("awayLabel", () => {
  it("names only the length", () => {
    expect(awayLabel(gap(at(12), at(13, 10)))).toBe("Away · 1h 10m");
  });
});

describe("riverRowKey", () => {
  it("keys card rows by activity id, so duplicate start times can't collide", () => {
    // Two Activities sharing a startedAtMs — the shape that threw
    // each_key_duplicate and froze the river on its skeleton.
    const a = { ...slot(at(9)), activity: { ...slot(at(9)).activity, id: 1 } };
    const b = { ...slot(at(9)), activity: { ...slot(at(9)).activity, id: 2 } };
    const keys = buildRiver([a, b], []).map(riverRowKey);
    expect(new Set(keys).size).toBe(keys.length);
  });
  it("keeps gap keys distinct from card keys at the same timestamp", () => {
    const keys = buildRiver([slot(at(9))], [gap(at(9), at(10))]).map(riverRowKey);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("pendingReasonCopy", () => {
  it("maps known codes and prefixes to paused sentences", () => {
    expect(pendingReasonCopy("ai_runtime_disabled")).toContain("AI features are turned off");
    expect(pendingReasonCopy("no_provider_key:openai")).toContain("no API key");
    expect(pendingReasonCopy("provider_not_connected:x")).toContain("isn't connected");
  });
  it("names the ChatGPT reconnect code instead of the generic fallback", () => {
    // `needs_reconnect:<id>` is what the engine resolver returns for a chatgpt
    // provider with no usable token set. It reaches this surface through
    // UserContextStatus.reason exactly like the other prefixed codes, and the
    // only fix the user can take is to sign in again.
    expect(pendingReasonCopy("needs_reconnect:chatgpt")).toContain("sign in");
  });
  it("does not tell an offline user to sign in again", () => {
    // `provider_unreachable:` is the live resolver's transient code: the
    // ChatGPT login is intact, the auth endpoint just didn't answer. Rendering
    // the sign-in sentence here pushes the user toward Disconnect, which
    // destroys a working credential over a dropped Wi-Fi connection.
    const copy = pendingReasonCopy("provider_unreachable:chatgpt");
    expect(copy).not.toContain("sign in");
    expect(copy).toContain("reach");
  });
  it("falls back to a safe generic sentence for unknown codes", () => {
    expect(pendingReasonCopy("something_new")).toContain("isn't available right now");
    expect(pendingReasonCopy("")).toContain("isn't available right now");
  });
});
