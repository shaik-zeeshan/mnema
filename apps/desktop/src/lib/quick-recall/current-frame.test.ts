// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  quickRecallHeightForMode,
  CURRENT_FRAME_ANSWER_HEIGHT,
  CURRENT_FRAME_BAR_HEIGHT,
  CURRENT_FRAME_DISCLOSURE_HEIGHT,
} from "./current-frame";

const bar = { answerVisible: false, hasVisionNote: false };

describe("quickRecallHeightForMode", () => {
  // The regression that mattered: leaving the bar has to restore the launcher.
  // When this rule early-returned for non-frame modes, a dismiss-while-collapsed
  // left the panel pinned at 96px and the next summon showed a clipped sliver.
  test("every non-frame mode restores the full launcher", () => {
    expect(quickRecallHeightForMode("search", bar)).toBeNull();
    expect(quickRecallHeightForMode("ask", bar)).toBeNull();
    // …including while the frame state is still stale mid-teardown.
    expect(
      quickRecallHeightForMode("search", { answerVisible: true, hasVisionNote: true }),
    ).toBeNull();
  });

  test("frame mode collapses to the bar, and grows for its parts", () => {
    expect(quickRecallHeightForMode("frame", bar)).toBe(CURRENT_FRAME_BAR_HEIGHT);
    expect(
      quickRecallHeightForMode("frame", { answerVisible: false, hasVisionNote: true }),
    ).toBe(CURRENT_FRAME_BAR_HEIGHT + CURRENT_FRAME_DISCLOSURE_HEIGHT);
    expect(
      quickRecallHeightForMode("frame", { answerVisible: true, hasVisionNote: true }),
    ).toBe(CURRENT_FRAME_ANSWER_HEIGHT);
  });
});
