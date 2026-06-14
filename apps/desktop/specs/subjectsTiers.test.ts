import { describe, it, expect, jest } from "bun:test";
import {
  DISPLAY_FLOOR,
  INITIAL_BASE,
  STRONGLY_HELD,
  SPARSE_LIMIT,
  isSparse,
  convictionTierId,
  movementTierId,
  tierFor,
  deriveTrend,
  buildTiers,
  summaryCounts,
  subjectsDiff,
  decideRefresh,
  debounce,
  type TierSubject,
  type Trend,
} from "../src/lib/insights/subjectsTiers";

function subject(over: Partial<TierSubject> = {}): TierSubject {
  return {
    topConfidence: 0.5,
    faded: false,
    trend: "steady",
    lastMovedAtMs: 0,
    ...over,
  };
}

describe("threshold constants", () => {
  it("mirror the engine policy values", () => {
    expect(DISPLAY_FLOOR).toBe(0.15);
    expect(INITIAL_BASE).toBe(0.3);
    expect(STRONGLY_HELD).toBe(0.68);
    expect(SPARSE_LIMIT).toBe(5);
  });
});

describe("isSparse", () => {
  it("is true below the limit and false at/above it (4-vs-5 boundary)", () => {
    expect(isSparse(0)).toBe(true);
    expect(isSparse(4)).toBe(true);
    expect(isSparse(5)).toBe(false);
    expect(isSparse(6)).toBe(false);
  });
});

describe("convictionTierId", () => {
  it("classifies by topConfidence at the boundaries", () => {
    // At/above STRONGLY_HELD (0.68) => strong.
    expect(convictionTierId(subject({ topConfidence: 0.68 }))).toBe("strong");
    expect(convictionTierId(subject({ topConfidence: 0.9 }))).toBe("strong");
    // Just below STRONGLY_HELD => forming.
    expect(convictionTierId(subject({ topConfidence: 0.679 }))).toBe("forming");
    // At INITIAL_BASE (0.30) => forming (>= INITIAL_BASE).
    expect(convictionTierId(subject({ topConfidence: 0.3 }))).toBe("forming");
    // Just below INITIAL_BASE => shaping.
    expect(convictionTierId(subject({ topConfidence: 0.299 }))).toBe("shaping");
    // At DISPLAY_FLOOR (0.15), not faded => still shaping (< INITIAL_BASE).
    expect(convictionTierId(subject({ topConfidence: 0.15 }))).toBe("shaping");
    expect(convictionTierId(subject({ topConfidence: 0 }))).toBe("shaping");
  });

  it("faded wins over confidence", () => {
    expect(
      convictionTierId(subject({ faded: true, topConfidence: 0.99 })),
    ).toBe("fading");
    expect(
      convictionTierId(subject({ faded: true, topConfidence: 0.05 })),
    ).toBe("fading");
  });
});

describe("movementTierId", () => {
  it("maps trend to tier when not faded", () => {
    expect(movementTierId(subject({ trend: "up" }))).toBe("warming");
    expect(movementTierId(subject({ trend: "steady" }))).toBe("steady");
    expect(movementTierId(subject({ trend: "down" }))).toBe("cooling");
  });

  it("faded wins over trend", () => {
    expect(movementTierId(subject({ faded: true, trend: "up" }))).toBe(
      "fading",
    );
    expect(movementTierId(subject({ faded: true, trend: "down" }))).toBe(
      "fading",
    );
  });
});

describe("tierFor", () => {
  it("dispatches to the axis-specific helper", () => {
    const s = subject({ topConfidence: 0.7, trend: "down" });
    expect(tierFor(s, "conviction")).toBe("strong");
    expect(tierFor(s, "movement")).toBe("cooling");
  });
});

describe("deriveTrend", () => {
  const cs = [{ id: 1, status: "visible" }];

  it("reads rising 2-point history as up", () => {
    const history = new Map<number, number[]>([[1, [0.3, 0.5]]]);
    expect(deriveTrend(cs, history)).toBe("up");
  });

  it("reads falling 2-point history as down", () => {
    const history = new Map<number, number[]>([[1, [0.5, 0.3]]]);
    expect(deriveTrend(cs, history)).toBe("down");
  });

  it("reads flat history as steady", () => {
    const history = new Map<number, number[]>([[1, [0.4, 0.4]]]);
    expect(deriveTrend(cs, history)).toBe("steady");
  });

  it("treats deltas within +/-0.04 as steady", () => {
    expect(deriveTrend(cs, new Map([[1, [0.4, 0.44]]]))).toBe("steady"); // exactly +0.04, not > 0.04
    expect(deriveTrend(cs, new Map([[1, [0.44, 0.4]]]))).toBe("steady"); // exactly -0.04, not < -0.04
  });

  it("averages across measured conclusions", () => {
    const two = [
      { id: 1, status: "visible" },
      { id: 2, status: "visible" },
    ];
    // +0.20 and -0.02 => avg +0.09 => up.
    const up = new Map<number, number[]>([
      [1, [0.3, 0.5]],
      [2, [0.5, 0.48]],
    ]);
    expect(deriveTrend(two, up)).toBe("up");
    // +0.20 and -0.30 => avg -0.05 => down.
    const down = new Map<number, number[]>([
      [1, [0.3, 0.5]],
      [2, [0.6, 0.3]],
    ]);
    expect(deriveTrend(two, down)).toBe("down");
  });

  it("ignores conclusions with fewer than 2 history points", () => {
    // Only one point => unmeasured; falls back to status (visible => steady).
    expect(deriveTrend(cs, new Map([[1, [0.5]]]))).toBe("steady");
  });

  it("falls back to status when no history is available", () => {
    expect(deriveTrend([{ id: 1, status: "visible" }], undefined)).toBe(
      "steady",
    );
    expect(deriveTrend([{ id: 1, status: "faded" }], undefined)).toBe("down");
    expect(deriveTrend([{ id: 1, status: "faded" }], new Map())).toBe("down");
    // Mixed statuses: not ALL faded => steady.
    expect(
      deriveTrend(
        [
          { id: 1, status: "faded" },
          { id: 2, status: "visible" },
        ],
        undefined,
      ),
    ).toBe("steady");
  });
});

describe("buildTiers", () => {
  function row(id: string, over: Partial<TierSubject> = {}) {
    return { id, ...subject(over) };
  }

  it("returns conviction tiers in top->bottom order, sorted by confidence desc", () => {
    const rows = [
      row("a", { topConfidence: 0.4 }), // forming
      row("b", { topConfidence: 0.9 }), // strong
      row("c", { topConfidence: 0.2 }), // shaping
      row("d", { topConfidence: 0.5 }), // forming
      row("e", { faded: true, topConfidence: 0.99 }), // fading
    ];
    const tiers = buildTiers(rows, "conviction");
    expect(tiers.map((t) => t.id)).toEqual([
      "strong",
      "forming",
      "shaping",
      "fading",
    ]);
    expect(tiers[0].items.map((i) => i.id)).toEqual(["b"]);
    // forming sorted by topConfidence desc: d (0.5) before a (0.4).
    expect(tiers[1].items.map((i) => i.id)).toEqual(["d", "a"]);
    expect(tiers[2].items.map((i) => i.id)).toEqual(["c"]);
    expect(tiers[3].items.map((i) => i.id)).toEqual(["e"]);
    // Only the fading tier is marked faded.
    expect(tiers.map((t) => t.faded)).toEqual([false, false, false, true]);
    expect(tiers[3].title).toBe("Fading · kept for history");
    expect(tiers[3].note).toBe("below display floor");
  });

  it("returns movement tiers in top->bottom order, sorted by lastMovedAtMs desc", () => {
    const rows = [
      row("a", { trend: "steady", lastMovedAtMs: 10 }),
      row("b", { trend: "up", lastMovedAtMs: 30 }),
      row("c", { trend: "down", lastMovedAtMs: 20 }),
      row("d", { trend: "up", lastMovedAtMs: 40 }),
      row("e", { faded: true, trend: "up", lastMovedAtMs: 99 }),
    ];
    const tiers = buildTiers(rows, "movement");
    expect(tiers.map((t) => t.id)).toEqual([
      "warming",
      "steady",
      "cooling",
      "fading",
    ]);
    // warming sorted by lastMovedAtMs desc: d (40) before b (30).
    expect(tiers[0].items.map((i) => i.id)).toEqual(["d", "b"]);
    expect(tiers[1].items.map((i) => i.id)).toEqual(["a"]);
    expect(tiers[2].items.map((i) => i.id)).toEqual(["c"]);
    expect(tiers[3].items.map((i) => i.id)).toEqual(["e"]);
  });

  it("always includes every tier, even when empty", () => {
    const tiers = buildTiers([], "conviction");
    expect(tiers).toHaveLength(4);
    expect(tiers.every((t) => t.items.length === 0)).toBe(true);
  });

  it("places a single all-faded subject only in the fading tier", () => {
    const only = row("x", { faded: true, topConfidence: 0.8, trend: "up" });
    for (const axis of ["conviction", "movement"] as const) {
      const tiers = buildTiers([only], axis);
      for (const t of tiers) {
        if (t.id === "fading") expect(t.items.map((i) => i.id)).toEqual(["x"]);
        else expect(t.items).toHaveLength(0);
      }
    }
  });

  it("does not mutate the input array", () => {
    const rows = [
      row("a", { topConfidence: 0.1 }),
      row("b", { topConfidence: 0.9 }),
    ];
    const snapshot = rows.map((r) => r.id);
    buildTiers(rows, "conviction");
    expect(rows.map((r) => r.id)).toEqual(snapshot);
  });
});

describe("summaryCounts", () => {
  it("partitions a mixed set: active vs fading, with trend within active", () => {
    expect(
      summaryCounts([
        { faded: false, trend: "up" },
        { faded: false, trend: "up" },
        { faded: false, trend: "steady" },
        { faded: false, trend: "down" },
        { faded: true, trend: "up" }, // faded never counts toward warming
        { faded: true, trend: "down" },
      ]),
    ).toEqual({ active: 4, fading: 2, warming: 2, steady: 1, cooling: 1 });
  });

  it("counts an all-faded set with zero active movement", () => {
    expect(
      summaryCounts([
        { faded: true, trend: "up" },
        { faded: true, trend: "steady" },
      ]),
    ).toEqual({ active: 0, fading: 2, warming: 0, steady: 0, cooling: 0 });
  });

  it("returns all zeros for an empty set", () => {
    expect(summaryCounts([])).toEqual({
      active: 0,
      fading: 0,
      warming: 0,
      steady: 0,
      cooling: 0,
    });
  });
});

describe("subjectsDiff", () => {
  it("reports no change for identical lists", () => {
    expect(subjectsDiff(["a", "b", "c"], ["a", "b", "c"])).toEqual({
      changed: false,
      count: 0,
    });
  });

  it("counts a single added subject", () => {
    expect(subjectsDiff(["a", "b"], ["a", "b", "c"])).toEqual({
      changed: true,
      count: 1,
    });
  });

  it("counts a single removed subject", () => {
    expect(subjectsDiff(["a", "b", "c"], ["a", "b"])).toEqual({
      changed: true,
      count: 1,
    });
  });

  it("counts the symmetric difference when one added and one removed", () => {
    // 'c' removed, 'd' added => symmetric difference of 2.
    expect(subjectsDiff(["a", "b", "c"], ["a", "b", "d"])).toEqual({
      changed: true,
      count: 2,
    });
  });

  it("flags a pure reorder as changed with count 0", () => {
    expect(subjectsDiff(["a", "b", "c"], ["c", "b", "a"])).toEqual({
      changed: true,
      count: 0,
    });
  });
});

describe("decideRefresh", () => {
  it("ignores when nothing changed", () => {
    expect(
      decideRefresh({ changed: false, expanded: false, atTop: true }),
    ).toBe("ignore");
    expect(
      decideRefresh({ changed: false, expanded: true, atTop: false }),
    ).toBe("ignore");
  });

  it("applies when changed and no row open and at the top", () => {
    expect(
      decideRefresh({ changed: true, expanded: false, atTop: true }),
    ).toBe("apply");
  });

  it("stages when changed and a row is expanded", () => {
    expect(
      decideRefresh({ changed: true, expanded: true, atTop: true }),
    ).toBe("stage");
  });

  it("stages when changed and not at the top", () => {
    expect(
      decideRefresh({ changed: true, expanded: false, atTop: false }),
    ).toBe("stage");
  });
});

describe("debounce", () => {
  it("collapses rapid calls into one trailing call with the last args", () => {
    jest.useFakeTimers();
    try {
      const fn = jest.fn();
      const debounced = debounce(fn, 500);
      debounced("a");
      debounced("b");
      debounced("c");
      expect(fn).not.toHaveBeenCalled();
      jest.advanceTimersByTime(500);
      expect(fn).toHaveBeenCalledTimes(1);
      expect(fn).toHaveBeenCalledWith("c");
    } finally {
      jest.useRealTimers();
    }
  });

  it("cancel() prevents a pending call", () => {
    jest.useFakeTimers();
    try {
      const fn = jest.fn();
      const debounced = debounce(fn, 500);
      debounced("x");
      debounced.cancel();
      jest.advanceTimersByTime(500);
      expect(fn).not.toHaveBeenCalled();
    } finally {
      jest.useRealTimers();
    }
  });
});

// Type-level sanity: a SubjectRow-like object with extra fields satisfies the
// generic constraint and is preserved through buildTiers.
describe("type plumbing", () => {
  it("preserves extra fields on T through buildTiers", () => {
    interface Row extends TierSubject {
      subject: string;
    }
    const rows: Row[] = [
      { subject: "rust", topConfidence: 0.9, faded: false, trend: "up" as Trend, lastMovedAtMs: 1 },
    ];
    const tiers = buildTiers(rows, "conviction");
    expect(tiers[0].items[0].subject).toBe("rust");
  });
});
