// Pins the start-capture request `finishOnboarding` dispatches (extracted to
// onboarding-start-request.ts). The load-bearing rule is system audio's ADR 0052
// exemption: its grant is unreadable, so the draft flag passes straight through
// with NO permission gate and NO screen dependency — while the microphone stays
// gated on an actually-granted OS permission.
import { describe, expect, test } from "bun:test";
import {
  buildStartCaptureRequest,
  type StartCaptureRequestSource,
} from "../src/routes/onboarding/onboarding-start-request";

const source = (over: Partial<StartCaptureRequestSource> = {}): StartCaptureRequestSource => ({
  draftCaptureScreen: true,
  draftCaptureMicrophone: false,
  draftCaptureSystemAudio: false,
  permissions: { screen: "granted", microphone: "granted", systemAudio: "not_determined" },
  ...over,
});

describe("buildStartCaptureRequest", () => {
  test("system audio passes through ungated — even screen-off and not_determined", () => {
    const request = buildStartCaptureRequest(
      source({
        draftCaptureScreen: false,
        draftCaptureSystemAudio: true,
        permissions: { screen: "granted", microphone: "granted", systemAudio: "not_determined" },
      }),
    );
    expect(request.captureSystemAudio).toBe(true);
    expect(request.captureScreen).toBe(false);
  });

  test("microphone draft on but permission not granted -> false", () => {
    const denied = buildStartCaptureRequest(
      source({
        draftCaptureMicrophone: true,
        permissions: { screen: "granted", microphone: "denied", systemAudio: "not_determined" },
      }),
    );
    expect(denied.captureMicrophone).toBe(false);
    // Null permissions (never loaded) must also fail closed.
    const unloaded = buildStartCaptureRequest(
      source({ draftCaptureMicrophone: true, permissions: null }),
    );
    expect(unloaded.captureMicrophone).toBe(false);
  });

  test("microphone draft on + granted -> true", () => {
    const request = buildStartCaptureRequest(source({ draftCaptureMicrophone: true }));
    expect(request.captureMicrophone).toBe(true);
  });

  test("screen draft passes through as-is", () => {
    expect(buildStartCaptureRequest(source({ draftCaptureScreen: true })).captureScreen).toBe(true);
    expect(buildStartCaptureRequest(source({ draftCaptureScreen: false })).captureScreen).toBe(false);
  });
});
