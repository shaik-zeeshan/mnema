// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, test } from "bun:test";
import type { RuntimeSourceStatus, RuntimeSourcesStatus } from "$lib/types/inactivity";
import type { AiRuntimeStatus } from "$lib/types/recording";
import {
  engineFlag,
  formatClock,
  formatTracked,
  liveLanes,
  recView,
  statusFlag,
  stoppedReasonFrom,
  type StatusInput,
} from "./status-bar";

const src = (over: Partial<RuntimeSourceStatus> = {}): RuntimeSourceStatus => ({
  requested: true,
  paused: false,
  sessionActive: true,
  writerActive: true,
  outputPath: null,
  reason: null,
  ...over,
});
const rt = (over: Partial<RuntimeSourcesStatus> = {}): RuntimeSourcesStatus => ({
  screen: src(),
  microphone: src(),
  systemAudio: src({ requested: false }),
  ...over,
});
const base = (over: Partial<StatusInput> = {}): StatusInput => ({
  running: false,
  loadingStart: false,
  userPaused: false,
  inactivityPaused: false,
  lowDisk: false,
  runtime: null,
  selected: { screen: true, microphone: true, systemAudio: false },
  permissions: { screen: "granted", microphone: "granted", systemAudio: "assumed_working" },
  startError: null,
  readOnly: false,
  stoppedReason: null,
  ...over,
});

describe("recView", () => {
  test("idle only when every requested source idles", () => {
    const oneIdle = base({ running: true, inactivityPaused: true, runtime: rt({ screen: src({ paused: true }) }) });
    expect(recView(oneIdle).label).toBe("REC");
    expect(liveLanes(oneIdle)[0]).toEqual({ key: "screen", mod: "paused", small: "idle" });
    const allIdle = base({
      running: true,
      inactivityPaused: true,
      runtime: rt({ screen: src({ paused: true }), microphone: src({ paused: true }) }),
    });
    expect(recView(allIdle)).toEqual({ mod: "paused", label: "PAUSED · idle", clock: null });
  });

  test("user pause and low disk", () => {
    expect(recView(base({ running: true, userPaused: true })).clock).toBe("paused");
    expect(recView(base({ running: true, lowDisk: true })).mod).toBe("suspended");
    expect(recView(base({ loadingStart: true })).label).toBe("STARTING…");
    expect(recView(base({ stoppedReason: "disk full" })).label).toBe("STOPPED · disk full");
  });
});

describe("statusFlag — at most one, by precedence", () => {
  test("privacy restart beats retry beats low disk", () => {
    const retry = rt({ screen: src({ reason: "privacy_filter_apply_failed" }) });
    expect(statusFlag(base({ running: true, lowDisk: true, runtime: retry }))?.kind).toBe("privacy-retry");
    const restart = rt({
      screen: src({ reason: "privacy_filter_apply_failed" }),
      systemAudio: src({ reason: "privacy_recovery_restart_required" }),
    });
    const flag = statusFlag(base({ running: true, runtime: restart }));
    expect(flag?.kind).toBe("privacy-restart");
    expect(flag?.text).toBe("restart to resume screen and system audio ·");
    expect(statusFlag(base({ running: true, lowDisk: true, runtime: rt() }))?.kind).toBe("low-disk");
    expect(statusFlag(base({ running: true, runtime: rt() }))).toBeNull();
  });

  test("not recording: start failure > read-only > stopped > permission", () => {
    const denied = { screen: "denied", microphone: "granted", systemAudio: "unknown" } as const;
    const all = base({ startError: "boom", readOnly: true, stoppedReason: "disk full", permissions: denied });
    expect(statusFlag(all)?.kind).toBe("start-failed");
    expect(statusFlag({ ...all, startError: null })?.kind).toBe("read-only");
    expect(statusFlag({ ...all, startError: null, readOnly: false })?.kind).toBe("stopped");
    expect(statusFlag(base({ permissions: denied }))?.kind).toBe("permission");
    // Screen not selected → its permission doesn't matter for the next recording.
    expect(statusFlag(base({ permissions: denied, selected: { screen: false, microphone: true, systemAudio: false } }))).toBeNull();
  });
});

describe("engineFlag", () => {
  const status = (over: Partial<AiRuntimeStatus>): AiRuntimeStatus => ({
    enabled: true,
    hasProviders: true,
    configured: true,
    available: true,
    reason: null,
    ...over,
  });
  test("healthy, off, and unchecked show nothing", () => {
    expect(engineFlag(status({}), null)).toBeNull();
    expect(engineFlag(status({ enabled: false, available: false, reason: "ai_runtime_disabled" }), null)).toBeNull();
    expect(engineFlag(null, null)).toBeNull();
  });
  test("unreachable reads offline, never reconnect (ADR 0058)", () => {
    const flag = engineFlag(status({ available: false, reason: "provider_unreachable:chatgpt" }), null);
    expect(flag?.label).toBe("offline");
    expect(flag?.error).toBe(true);
    expect(engineFlag(status({ available: false, reason: "needs_reconnect:chatgpt" }), null)?.label).toBe("needs reconnect");
    expect(engineFlag(status({ hasProviders: false, available: false, reason: "no_providers" }), null)?.label).toBe("not set up");
  });
});

test("stopped notification is history once a session started after it", () => {
  const n = [{ id: "capture_disk_full_stopped", createdAtUnixMs: 1_000 }];
  expect(stoppedReasonFrom(n, null)).toBe("disk full");
  expect(stoppedReasonFrom(n, 500)).toBe("disk full");
  expect(stoppedReasonFrom(n, 2_000)).toBeNull();
  expect(stoppedReasonFrom([{ id: "capture_low_disk", createdAtUnixMs: 1 }], null)).toBeNull();
});

test("formatters", () => {
  expect(formatTracked(4 * 3_600_000 + 41 * 60_000 + 59_000)).toBe("4h 41m");
  expect(formatTracked(12 * 60_000)).toBe("12m");
  expect(formatClock(5_947_000, true)).toBe("01:39:07");
  expect(formatClock(12_000, false)).toBe("00:12");
});
