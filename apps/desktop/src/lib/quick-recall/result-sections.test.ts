// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  AUDIO_VISIBLE_CAP,
  FRAME_VISIBLE_CAP,
  moreRowLabel,
  remapSelection,
  visibleCount,
} from "./result-sections";

describe("visibleCount", () => {
  test("below the cap renders everything", () => {
    expect(visibleCount(5, FRAME_VISIBLE_CAP, false)).toBe(5);
  });

  test("at the cap renders everything", () => {
    expect(visibleCount(8, FRAME_VISIBLE_CAP, false)).toBe(8);
  });

  test("above the cap collapsed renders the cap", () => {
    expect(visibleCount(24, FRAME_VISIBLE_CAP, false)).toBe(8);
    expect(visibleCount(12, AUDIO_VISIBLE_CAP, false)).toBe(3);
  });

  test("above the cap expanded renders everything", () => {
    expect(visibleCount(24, FRAME_VISIBLE_CAP, true)).toBe(24);
  });

  test("empty section renders nothing either way", () => {
    expect(visibleCount(0, FRAME_VISIBLE_CAP, false)).toBe(0);
    expect(visibleCount(0, FRAME_VISIBLE_CAP, true)).toBe(0);
  });
});

describe("moreRowLabel", () => {
  test("no row when the section fits its cap", () => {
    expect(moreRowLabel(8, FRAME_VISIBLE_CAP, false, "screen")).toBeNull();
    expect(moreRowLabel(0, AUDIO_VISIBLE_CAP, false, "audio")).toBeNull();
  });

  test("collapsed overflow shows the hidden count (mockup wording)", () => {
    expect(moreRowLabel(24, FRAME_VISIBLE_CAP, false, "screen")).toBe(
      "↓ show 16 more screen results",
    );
    expect(moreRowLabel(4, AUDIO_VISIBLE_CAP, false, "audio")).toBe(
      "↓ show 1 more audio results",
    );
  });

  test("expanded overflow shows the collapse toggle", () => {
    expect(moreRowLabel(24, FRAME_VISIBLE_CAP, true, "screen")).toBe("↑ show less");
  });
});

describe("remapSelection", () => {
  // Index space: visible frames first, then visible audio.
  test("no selection stays unselected", () => {
    expect(remapSelection(-1, { frames: 24, audio: 3 }, { frames: 8, audio: 3 })).toBe(-1);
  });

  test("everything gone yields -1", () => {
    expect(remapSelection(2, { frames: 8, audio: 3 }, { frames: 0, audio: 0 })).toBe(-1);
  });

  test("still-visible frame keeps its position", () => {
    expect(remapSelection(5, { frames: 24, audio: 3 }, { frames: 8, audio: 3 })).toBe(5);
  });

  test("frame hidden by collapse clamps to the last visible frame", () => {
    expect(remapSelection(20, { frames: 24, audio: 3 }, { frames: 8, audio: 3 })).toBe(7);
  });

  test("frame section emptying clamps to the first remaining row", () => {
    expect(remapSelection(3, { frames: 8, audio: 3 }, { frames: 0, audio: 3 })).toBe(0);
  });

  test("audio selection survives a frame-section collapse (index shifts)", () => {
    // Audio position 1: index 24+1 → 8+1.
    expect(remapSelection(25, { frames: 24, audio: 3 }, { frames: 8, audio: 3 })).toBe(9);
  });

  test("audio selection survives an audio expand", () => {
    expect(remapSelection(8 + 2, { frames: 8, audio: 3 }, { frames: 8, audio: 12 })).toBe(10);
  });

  test("audio hidden by collapse clamps to the last visible row", () => {
    expect(remapSelection(8 + 10, { frames: 8, audio: 12 }, { frames: 8, audio: 3 })).toBe(10);
  });

  test("audio section emptying clamps to the last visible frame", () => {
    expect(remapSelection(8 + 1, { frames: 8, audio: 3 }, { frames: 8, audio: 0 })).toBe(7);
  });

  test("out-of-range selection clamps into the new space", () => {
    expect(remapSelection(50, { frames: 8, audio: 3 }, { frames: 8, audio: 3 })).toBe(10);
  });
});
