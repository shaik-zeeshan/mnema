// The onboarding flow's gate predicates (issue #195, slice 5).
//
// The flow has EXACTLY TWO hard gates, both on *Capture & Storage*:
//   1. the storage path exists and is writable
//   2. the volume has room for the download work-list (`workListBytes`)
// plus the pre-existing custom resolution (16–8192 px) / bitrate (1–40 Mbps)
// range validation, which blocks because those values serialize as `null` and
// break the backend save.
//
// Nothing else gates. Permissions gates nothing (macOS never re-prompts after a
// denial, so a hard gate there would trap the user with no in-app recovery).
// Setup gates NOTHING, EVER — its Continue is live on arrival and never
// disables. The old "attention item" concept, where a selected-but-missing model
// blocked finishing, is gone; there is deliberately no `canFinish` here.
//
// Pure: no Svelte, no `invoke`. `formatBytes` is the app's existing formatter.
import { formatBytes } from "../settings/state/format";

/**
 * What the *Capture & Storage* screen measured about the chosen save directory.
 * `freeBytes` is `null` when the volume could not be read.
 */
export interface StorageProbe {
  exists: boolean;
  writable: boolean;
  freeBytes: number | null;
}

export interface CaptureStorageGateInput {
  /** `null` until the screen has probed. An unmeasured path never blocks. */
  probe: StorageProbe | null;
  /** `workListBytes(resolved.workList)` — what the downloads will fetch. */
  requiredBytes: number;
  /** From `onboarding-attention.ts`, unchanged. */
  customResolutionErrors: readonly string[];
  customBitrateErrors: readonly string[];
}

/**
 * The one blocking reason to render on *Capture & Storage*, or `null` when the
 * screen is free to continue.
 *
 * An inability to MEASURE never blocks (a null probe, or a null `freeBytes`):
 * same discipline as the capture pipeline's low-disk preflight (ADR 0040) —
 * only a measured shortfall acts.
 */
export function captureStorageBlockReason(
  input: CaptureStorageGateInput,
): string | null {
  const probe = input.probe;
  if (probe) {
    if (!probe.exists) {
      return "That folder doesn't exist. Choose another folder.";
    }
    if (!probe.writable) {
      return "That folder is not writable. Choose another folder.";
    }
    if (probe.freeBytes !== null && probe.freeBytes < input.requiredBytes) {
      return `Not enough room for the downloads. ${formatBytes(probe.freeBytes)} free · ${formatBytes(input.requiredBytes)} needed.`;
    }
  }
  return input.customResolutionErrors[0] ?? input.customBitrateErrors[0] ?? null;
}
