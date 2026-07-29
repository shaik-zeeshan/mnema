// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { buildStartCaptureRequest } from "./onboarding-start-request";
import type { PermissionKey, PermissionValue } from "./onboarding-attention";

function perms(
  overrides: Partial<Record<PermissionKey, PermissionValue>> = {},
): Record<PermissionKey, PermissionValue> {
  return {
    screen: "not_determined",
    microphone: "not_determined",
    systemAudio: "not_determined",
    ...overrides,
  } as Record<PermissionKey, PermissionValue>;
}

const allOn = {
  draftCaptureScreen: true,
  draftCaptureMicrophone: true,
  draftCaptureSystemAudio: true,
};

describe("buildStartCaptureRequest", () => {
  it("asks for a source only once its permission is granted", () => {
    expect(
      buildStartCaptureRequest({
        ...allOn,
        permissions: perms({ screen: "granted", microphone: "granted" }),
      }),
    ).toEqual({ captureScreen: true, captureMicrophone: true, captureSystemAudio: true });
  });

  // The regression: a denied screen used to ride through, and
  // `start_capture_runtime` propagates that failure out of the WHOLE start —
  // taking the granted microphone down with it.
  it("drops a denied screen instead of failing the granted microphone", () => {
    expect(
      buildStartCaptureRequest({
        ...allOn,
        permissions: perms({ screen: "denied", microphone: "granted" }),
      }),
    ).toEqual({ captureScreen: false, captureMicrophone: true, captureSystemAudio: true });
  });

  // ADR 0052: system audio's grant cannot be read, so gating it on one would
  // mean never starting it.
  it("takes system audio's draft flag through unread", () => {
    expect(
      buildStartCaptureRequest({ ...allOn, permissions: null }),
    ).toEqual({ captureScreen: false, captureMicrophone: false, captureSystemAudio: true });
  });

  it("never asks for a source the user turned off", () => {
    expect(
      buildStartCaptureRequest({
        draftCaptureScreen: false,
        draftCaptureMicrophone: false,
        draftCaptureSystemAudio: false,
        permissions: perms({ screen: "granted", microphone: "granted" }),
      }),
    ).toEqual({ captureScreen: false, captureMicrophone: false, captureSystemAudio: false });
  });
});
