// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  activeKeyAt,
  defaultSelectedKey,
  nextClipTurn,
  selectionIndex,
  turnAtMs,
} from "./receipt-lane";

// Minimal TurnView-shaped fixtures — only the fields the lane helpers read.
function turn(overrides) {
  return {
    key: "seg:turn",
    turnId: 0,
    audioSegmentId: 0,
    startMs: 0,
    endMs: 1000,
    speaker: "You",
    isFallback: false,
    colorVar: "--cat-communication",
    sourceKind: "microphone",
    sourceMeta: "microphone",
    text: "",
    cited: false,
    isHeadline: false,
    ...overrides,
  };
}

describe("selectionIndex — the shared selection bus", () => {
  const turns = [
    turn({ key: "a" }),
    turn({ key: "b" }),
    turn({ key: "c" }),
    turn({ key: "d" }),
  ];

  it("a reader-row key resolves to its index", () => {
    const key = turns[2].key;
    const rowIndex = turns.findIndex((t) => t.key === key);
    expect(selectionIndex(turns, key)).toBe(2);
    expect(selectionIndex(turns, key)).toBe(rowIndex);
  });

  it("returns -1 for a missing or null key", () => {
    expect(selectionIndex(turns, "nope")).toBe(-1);
    expect(selectionIndex(turns, null)).toBe(-1);
  });
});

describe("defaultSelectedKey", () => {
  it("headline wins over cited and first", () => {
    const turns = [
      turn({ key: "a" }),
      turn({ key: "b", cited: true }),
      turn({ key: "c", cited: true, isHeadline: true }),
    ];
    expect(defaultSelectedKey(turns)).toBe("c");
  });

  it("first cited when there is no headline", () => {
    const turns = [
      turn({ key: "a" }),
      turn({ key: "b", cited: true }),
      turn({ key: "c", cited: true }),
    ];
    expect(defaultSelectedKey(turns)).toBe("b");
  });

  it("first turn when nothing is cited", () => {
    const turns = [turn({ key: "a" }), turn({ key: "b" })];
    expect(defaultSelectedKey(turns)).toBe("a");
  });

  it("null for an empty set", () => {
    expect(defaultSelectedKey([])).toBeNull();
  });
});

describe("activeKeyAt — the turn under the playhead", () => {
  const turns = [
    turn({ key: "a", startMs: 1000, endMs: 2000 }),
    turn({ key: "b", startMs: 3000, endMs: 4000 }),
    turn({ key: "c", startMs: 6000, endMs: 7000 }),
  ];

  it("null before the first turn starts", () => {
    expect(activeKeyAt(turns, 500)).toBeNull();
  });

  it("the turn that contains the instant", () => {
    expect(activeKeyAt(turns, 1500)).toBe("a");
    expect(activeKeyAt(turns, 3200)).toBe("b");
    expect(activeKeyAt(turns, 6000)).toBe("c"); // inclusive of start
  });

  it("in a gap, keeps the most recently started turn lit (no flicker)", () => {
    expect(activeKeyAt(turns, 2500)).toBe("a"); // between a's end and b's start
    expect(activeKeyAt(turns, 9999)).toBe("c"); // past the last turn
  });

  it("null for a null instant or empty set", () => {
    expect(activeKeyAt(turns, null)).toBeNull();
    expect(activeKeyAt([], 1500)).toBeNull();
  });
});

describe("turnAtMs — the segment to relive on a scrub-bar click", () => {
  const turns = [
    turn({ key: "a", startMs: 1000, endMs: 2000 }),
    turn({ key: "b", startMs: 3000, endMs: 4000 }),
  ];

  it("the turn whose window contains the instant (inclusive bounds)", () => {
    expect(turnAtMs(turns, 1000)?.key).toBe("a");
    expect(turnAtMs(turns, 1500)?.key).toBe("a");
    expect(turnAtMs(turns, 2000)?.key).toBe("a");
    expect(turnAtMs(turns, 3500)?.key).toBe("b");
  });

  it("null in a gap between turns, or before/after all", () => {
    expect(turnAtMs(turns, 2500)).toBeNull(); // gap
    expect(turnAtMs(turns, 500)).toBeNull(); // before
    expect(turnAtMs(turns, 9999)).toBeNull(); // after
  });

  it("overlapping turns resolve to the earlier-starting one", () => {
    const overlap = [
      turn({ key: "mic", audioSegmentId: 1, startMs: 1000, endMs: 3000 }),
      turn({ key: "sys", audioSegmentId: 2, startMs: 2000, endMs: 4000 }),
    ];
    expect(turnAtMs(overlap, 2500)?.key).toBe("mic");
  });

  it("null for a null instant or empty set", () => {
    expect(turnAtMs(turns, null)).toBeNull();
    expect(turnAtMs([], 1500)).toBeNull();
  });
});

describe("nextClipTurn — auto-advance across segments", () => {
  // Two segments, two turns each, interleaved-by-nothing (segment 10 then 20).
  const turns = [
    turn({ key: "10:a", audioSegmentId: 10, startMs: 1000 }),
    turn({ key: "10:b", audioSegmentId: 10, startMs: 2000 }),
    turn({ key: "20:a", audioSegmentId: 20, startMs: 5000 }),
    turn({ key: "20:b", audioSegmentId: 20, startMs: 6000 }),
  ];

  it("advances to the FIRST turn of the next distinct segment", () => {
    expect(nextClipTurn(turns, 10)?.key).toBe("20:a");
  });

  it("null after the last segment (playback stops)", () => {
    expect(nextClipTurn(turns, 20)).toBeNull();
  });

  it("null for an unknown or null segment id, or empty set", () => {
    expect(nextClipTurn(turns, 999)).toBeNull();
    expect(nextClipTurn(turns, null)).toBeNull();
    expect(nextClipTurn([], 10)).toBeNull();
  });

  it("orders segments by first appearance, so overlapping mic/system play back-to-back", () => {
    const interleaved = [
      turn({ key: "mic:a", audioSegmentId: 1, startMs: 1000 }),
      turn({ key: "sys:a", audioSegmentId: 2, startMs: 1500 }),
      turn({ key: "mic:b", audioSegmentId: 1, startMs: 2000 }),
      turn({ key: "sys:b", audioSegmentId: 2, startMs: 2500 }),
    ];
    // Segment 1 (mic) finishes → next is segment 2 (system), not another mic turn.
    expect(nextClipTurn(interleaved, 1)?.key).toBe("sys:a");
    expect(nextClipTurn(interleaved, 2)).toBeNull();
  });
});
