// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { pickStrip, RANGE, sourceResult, stepCited, stripWindow, signedClock } from "./quick-look";

describe("pickStrip", () => {
  const match = { id: 100, ms: 1_000_000 };

  test("picks the frame nearest each 15 s step; step 0 is the hit", () => {
    // frames every 4 s from −92 s to +92 s around the match
    const frames = [];
    for (let s = -92; s <= 92; s += 4) frames.push({ id: 1000 + s, ms: match.ms + s * 1000 });
    const strip = pickStrip(frames, match);
    expect(strip.length).toBe(2 * RANGE + 1);
    expect(strip[RANGE]).toBe(match);
    // +15 s → +16 (nearest); −30 s → −32/−28 tie, the first scanned wins
    expect(strip[RANGE + 1].id).toBe(1016);
    expect(strip[RANGE - 2].id).toBe(1000 - 32);
    // ±90 s → ±88/±92 tie, first scanned wins
    expect(strip[0].id).toBe(1000 - 92);
    expect(strip[2 * RANGE].id).toBe(1088);
  });

  test("sparse capture repeats the nearest frame; nothing captured → null", () => {
    const strip = pickStrip([{ id: 7, ms: match.ms + 40_000 }], match);
    expect(strip[RANGE + 1].id).toBe(7);
    expect(strip[0].id).toBe(7);
    expect(pickStrip([], match)[RANGE + 1]).toBeNull();
  });
});

describe("stripWindow", () => {
  test("centres on k and clamps at the ends", () => {
    expect(stripWindow(0, -6, 6)).toEqual([-3, -2, -1, 0, 1, 2, 3]);
    expect(stripWindow(5, -6, 6)).toEqual([0, 1, 2, 3, 4, 5, 6]);
    expect(stripWindow(-6, -6, 6)).toEqual([-6, -5, -4, -3, -2, -1, 0]);
    expect(stripWindow(1, 0, 3)).toEqual([0, 1, 2, 3]);
    expect(stripWindow(5, 0, 9, 4)).toEqual([4, 5, 6, 7]);
  });
});

test("signedClock", () => {
  expect(signedClock(15_000)).toBe("+0:15");
  expect(signedClock(-45_000)).toBe("−0:45");
});

describe("stepCited", () => {
  test("walks the citations, skipping ones Quick Look can't show, stopping at the ends", () => {
    const cites = ["a", null, "c", "d"];
    expect(stepCited(cites, 0, 1)).toBe(2);
    expect(stepCited(cites, 2, -1)).toBe(0);
    expect(stepCited(cites, 3, 1)).toBe(-1);
    expect(stepCited(cites, 0, -1)).toBe(-1);
    expect(stepCited([null, "b"], 1, -1)).toBe(-1);
  });
});

describe("sourceResult", () => {
  const cite = { kind: "frame", frameId: 7, audioSegmentId: null, appName: "Notes", windowTitle: "Q4", url: null, startedAt: "2026-10-08T09:30:00Z", endedAt: "2026-10-08T09:40:00Z", sourceKind: null };
  const audio = (id, span) => ({ kind: "audio", audio: { audioSegment: { id }, spanStartMs: span } });

  test("reuses the list's own hit for the cited frame", () => {
    const hit = { kind: "frame", frame: { representativeFrame: { id: 7 } } };
    expect(sourceResult(cite, [audio(7, 0), hit])).toBe(hit);
  });

  test("builds a bare frame result when the list lacks it", () => {
    const r = sourceResult(cite, []);
    expect(r.frame.representativeFrame).toMatchObject({ id: 7, capturedAt: cite.startedAt });
    expect(r.frame.thumbnailFrameId).toBe(7);
    expect(r.frame.matchBoxes).toEqual([]);
  });

  test("audio: the nearest span of the same recording, else a built one", () => {
    const a = { ...cite, kind: "audio", frameId: null, audioSegmentId: 3, sourceKind: "system", spanStartMs: 60_000, alignedFrameId: 9 };
    const near = audio(3, 50_000);
    expect(sourceResult(a, [audio(3, 0), near, audio(4, 60_000)])).toBe(near);
    const built = sourceResult(a, []);
    expect(built.audio).toMatchObject({ sourceKind: "system_audio", spanStartMs: 60_000, alignedFrame: { id: 9 } });
    expect(built.audio.absoluteStartAt).toBe("2026-10-08T09:31:00.000Z");
  });

  test("a citation with no id has nothing to look at", () => {
    expect(sourceResult({ ...cite, frameId: null }, [])).toBeNull();
  });
});
