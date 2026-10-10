// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { FORTNIGHT_MS, fortnightAgo, subjectsHeadline } from "./subjectsTiers";

const none = { strong: [], forming: [], shaping: [], fading: [] };

describe("subjectsHeadline", () => {
  test("mockup sentence: held, forming, and a de-emphasised fading tail", () => {
    expect(
      subjectsHeadline({ ...none, strong: ["Mnema", "Rust", "Comms", "Rhythm"], forming: ["CoreML"], fading: ["Diarization", "Fitness"] }),
    ).toEqual({
      lead: "Mnema, Rust and 2 more are strongly held. CoreML is forming",
      tail: "— and Diarization and Fitness are fading into history.",
    });
  });
  test("no fading closes the lead; shaping speaks only when nothing is held or forming", () => {
    expect(subjectsHeadline({ ...none, strong: ["Rust"], shaping: ["Keys"] })).toEqual({ lead: "Rust is strongly held.", tail: "" });
    expect(subjectsHeadline({ ...none, shaping: ["Keys"] })).toEqual({ lead: "Keys is just taking shape.", tail: "" });
    expect(subjectsHeadline({ ...none, fading: ["Fitness"] })).toEqual({ lead: "Fitness is fading into history.", tail: "" });
  });
});

describe("fortnightAgo", () => {
  const NOW = 100 * 86_400_000;
  const snap = (daysAgo, confidence) => ({ snapshotAtMs: NOW - daysAgo * 86_400_000, confidence });
  test("interpolates the lead's history at now − 14 days", () => {
    const history = [snap(20, 0.4), snap(10, 0.8)];
    expect(fortnightAgo({ formedAtMs: NOW - 20 * 86_400_000, confidence: 0.9 }, history, NOW)).toBeCloseTo(0.64);
  });
  test("formed inside the fortnight → null (new)", () => {
    expect(fortnightAgo({ formedAtMs: NOW - FORTNIGHT_MS + 1, confidence: 0.5 }, [], NOW)).toBeNull();
  });
  test("no history falls back to today's confidence (no tail)", () => {
    expect(fortnightAgo({ formedAtMs: 0, confidence: 0.5 }, [], NOW)).toBe(0.5);
  });
});
