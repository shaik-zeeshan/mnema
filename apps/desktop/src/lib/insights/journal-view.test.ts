// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-day.test.ts).
import { describe, expect, it } from "bun:test";
import {
  awayLabel,
  buildRiver,
  capturedHoursByDay,
  categoryMix,
  foldCards,
  pendingReasonCopy,
  shownSlots,
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
// A card shown as-is (no folding), for the river-ordering tests.
const card = (s) => ({ slot: s, startMs: s.activity.startedAtMs, endMs: s.activity.endedAtMs, subs: [], slim: false });
const act = (id, startMs, endMs, category = null) => ({ id, startedAtMs: startMs, endedAtMs: endMs, category });

describe("buildRiver", () => {
  it("is newest first, gaps interleaved by start", () => {
    const rows = buildRiver([card(slot(at(9))), card(slot(at(13)))], [gap(at(11), at(12))]);
    expect(rows.map((r) => r.kind)).toEqual(["card", "gap", "card"]);
    expect(rows.map((r) => r.atMs)).toEqual([at(13), at(11), at(9)]);
  });

  it("puts the shorter of two same-start cards first", () => {
    const rows = buildRiver([card(slot(at(9), at(11))), card(slot(at(9), at(9, 20)))], []);
    expect(rows.map((r) => r.slot.activity.endedAtMs)).toEqual([at(9, 20), at(11)]);
  });
});

describe("foldCards", () => {
  // A frame-backed activity: `frames` screen frames, given category.
  const fa = (id, startMs, endMs, category = null, frames = 20) => ({
    activity: { id, startedAtMs: startMs, endedAtMs: endMs, category },
    frameCount: frames,
    media: "frames",
  });
  const shape = (cards) => cards.map((c) => [c.slot.activity.id, c.subs.map((s) => s.slot.activity.id), c.slim]);

  it("folds a short activity into the same-category neighbour over a closer one", () => {
    const cards = foldCards([
      fa(1, at(9), at(9, 30), "communication"),
      fa(2, at(9, 30), at(10), "creating"),
      fa(3, at(10, 4), at(10, 4) + 6_000, "communication"), // 6 s, 4 min after 2
      fa(4, at(10, 8) + 30_000, at(11), "communication"), // ~4.4 min after 3: further, same category
    ]);
    expect(shape(cards)).toEqual([[1, [], false], [2, [], false], [4, [3], false]]);
  });

  it("folds into the closer neighbour when no category matches", () => {
    const cards = foldCards([
      fa(1, at(9), at(9, 30), "research"),
      fa(2, at(9, 31), at(9, 32), "communication", 1), // 1 frame, 1 min after 1
      fa(3, at(9, 35), at(10), "creating"),
    ]);
    expect(shape(cards)).toEqual([[1, [2], false], [3, [], false]]);
  });

  it("treats too few frames as short even when the span is long", () => {
    const cards = foldCards([fa(1, at(9), at(9, 30)), fa(2, at(9, 30), at(9, 40), null, 2)]);
    expect(shape(cards)).toEqual([[1, [2], false]]);
  });

  it("keeps an isolated short activity as its own slim row", () => {
    const cards = foldCards([fa(1, at(9), at(9, 30)), fa(2, at(9, 40), at(9, 40) + 6_000), fa(3, at(9, 50), at(10, 30))]);
    expect(shape(cards)).toEqual([[1, [], false], [2, [], true], [3, [], false]]);
  });

  it("clips an overlapping card to start where the earlier ends, or folds it when nothing remains", () => {
    const cards = foldCards([
      fa(1, at(6, 37), at(7, 13)), // 36 min
      fa(2, at(6, 37), at(7, 10)), // 33 min, same start → nothing remains
      fa(3, at(7), at(7, 40)), // overlaps → shown from 7:13
    ]);
    expect(shape(cards)).toEqual([[1, [2], false], [3, [], false]]);
    expect([cards[1].startMs, cards[1].endMs]).toEqual([at(7, 13), at(7, 40)]);
  });

  it("tallies the Day mix over the shown spans, never counting overlap twice", () => {
    const cards = foldCards([
      fa(1, at(6, 37), at(7, 13), "creating"),
      fa(2, at(6, 37), at(7, 10), "entertainment"), // nested → sub-line, no time of its own
      fa(3, at(7), at(7, 40), "creating"), // shown from 7:13
    ]);
    const mix = categoryMix(shownSlots(cards).map((s) => s.activity));
    expect(mix).toEqual([{ category: "creating", ms: at(7, 40) - at(6, 37) }]);
  });

  it("counts only full cards and slim rows, never sub-lines", () => {
    const slots = [fa(1, at(9), at(10)), fa(2, at(10), at(10) + 6_000), fa(3, at(10, 1), at(10, 1)), fa(4, at(13), at(13) + 1_000)];
    const cards = foldCards(slots);
    expect(cards).toHaveLength(2);
    expect(buildRiver(cards, []).filter((r) => r.kind === "card")).toHaveLength(2);
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
    const keys = buildRiver([card(a), card(b)], []).map(riverRowKey);
    expect(new Set(keys).size).toBe(keys.length);
  });
  it("keeps gap keys distinct from card keys at the same timestamp", () => {
    const keys = buildRiver([card(slot(at(9)))], [gap(at(9), at(10))]).map(riverRowKey);
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
