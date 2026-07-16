// Pure construction of the `start_native_capture` request that `finishOnboarding`
// dispatches. Lives in its own dependency-free file (onboarding-lifecycle.ts
// imports `$app/navigation`/`invoke` at module level, so bun:test can't load it).
//
// Defense-in-depth: never request a source whose OS permission isn't granted,
// independent of the attention gate. Capture must not outrun authorization even
// if the gating logic ever changes. System audio is exempt and takes its draft
// flag straight through: its grant is unreadable (ADR 0052), the prompt fires
// when the tap is first read, and it has no screen dependency — gating it here
// would never let it start.
import type { PermissionKey, PermissionValue } from "./onboarding-attention";

export interface StartCaptureRequestSource {
  draftCaptureScreen: boolean;
  draftCaptureMicrophone: boolean;
  draftCaptureSystemAudio: boolean;
  permissions: Record<PermissionKey, PermissionValue> | null;
}

export function buildStartCaptureRequest(target: StartCaptureRequestSource): {
  captureScreen: boolean;
  captureMicrophone: boolean;
  captureSystemAudio: boolean;
} {
  return {
    captureScreen: target.draftCaptureScreen,
    captureMicrophone:
      target.draftCaptureMicrophone && target.permissions?.microphone === "granted",
    captureSystemAudio: target.draftCaptureSystemAudio,
  };
}
