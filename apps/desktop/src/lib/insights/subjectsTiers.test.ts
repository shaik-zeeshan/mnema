// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { FORTNIGHT_MS, fortnightAgo, pageOf, rankMovers, subjectsHeadline } from "./subjectsTiers";
import { rankSubjects } from "./subjectSearch";

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

describe("pageOf", () => {
  const xs = Array.from({ length: 45 }, (_, i) => i);
  test("first page, then a capped 'more', then 'less' once everything is out", () => {
    expect(pageOf(xs, 20, 20)).toMatchObject({ more: 20, less: false });
    expect(pageOf(xs, 20, 20).visible).toHaveLength(20);
    expect(pageOf(xs, 40, 20)).toMatchObject({ more: 5, less: false });
    expect(pageOf(xs, 60, 20)).toMatchObject({ more: 0, less: true });
    expect(pageOf(xs.slice(0, 7), 20, 20)).toMatchObject({ more: 0, less: false });
  });
});

describe("rankMovers", () => {
  const row = (subject, now, then, sinceMs = 0) => ({ subject, topConfidence: now, then, sinceMs });
  test("real movers by size first, then newest 'formed', flat dropped, capped", () => {
    const rows = [
      row("new-old", 0.4, null, 1),
      row("up5", 0.55, 0.5),
      row("flat", 0.5, 0.5),
      row("down20", 0.3, 0.5),
      row("new-recent", 0.4, null, 9),
    ];
    expect(rankMovers(rows).map((r) => r.subject)).toEqual(["down20", "up5", "new-recent", "new-old"]);
    expect(rankMovers(rows, 2).map((r) => r.subject)).toEqual(["down20", "up5"]);
  });
});

describe("rankSubjects as a filter", () => {
  const s = (subject, ...statements) => ({ subject, conclusions: statements.map((statement) => ({ statement })) });
  const rows = [s("AI tools", "Uses Claude daily"), s("Rust", "Writes Rust at work"), s("Fitness", "Runs in the morning")];
  test("a set query keeps only matches — even one too short to rank", () => {
    expect(rankSubjects(rows, "rust").map((r) => r.subject)).toEqual(["Rust"]);
    expect(rankSubjects(rows, "ai").map((r) => r.subject)).toEqual(["AI tools"]);
    expect(rankSubjects(rows, "zz")).toEqual([]);
    expect(rankSubjects(rows, "   ")).toEqual(rows);
  });
});
