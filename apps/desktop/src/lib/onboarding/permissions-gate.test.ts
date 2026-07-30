// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (log-filter.test.ts precedent).
import { describe, expect, it } from "bun:test";
import { isDenied, isGranted, screenGate } from "./permissions-gate";

describe("isGranted", () => {
  // The regression the deleted onboarding-feature-gate.test.ts owned. A machine
  // that cannot report Screen Recording must not be blocked: macOS never
  // re-prompts after a denial, so a user trapped here has no in-app recovery at
  // all — they would have to reinstall to finish onboarding.
  it("treats an unsupported permission as granted, never as a denial", () => {
    expect(isGranted("unsupported")).toBe(true);
    expect(isDenied("unsupported")).toBe(false);
  });

  it("accepts a real grant and the sticky system-audio evidence", () => {
    expect(isGranted("granted")).toBe(true);
    expect(isGranted("assumed_working")).toBe(true);
  });

  it("does not accept a denial, a restriction, or an unanswered prompt", () => {
    for (const value of ["denied", "restricted", "not_determined", undefined]) {
      expect(isGranted(value)).toBe(false);
    }
    expect(isDenied("denied")).toBe(true);
    expect(isDenied("restricted")).toBe(true);
    expect(isDenied("not_determined")).toBe(false);
  });
});

describe("screenGate", () => {
  it("passes an unsupported machine straight through", () => {
    expect(screenGate("unsupported", false)).toEqual({ ready: true, primaryLabel: "Continue" });
  });

  it("holds Continue with a named reason while the grant is missing", () => {
    expect(screenGate("denied", false)).toEqual({
      ready: false,
      primaryLabel: "Screen recording required",
    });
    expect(screenGate("not_determined", false)).toEqual({
      ready: false,
      primaryLabel: "Screen recording required",
    });
  });

  // A grant made in THIS process reads as granted while ScreenCaptureKit still
  // cannot use it. Continuing would reach the Finale only to fail there, so the
  // gate demands the relaunch first — and this is the state whose latch used to
  // be component-local, so Back → Begin setup walked straight past it.
  it("demands one relaunch when the grant appeared during this session", () => {
    expect(screenGate("granted", true)).toEqual({
      ready: false,
      primaryLabel: "Relaunch to continue",
    });
    // The relaunch requirement outranks everything, including unsupported.
    expect(screenGate("unsupported", true).ready).toBe(false);
  });
});
