// Pure cross-domain validation for the recording-settings store.
//
// These are pure functions of the live draft store plus a small bundle of
// capture-support GATES the page computes from `captureSupport` (which is page
// state, not a recording draft). Keeping them pure keeps the autosave
// `blocked()` closures and the page's validation `$derived`s reading one source
// of truth, and makes the rules unit-testable without a Svelte runtime.

import type { AutosaveRecordingDomain } from "./autosave-core";

// The capture-support-derived gates the validation needs. The page owns
// `captureSupport` and derives these; they are injected so the validation stays
// free of page state.
export interface RecordingValidationGates {
  // True while support is in-flight AND a non-original resolution is selected
  // for a screen capture — saving video must wait for the lookup.
  resolutionSupportPendingForNonOriginal: boolean;
}

// The structural draft slice the validation reads. Includes the raw text inputs
// for custom resolution/bitrate (entangled with the video domain).
export interface RecordingValidationState {
  draftCaptureScreen: boolean;
  draftCaptureMicrophone: boolean;
  draftCaptureSystemAudio: boolean;
  draftSaveDirectory: string;
  draftResolutionMode: string;
  draftBitrateMode: string;
  customWidthRaw: string;
  customHeightRaw: string;
  draftCustomMbpsRaw: string;
}

export function parseCustomDimension(raw: string): number | null {
  if (!/^\d+$/.test(raw)) return null;
  const value = Number(raw);
  if (!Number.isInteger(value)) return null;
  return value;
}

export function customResolutionErrors(rec: RecordingValidationState): string[] {
  if (rec.draftResolutionMode !== "custom") return [];
  const errors: string[] = [];
  const w = parseCustomDimension(rec.customWidthRaw);
  const h = parseCustomDimension(rec.customHeightRaw);
  if (rec.customWidthRaw && w === null) errors.push("Width must be an integer.");
  if (rec.customHeightRaw && h === null) errors.push("Height must be an integer.");
  if (w != null && (w < 16 || w > 8192)) errors.push("Width must be between 16 and 8192.");
  if (h != null && (h < 16 || h > 8192)) errors.push("Height must be between 16 and 8192.");
  if (!rec.customWidthRaw || !rec.customHeightRaw)
    errors.push("Both width and height are required for custom mode.");
  return errors;
}

export function customResolutionBlocked(rec: RecordingValidationState): boolean {
  return rec.draftResolutionMode === "custom" && customResolutionErrors(rec).length > 0;
}

export function customBitrateErrors(rec: RecordingValidationState): string[] {
  if (rec.draftBitrateMode !== "custom") return [];
  const errors: string[] = [];
  if (!rec.draftCustomMbpsRaw) {
    errors.push("Custom bitrate is required (1–40 Mbps, whole number).");
  } else if (!/^\d+$/.test(rec.draftCustomMbpsRaw.trim())) {
    errors.push("Bitrate must be a whole number of Mbps (e.g. 12).");
  } else {
    const val = parseInt(rec.draftCustomMbpsRaw.trim(), 10);
    if (!Number.isInteger(val) || val <= 0) {
      errors.push("Bitrate must be a positive whole number.");
    } else if (val < 1) {
      errors.push("Bitrate must be at least 1 Mbps.");
    } else if (val > 40) {
      errors.push("Bitrate must not exceed 40 Mbps.");
    }
  }
  return errors;
}

export function customBitrateBlocked(rec: RecordingValidationState): boolean {
  return rec.draftBitrateMode === "custom" && customBitrateErrors(rec).length > 0;
}

export function recValidationErrors(
  rec: RecordingValidationState,
  gates: RecordingValidationGates,
): string[] {
  const errors: string[] = [];
  const anySource =
    rec.draftCaptureScreen || rec.draftCaptureMicrophone || rec.draftCaptureSystemAudio;
  if (!anySource) {
    errors.push(
      "At least one capture source (Screen, Microphone, or System Audio) must be enabled.",
    );
  }
  // System audio is an independent capture family with no screen dependency
  // (ADR 0052) — audio-only sessions are allowed.
  if (gates.resolutionSupportPendingForNonOriginal) {
    errors.push("Wait for capture support to load before saving preset/custom resolution.");
  }
  return errors;
}

export function recSaveBlocked(
  rec: RecordingValidationState,
  gates: RecordingValidationGates,
): boolean {
  return (
    recValidationErrors(rec, gates).length > 0 ||
    !rec.draftSaveDirectory ||
    customResolutionBlocked(rec) ||
    customBitrateBlocked(rec)
  );
}

// The per-domain save-block predicate the autosave engine consults.
export function recDomainSaveBlocked(
  domain: AutosaveRecordingDomain,
  rec: RecordingValidationState,
  gates: RecordingValidationGates,
): boolean {
  if (domain === "capture_sources") {
    // System audio has no screen dependency (ADR 0052); only "no source at all"
    // blocks the capture_sources autosave.
    return !rec.draftCaptureScreen && !rec.draftCaptureMicrophone && !rec.draftCaptureSystemAudio;
  }
  if (domain === "video") {
    return (
      gates.resolutionSupportPendingForNonOriginal ||
      customResolutionBlocked(rec) ||
      customBitrateBlocked(rec)
    );
  }
  if (domain === "storage") {
    return !rec.draftSaveDirectory;
  }
  return false;
}
