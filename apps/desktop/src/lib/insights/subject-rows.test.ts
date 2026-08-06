// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import type { Conclusion } from "$lib/types/recording";
import {
  SPARK_LEAD,
  SPARK_REST,
  buildSubjectRows,
  displayedSubjectOrder,
  metaLabel,
  sortDisplayRows,
} from "./subject-rows";

function conclusion(over: Partial<Conclusion> = {}): Conclusion {
  return {
    id: 1,
    subject: "s",
    statement: "stmt",
    confidence: 0.5,
    status: "visible",
    pinned: false,
    formedAtMs: 1_000,
    lastSupportedAtMs: 5_000,
    updatedAtMs: 5_000,
    evidence: [],
    ...over,
  };
}

describe("buildSubjectRows", () => {
  test("the row's number is the TOP conclusion's confidence, never an average", () => {
    const rows = buildSubjectRows(
      [
        conclusion({ id: 1, subject: "a", confidence: 0.72 }),
        conclusion({ id: 2, subject: "a", confidence: 0.2 }),
      ],
      new Map(),
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].topConfidence).toBe(0.72);
    expect(rows[0].conclusionCount).toBe(2);
    // The headline is the top conclusion's wording, not the first-loaded one.
    expect(rows[0].conclusions[0].id).toBe(1);
  });

  test("below-floor counts faded conclusions only inside an ACTIVE subject", () => {
    const [active] = buildSubjectRows(
      [
        conclusion({ id: 1, subject: "a", confidence: 0.72 }),
        conclusion({ id: 2, subject: "a", confidence: 0.1, status: "faded" }),
      ],
      new Map(),
    );
    expect(active.faded).toBe(false);
    expect(active.belowFloorCount).toBe(1);
    expect(metaLabel(active)).toBe("2 conclusions");

    // A wholly faded subject says "faded" instead — it has no floor to be under.
    const [gone] = buildSubjectRows(
      [conclusion({ id: 3, subject: "b", confidence: 0.09, status: "faded" })],
      new Map(),
    );
    expect(gone.faded).toBe(true);
    expect(gone.belowFloorCount).toBe(0);
    expect(metaLabel(gone)).toBe("1 conclusion · faded");
  });

  test("a faded subject loses its colour: every spark line goes grey", () => {
    const [active] = buildSubjectRows(
      [conclusion({ id: 1, subject: "a", confidence: 0.7 })],
      new Map(),
    );
    expect(active.spark[0].colorVar).toBe(SPARK_LEAD);

    const [gone] = buildSubjectRows(
      [conclusion({ id: 2, subject: "b", confidence: 0.1, status: "faded" })],
      new Map(),
    );
    expect(gone.spark[0].colorVar).toBe(SPARK_REST);
  });

  test("a single history point flattens to a 2-point baseline (a line must draw)", () => {
    const [row] = buildSubjectRows(
      [conclusion({ id: 1, subject: "a", confidence: 0.4 })],
      new Map([["a", new Map([[1, [0.4]]])]]),
    );
    expect(row.spark[0].points).toEqual([0.4, 0.4]);
  });

  test("real history drives the spark points", () => {
    const [row] = buildSubjectRows(
      [conclusion({ id: 1, subject: "a", confidence: 0.72 })],
      new Map([["a", new Map([[1, [0.42, 0.6, 0.72]]])]]),
    );
    expect(row.spark[0].points).toEqual([0.42, 0.6, 0.72]);
    expect(row.trend).toBe("up");
  });
});

describe("display order", () => {
  test("active by confidence desc, faded sunk to the bottom, ties by name", () => {
    const rows = [
      { subject: "zed", faded: false, topConfidence: 0.4 },
      { subject: "old", faded: true, topConfidence: 0.9 },
      { subject: "bun", faded: false, topConfidence: 0.8 },
      { subject: "alt", faded: false, topConfidence: 0.4 },
    ];
    expect(sortDisplayRows(rows).map((r) => r.subject)).toEqual([
      "bun",
      "alt",
      "zed",
      "old",
    ]);
  });

  test("displayedSubjectOrder matches the order the rows render in", () => {
    const list = [
      conclusion({ id: 1, subject: "old", confidence: 0.9, status: "faded" }),
      conclusion({ id: 2, subject: "bun", confidence: 0.8 }),
      conclusion({ id: 3, subject: "crl", confidence: 0.24 }),
    ];
    const rendered = sortDisplayRows(buildSubjectRows(list, new Map())).map(
      (r) => r.subject,
    );
    expect(displayedSubjectOrder(list)).toEqual(rendered);
  });
});
