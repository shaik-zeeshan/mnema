// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  resolveRecordingPillState,
  type RecordingPillInput,
} from "./recording-pill-state";

function input(over: Partial<RecordingPillInput> = {}): RecordingPillInput {
  return {
    running: false,
    loadingStart: false,
    loadingStop: false,
    loadingSettings: false,
    userPaused: false,
    inactivityPaused: false,
    lowDiskSuspended: false,
    screenReason: null,
    hasBlockedSource: false,
    hasLostSource: false,
    ...over,
  };
}

describe("resolveRecordingPillState", () => {
  it("renders all nine states", () => {
    expect(resolveRecordingPillState(input())).toBe("idle");
    expect(resolveRecordingPillState(input({ loadingStart: true }))).toBe("starting");
    expect(resolveRecordingPillState(input({ running: true, loadingStop: true }))).toBe("stopping");
    expect(resolveRecordingPillState(input({ running: true }))).toBe("recording");
    expect(resolveRecordingPillState(input({ running: true, userPaused: true }))).toBe(
      "paused-manual",
    );
    expect(resolveRecordingPillState(input({ running: true, inactivityPaused: true }))).toBe(
      "paused-inactive",
    );
    expect(resolveRecordingPillState(input({ running: true, lowDiskSuspended: true }))).toBe(
      "low-disk",
    );
    expect(
      resolveRecordingPillState(
        input({ running: true, screenReason: "capture_display_unavailable" }),
      ),
    ).toBe("screen-asleep");
    expect(
      resolveRecordingPillState(
        input({ running: true, screenReason: "privacy_recovery_restart_required" }),
      ),
    ).toBe("degraded");
    expect(resolveRecordingPillState(input({ running: true, hasBlockedSource: true }))).toBe(
      "permission",
    );
  });

  // ADR 0040: a low-disk suspension keeps `isRunning` true, so it has to beat
  // every other running read or the pill claims to be recording while held.
  it("low disk outranks every other running state", () => {
    expect(
      resolveRecordingPillState(
        input({
          running: true,
          lowDiskSuspended: true,
          userPaused: true,
          inactivityPaused: true,
          screenReason: "capture_display_unavailable",
          hasBlockedSource: true,
          hasLostSource: true,
        }),
      ),
    ).toBe("low-disk");
  });

  // A privacy-filter suspension and a display-unavailable one both land on the
  // screen's reason; the privacy one is actionable, so it wins.
  it("privacy suspension outranks display-unavailable", () => {
    expect(
      resolveRecordingPillState(
        input({ running: true, screenReason: "privacy_filter_apply_failed" }),
      ),
    ).toBe("degraded");
  });

  it("a stop in flight beats everything, including a not-yet-running session", () => {
    expect(
      resolveRecordingPillState(input({ running: true, loadingStop: true, userPaused: true })),
    ).toBe("stopping");
  });

  it("a blocked permission is visible while idle, not just while recording", () => {
    expect(resolveRecordingPillState(input({ hasBlockedSource: true }))).toBe("permission");
  });
});
