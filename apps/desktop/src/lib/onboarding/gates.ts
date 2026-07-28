// The onboarding flow's gate predicates (issue #195, slice 5).
//
// The flow has EXACTLY TWO hard gates, both on *Capture & Storage*:
//   1. the storage path exists and is writable
//   2. the volume has room to actually run: the reserve the backend already
//      enforces + the download work-list + one day of capture
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
import { estimateDailyStorageMb } from "./disk-estimate";

/**
 * The reserve the capture pipeline already refuses to record below — mirrors
 * `RESERVE_FLOOR_BYTES` in `src-tauri/src/native_capture/disk_space.rs` (1 GiB).
 * Rust cannot be imported here; `gates.test.ts` reads that file and fails if the
 * two ever drift apart.
 */
export const RESERVE_FLOOR_BYTES = 1024 * 1024 * 1024;

/**
 * What the volume must actually hold before capture is possible: the backend's
 * reserve, the model downloads, and one day of recording at the chosen rate.
 * `estimateDailyStorageMb` is the same arithmetic the screen prints, in MB.
 */
export function storageNeedBytes(
  downloadBytes: number,
  captureIntervalSeconds: number,
): number {
  return (
    RESERVE_FLOOR_BYTES +
    downloadBytes +
    estimateDailyStorageMb(captureIntervalSeconds) * 1e6
  );
}

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
  /** `flow.downloadBytes` — what the downloads will fetch. */
  requiredBytes: number;
  /** Seconds between snapshots at the chosen capture rate (ladder stop). */
  captureIntervalSeconds: number;
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
    const need = storageNeedBytes(
      input.requiredBytes,
      input.captureIntervalSeconds,
    );
    if (probe.freeBytes !== null && probe.freeBytes < need) {
      const figures = `${formatBytes(probe.freeBytes)} free · ${formatBytes(need)} needed.`;
      // Both shortfalls quote the same total — the volume needs all of it either
      // way. Only the leading clause differs, so the screen (and a user) can
      // tell "the models don't fit" from "there is nowhere to record".
      return probe.freeBytes < RESERVE_FLOOR_BYTES + input.requiredBytes
        ? `Not enough room for the downloads. ${figures}`
        : `Not enough room to record a day of capture. ${figures}`;
    }
  }
  return input.customResolutionErrors[0] ?? input.customBitrateErrors[0] ?? null;
}
