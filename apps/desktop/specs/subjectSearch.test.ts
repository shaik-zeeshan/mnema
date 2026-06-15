import { describe, it, expect } from "bun:test";
import {
  stem,
  queryTokens,
  rankSubjects,
  type SubjectSearchable,
} from "../src/lib/insights/subjectSearch";

function subj(
  subject: string,
  statements: string[] = [],
): SubjectSearchable {
  return { subject, conclusions: statements.map((s) => ({ statement: s })) };
}

describe("stem", () => {
  it("collapses common suffixes to a shared key", () => {
    expect(stem("running")).toBe(stem("run"));
    expect(stem("tests")).toBe(stem("test"));
    expect(stem("quickly")).toBe(stem("quick"));
  });

  it("leaves short words intact (no over-stemming)", () => {
    expect(stem("red")).toBe("red");
    expect(stem("bus")).toBe("bus");
    expect(stem("is")).toBe("is");
  });
});

describe("queryTokens", () => {
  it("lowercases, drops short words + stopwords, and de-dupes", () => {
    expect(queryTokens("The Apple apple")).toEqual([stem("apple")]);
    expect(queryTokens("a in on")).toEqual([]);
  });

  it("returns empty for blank input", () => {
    expect(queryTokens("   ")).toEqual([]);
  });
});

describe("rankSubjects", () => {
  it("passes rows through unchanged for a blank query", () => {
    const rows = [subj("Apple"), subj("Rust")];
    expect(rankSubjects(rows, "")).toBe(rows);
    expect(rankSubjects(rows, "   ")).toBe(rows);
  });

  it("filters out non-matching subjects", () => {
    const rows = [
      subj("Apple", ["Interested in Apple hardware"]),
      subj("Gardening", ["Grows tomatoes"]),
    ];
    const out = rankSubjects(rows, "apple");
    expect(out.map((r) => r.subject)).toEqual(["Apple"]);
  });

  it("matches against conclusion statements, not just the name", () => {
    const rows = [
      subj("Hardware", ["Keeps buying Apple laptops"]),
      subj("Gardening", ["Grows tomatoes"]),
    ];
    const out = rankSubjects(rows, "apple");
    expect(out.map((r) => r.subject)).toEqual(["Hardware"]);
  });

  it("ranks a name hit above a statement-only hit", () => {
    const rows = [
      subj("Hardware", ["Owns an apple orchard reference"]),
      subj("Apple", ["Uses several devices"]),
    ];
    const out = rankSubjects(rows, "apple");
    expect(out[0].subject).toBe("Apple");
  });

  it("does whole-word matching (cat does not match category)", () => {
    const rows = [subj("Categories", ["Sorts things into categories"])];
    expect(rankSubjects(rows, "cat")).toEqual([]);
  });

  it("breaks score ties by input order", () => {
    const rows = [
      subj("Beta", ["mentions apple once"]),
      subj("Alpha", ["mentions apple once"]),
    ];
    const out = rankSubjects(rows, "apple");
    expect(out.map((r) => r.subject)).toEqual(["Beta", "Alpha"]);
  });
});
