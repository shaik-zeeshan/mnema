// Which of the state pill's states is showing. Pure so the precedence — the
// one branchy part of RecordingPill.svelte — is testable without a DOM.
//
// The order matters and mirrors the backend's own: a low-disk hold keeps the
// session "running" and must outrank the generic paused/recording read
// (ADR 0040), and display-unavailable is screen-only liveness that leaves mic
// and system audio recording (ADR 0021).
export type RecordingPillState =
  | "idle"
  | "starting"
  | "stopping"
  | "recording"
  | "paused-manual"
  | "paused-inactive"
  | "low-disk"
  | "screen-asleep"
  | "degraded"
  | "permission";

export interface RecordingPillInput {
  running: boolean;
  loadingStart: boolean;
  loadingStop: boolean;
  loadingSettings: boolean;
  userPaused: boolean;
  inactivityPaused: boolean;
  lowDiskSuspended: boolean;
  /** `runtimeSources.screen.reason` — the only source that carries suspensions. */
  screenReason: string | null;
  /** A selected source whose TCC answer is a hard denial. */
  hasBlockedSource: boolean;
  /** A requested source whose session died after the start-up grace period. */
  hasLostSource: boolean;
}

export function isPrivacySuspension(screenReason: string | null): boolean {
  return (
    screenReason === "privacy_filter_apply_failed" ||
    screenReason === "privacy_recovery_restart_required"
  );
}

export function resolveRecordingPillState(input: RecordingPillInput): RecordingPillState {
  if (input.loadingStop) return "stopping";
  if (!input.running) {
    if (input.loadingStart || input.loadingSettings) return "starting";
    return input.hasBlockedSource ? "permission" : "idle";
  }
  if (input.lowDiskSuspended) return "low-disk";
  if (input.userPaused) return "paused-manual";
  if (isPrivacySuspension(input.screenReason)) return "degraded";
  if (input.screenReason === "capture_display_unavailable") return "screen-asleep";
  if (input.hasBlockedSource) return "permission";
  if (input.inactivityPaused) return "paused-inactive";
  if (input.hasLostSource) return "degraded";
  return "recording";
}
