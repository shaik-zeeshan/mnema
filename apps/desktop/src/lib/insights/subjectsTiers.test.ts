// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  EVIDENCE_MIN_FIT,
  FORTNIGHT_MS,
  fitRows,
  fortnightAgo,
  listGroups,
  neighbourAfter,
  pageHolding,
  pageLabel,
  pageSlice,
  pageToShow,
  rankMovers,
  SHOW_ALL,
  stepIndex,
  subjectsHeadline,
} from "./subjectsTiers";
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

describe("pageSlice", () => {
  const xs = Array.from({ length: 138 }, (_, i) => i);
  test("slices page N and reports where it is", () => {
    const p = pageSlice(xs, 1, 20);
    expect(p).toMatchObject({ page: 1, pages: 7, from: 20, to: 40, total: 138 });
    expect(p.visible[0]).toBe(20);
    expect(pageLabel(p)).toBe("21–40 of 138");
  });
  test("clamps out-of-range pages (a list that shrank under you)", () => {
    expect(pageSlice(xs, 99, 20)).toMatchObject({ page: 6, from: 120, to: 138 });
    expect(pageSlice(xs, -3, 20)).toMatchObject({ page: 0, from: 0 });
    expect(pageSlice([], 2, 20)).toMatchObject({ page: 0, pages: 1, from: 0, to: 0, total: 0 });
  });
  test("Show all is one page holding everything", () => {
    const p = pageSlice(xs, 3, SHOW_ALL);
    expect(p).toMatchObject({ page: 0, pages: 1, from: 0, to: 138 });
    expect(p.visible).toHaveLength(138);
    expect(pageLabel(p)).toBe("All 138");
    expect(pageLabel(pageSlice(xs.slice(0, 11), 1, 10))).toBe("11 of 11");
  });
});


describe("pageHolding / pageToShow", () => {
  test("the page that holds a row; Show all is always page 0", () => {
    expect(pageHolding(9, 10)).toBe(0);
    expect(pageHolding(10, 10)).toBe(1);
    expect(pageHolding(137, 25)).toBe(5);
    expect(pageHolding(137, SHOW_ALL)).toBe(0);
  });
  test("a new page size keeps the anchor row on screen", () => {
    expect(pageHolding(23, 25)).toBe(0); // rows 21–30 at 10 → 1–25 at 25
    expect(pageHolding(30, 10)).toBe(3); // rows 26–50 at 25 → 31–40 at 10
  });
  test("back to the list: turn only when the row is off the page", () => {
    const xs = Array.from({ length: 69 }, (_, i) => i);
    expect(pageToShow(14, pageSlice(xs, 1, 10), 10)).toBeNull(); // already showing
    expect(pageToShow(42, pageSlice(xs, 1, 10), 10)).toBe(4);
    expect(pageToShow(3, pageSlice(xs, 6, 10), 10)).toBe(0);
    expect(pageToShow(60, pageSlice(xs, 0, SHOW_ALL), SHOW_ALL)).toBeNull();
    expect(pageToShow(-1, pageSlice(xs, 2, 10), 10)).toBeNull(); // not in this section
  });
});

describe("stepIndex / neighbourAfter", () => {
  test("↓/↑ with nothing open go to the first / last row", () => {
    expect(stepIndex(-1, 1, 24)).toBe(0);
    expect(stepIndex(-1, -1, 24)).toBe(23);
  });
  test("steps one row, and stops at either end (no wrap-around)", () => {
    expect(stepIndex(4, 1, 24)).toBe(5);
    expect(stepIndex(4, -1, 24)).toBe(3);
    expect(stepIndex(23, 1, 24)).toBeNull();
    expect(stepIndex(0, -1, 24)).toBeNull();
    expect(stepIndex(-1, 1, 0)).toBeNull();
  });
  test("dismissing the open row moves to the next, else the previous, else nothing", () => {
    expect(neighbourAfter(3, 10)).toBe(4);
    expect(neighbourAfter(9, 10)).toBe(8);
    expect(neighbourAfter(0, 1)).toBeNull();
  });
});

describe("fitRows", () => {
  test("as many rows as fit, never under the minimum", () => {
    expect(fitRows(400, 40)).toBe(10);
    expect(fitRows(419, 40)).toBe(10);
    expect(fitRows(100, 40)).toBe(EVIDENCE_MIN_FIT);
    expect(fitRows(-50, 40)).toBe(EVIDENCE_MIN_FIT);
  });
  test("an unmeasured row (0 px, NaN) gives the minimum, never NaN or Infinity", () => {
    expect(fitRows(400, 0)).toBe(EVIDENCE_MIN_FIT);
    expect(fitRows(0, 0)).toBe(EVIDENCE_MIN_FIT);
    expect(fitRows(400, Number.NaN)).toBe(EVIDENCE_MIN_FIT);
  });
});

describe("listGroups", () => {
  const row = (subject, topConfidence, faded = false) => ({ subject, topConfidence, faded, trend: "steady", lastMovedAtMs: 0 });
  const rows = Array.from({ length: 30 }, (_, i) => row(`S${i}`, 0.95 - i * 0.03, i >= 28));

  test("every tier of the axis, empty ones too, with the axis in each id", () => {
    const conv = listGroups(rows, "conviction", null);
    expect(conv.map((g) => g.id)).toEqual(["conviction:strong", "conviction:forming", "conviction:shaping", "conviction:fading"]);
    const mov = listGroups(rows, "movement", null);
    expect(mov.every((g) => g.id.startsWith("movement:"))).toBe(true);
    // Each axis keeps its own pages: no id is shared between the axes.
    expect(conv.some((g) => mov.some((m) => m.id === g.id))).toBe(false);
    expect(listGroups(rows.slice(0, 15), "conviction", null).find((g) => g.id === "conviction:fading")?.items).toEqual([]);
  });

  test("Fading is titled plainly and flagged to start folded; notes follow the thresholds", () => {
    const g = listGroups(rows, "conviction", null);
    const fading = g.find((x) => x.id === "conviction:fading");
    expect(fading).toMatchObject({ title: "Fading", faded: true, note: "kept for history" });
    expect(fading.items.map((r) => r.subject)).toEqual(["S28", "S29"]);
    expect(g.find((x) => x.id === "conviction:strong")?.note).toBe("confidence 68% and up");
    expect(g.find((x) => x.id === "conviction:forming")?.note).toBe("confidence 30–68% · building support");
    expect(g.find((x) => x.id === "conviction:shaping")?.note).toBe("confidence under 30% · early");
  });

  test("a search is one Matches section in rank order, or nothing when nothing matches", () => {
    const matches = [rows[7], rows[2]];
    expect(listGroups(rows, "conviction", matches)).toEqual([
      { id: "search", title: "Matches", note: "", faded: false, items: matches },
    ]);
    expect(listGroups(rows, "conviction", [])).toEqual([]);
  });

  test("a sparse dossier is one untitled run", () => {
    const few = rows.slice(0, 3);
    expect(listGroups(few, "conviction", null)).toEqual([{ id: "all", title: "", note: "", faded: false, items: few }]);
  });
});
