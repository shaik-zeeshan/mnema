// Pins the system-audio access hint rules (ADR 0052): the hint is a suspicion,
// not a verdict, so it shows only while the backend says so AND the user hasn't
// dismissed it — and a dismissal is only recordable with a prompt id.
import { describe, expect, test } from "bun:test";
import {
  canDismissSystemAudioHint,
  systemAudioHintVisible,
} from "../src/lib/settings/state/system-audio-access-logic";

describe("systemAudioHintVisible", () => {
  test("shouldShow and not dismissed -> visible", () => {
    expect(systemAudioHintVisible({ shouldShow: true }, false)).toBe(true);
  });
  test("shouldShow but dismissed -> hidden", () => {
    expect(systemAudioHintVisible({ shouldShow: true }, true)).toBe(false);
  });
  test("!shouldShow -> hidden regardless of dismissal", () => {
    expect(systemAudioHintVisible({ shouldShow: false }, false)).toBe(false);
    expect(systemAudioHintVisible({ shouldShow: false }, true)).toBe(false);
  });
  test("null hint (failed/absent probe) -> hidden", () => {
    expect(systemAudioHintVisible(null, false)).toBe(false);
    expect(systemAudioHintVisible(null, true)).toBe(false);
  });
});

describe("canDismissSystemAudioHint", () => {
  test("false when hint or promptId is missing", () => {
    expect(canDismissSystemAudioHint(null)).toBe(false);
    expect(canDismissSystemAudioHint({})).toBe(false);
    expect(canDismissSystemAudioHint({ promptId: null })).toBe(false);
    expect(canDismissSystemAudioHint({ promptId: "" })).toBe(false);
  });
  test("true when promptId is present", () => {
    expect(canDismissSystemAudioHint({ promptId: "system-audio-access" })).toBe(true);
  });
});
