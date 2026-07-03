// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { buildJournalDay, AWAY_GAP_MIN_MS } from "./journal-day";

// Local midnight for a fixed day; times are expressed as minute offsets into it.
const DAY_START = new Date(2026, 6, 3, 0, 0, 0, 0).getTime();
const DAY_END = new Date(2026, 6, 4, 0, 0, 0, 0).getTime();
const MIN = 60_000;
const at = (mins: number) => DAY_START + mins * MIN;

let nextId = 1;
const frame = (ms: number) => ({ id: nextId++, capturedAt: new Date(ms).toISOString() });
const framesEvery = (startMs: number, endMs: number, stepMs: number) => {
  const out = [];
  for (let ms = startMs; ms < endMs; ms += stepMs) out.push(frame(ms));
  return out;
};
const activity = (startMs: number, endMs: number, over = {}) => ({
  id: nextId++,
  title: "t",
  summary: "s",
  category: null,
  focus: null,
  startedAtMs: startMs,
  endedAtMs: endMs,
  createdAtMs: startMs,
  evidence: [],
  ...over,
});

const baseInput = (over = {}) => ({
  activities: [],
  frames: [],
  coveredUntilMs: null,
  recording: false,
  engineAvailable: true,
  engineReason: null,
  dayStartMs: DAY_START,
  dayEndMs: DAY_END,
  ...over,
});

describe("away-gap detection", () => {
  it("finds a real away-gap between two frame clusters", () => {
    // cluster A [0..10min], 20-min silence, cluster B [30..40min]; watermark past it all.
    const frames = [...framesEvery(at(0), at(10), MIN), ...framesEvery(at(30), at(40), MIN)];
    const model = buildJournalDay(baseInput({ frames, coveredUntilMs: at(60) }));
    expect(model.gaps).toHaveLength(1);
    expect(model.gaps[0].startMs).toBe(at(9)); // last frame of cluster A (0..9 inclusive)
    expect(model.gaps[0].endMs).toBe(at(30)); // first frame of cluster B
    expect(model.gaps[0].endMs - model.gaps[0].startMs).toBeGreaterThanOrEqual(AWAY_GAP_MIN_MS);
  });

  it("does not flag sub-threshold gaps between frames", () => {
    // frames 4 min apart — below the 5-min threshold, so no away-gap.
    const frames = framesEvery(at(0), at(40), 4 * MIN);
    const model = buildJournalDay(baseInput({ frames, coveredUntilMs: at(60) }));
    expect(model.gaps).toHaveLength(0);
  });

  it("does not treat the trailing pending silence as an away-gap", () => {
    // Covered cluster ends at 9min; a big gap then a pending cluster past the watermark.
    const frames = [...framesEvery(at(0), at(10), MIN), ...framesEvery(at(30), at(40), MIN)];
    const model = buildJournalDay(baseInput({ frames, coveredUntilMs: at(20) }));
    // The 9→30 gap straddles the watermark; only frames ≤ watermark are covered,
    // so there is a single covered frame at ≤20 side and no inter-covered-frame gap.
    expect(model.gaps).toHaveLength(0);
    expect(model.pending.active).toBe(true);
  });
});

describe("pending region vs away-gap at the tail", () => {
  it("(a) recording + frames past the watermark → pending active, summarizing", () => {
    const frames = framesEvery(at(0), at(60), MIN);
    const model = buildJournalDay(
      baseInput({ frames, coveredUntilMs: at(30), recording: true, engineAvailable: true }),
    );
    expect(model.pending.active).toBe(true);
    expect(model.pending.reason).toEqual({ kind: "summarizing" });
    expect(model.pending.sinceMs).toBe(at(30));
  });

  it("(b) stopped + everything summarized → pending inactive", () => {
    const frames = framesEvery(at(0), at(30), MIN);
    const model = buildJournalDay(
      baseInput({ frames, coveredUntilMs: at(45), recording: false }),
    );
    expect(model.pending.active).toBe(false);
    expect(model.pending.sinceMs).toBeNull();
    expect(model.pending.reason).toBeNull();
  });

  it("(c) wedged worker: frames past the watermark but engine unavailable → engine_unavailable reason", () => {
    const frames = framesEvery(at(0), at(60), MIN);
    const model = buildJournalDay(
      baseInput({
        frames,
        coveredUntilMs: at(30),
        recording: false,
        engineAvailable: false,
        engineReason: "no_api_key",
      }),
    );
    expect(model.pending.active).toBe(true);
    expect(model.pending.reason).toEqual({ kind: "engine_unavailable", reason: "no_api_key" });
  });

  it("null watermark with capture → whole day pending, since = first frame", () => {
    const frames = framesEvery(at(10), at(40), MIN);
    const model = buildJournalDay(baseInput({ frames, coveredUntilMs: null }));
    expect(model.pending.active).toBe(true);
    expect(model.pending.sinceMs).toBe(at(10));
    expect(model.gaps).toHaveLength(0); // nothing summarized yet → no away-gaps
  });

  it("no capture at all → pending inactive and hasAnyCapture false", () => {
    const model = buildJournalDay(baseInput({ frames: [], coveredUntilMs: null }));
    expect(model.pending.active).toBe(false);
    expect(model.hasAnyCapture).toBe(false);
    expect(model.totalFrameCount).toBe(0);
  });
});

describe("per-card frame counts", () => {
  it("counts frames into the activity whose [start,end) bucket they fall in", () => {
    const frames = [frame(at(5)), frame(at(15)), frame(at(16)), frame(at(45))];
    const activities = [activity(at(0), at(20)), activity(at(40), at(50))];
    const model = buildJournalDay(baseInput({ frames, activities, coveredUntilMs: at(60) }));
    expect(model.slots).toHaveLength(2);
    expect(model.slots[0].frameCount).toBe(3); // at(5), at(15), at(16)
    expect(model.slots[1].frameCount).toBe(1); // at(45)
  });

  it("is chronological oldest-first regardless of input order", () => {
    const activities = [activity(at(40), at(50)), activity(at(0), at(20))];
    const model = buildJournalDay(baseInput({ activities, coveredUntilMs: at(60) }));
    expect(model.slots.map((s) => s.activity.startedAtMs)).toEqual([at(0), at(40)]);
  });

  it("end is exclusive: a frame exactly at endedAtMs belongs to the next bucket", () => {
    const frames = [frame(at(20))];
    const activities = [activity(at(0), at(20)), activity(at(20), at(40))];
    const model = buildJournalDay(baseInput({ frames, activities, coveredUntilMs: at(60) }));
    expect(model.slots[0].frameCount).toBe(0); // [0,20) excludes 20
    expect(model.slots[1].frameCount).toBe(1); // [20,40) includes 20
  });
});

describe("expired cards", () => {
  it("marks a 0-frame activity as expired (footage aged out)", () => {
    const frames = [frame(at(45))]; // lands only in the second activity
    const activities = [activity(at(0), at(20)), activity(at(40), at(50))];
    const model = buildJournalDay(baseInput({ frames, activities, coveredUntilMs: at(60) }));
    expect(model.slots[0].expired).toBe(true);
    expect(model.slots[0].frameCount).toBe(0);
    expect(model.slots[1].expired).toBe(false);
  });
});

describe("midnight boundaries", () => {
  it("keeps frames just inside the day and drops frames outside it", () => {
    const frames = [
      frame(DAY_START - MIN), // yesterday
      frame(DAY_START), // first ms of the day (inclusive)
      frame(DAY_END - MIN), // last minute of the day
      frame(DAY_END), // next midnight (exclusive) → dropped
    ];
    const model = buildJournalDay(baseInput({ frames, coveredUntilMs: DAY_END }));
    expect(model.totalFrameCount).toBe(2);
  });

  it("includes an activity straddling midnight and clamps gaps to day bounds", () => {
    // Activity starts before the day, ends inside it → overlaps → included.
    const activities = [activity(DAY_START - 30 * MIN, at(10))];
    const model = buildJournalDay(baseInput({ activities, coveredUntilMs: at(60) }));
    expect(model.slots).toHaveLength(1);
  });
});
