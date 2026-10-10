// What the status bar shows (SHELL.md › Status bar), as pure functions over the
// capture / license / engine state the shell already holds. StatusBar.svelte only
// renders these; the one-flag precedence and the lane rules are tested here.

import type { PermissionsMap, RecordingSettings } from "$lib/types";
import type { AiRuntimeStatus } from "$lib/types/recording";
import type { RuntimeSourcesStatus } from "$lib/types/inactivity";
import { engineState } from "$lib/insights/engine-state";

export type LaneKey = "screen" | "microphone" | "systemAudio";
export const LANES: { key: LaneKey; label: string; mod: string }[] = [
  { key: "screen", label: "screen", mod: "screen" },
  { key: "microphone", label: "mic", mod: "mic" },
  { key: "systemAudio", label: "system", mod: "sysaudio" },
];

export interface StatusInput {
  running: boolean;
  loadingStart: boolean;
  userPaused: boolean;
  inactivityPaused: boolean;
  lowDisk: boolean;
  runtime: RuntimeSourcesStatus | null;
  /** Sources chosen for the next / current session (`captureScreen` …). */
  selected: Record<LaneKey, boolean>;
  permissions: PermissionsMap | null;
  startError: string | null;
  /** The license blocks new recording (read-only / revoked). */
  readOnly: boolean;
  /** Short reason from a "Recording stopped — …" notification, e.g. "disk full". */
  stoppedReason: string | null;
}

export type RecMod = "" | "starting" | "running" | "paused" | "suspended" | "stopped";
export interface RecView {
  mod: RecMod;
  label: string;
  /** Which clock follows the label: session elapsed, time since the user paused, or none. */
  clock: "elapsed" | "paused" | null;
}

const isPrivacyReason = (reason: string | null) =>
  reason === "privacy_filter_apply_failed" || reason === "privacy_recovery_restart_required";

/** Every requested source is idle — only then is the whole session "PAUSED · idle"
 *  (`isInactivityPaused` is set as soon as ANY source idles). */
function allRequestedIdle(runtime: RuntimeSourcesStatus | null): boolean {
  if (!runtime) return true;
  const requested = LANES.map(({ key }) => runtime[key]).filter((s) => s.requested);
  return requested.length > 0 && requested.every((s) => s.paused);
}

export function recView(i: StatusInput): RecView {
  if (!i.running) {
    if (i.loadingStart) return { mod: "starting", label: "STARTING…", clock: null };
    if (i.stoppedReason) return { mod: "stopped", label: `STOPPED · ${i.stoppedReason}`, clock: null };
    return { mod: "", label: "NOT RECORDING", clock: null };
  }
  if (i.lowDisk && !i.userPaused) return { mod: "suspended", label: "PAUSED · low disk", clock: null };
  if (i.userPaused) return { mod: "paused", label: "PAUSED", clock: "paused" };
  if (i.inactivityPaused && allRequestedIdle(i.runtime)) {
    return { mod: "paused", label: "PAUSED · idle", clock: null };
  }
  return { mod: "running", label: "REC", clock: "elapsed" };
}

export type LaneMod = "on" | "off" | "paused" | "starting" | "error";
export interface LiveLane {
  key: LaneKey;
  mod: LaneMod;
  small: string | null;
}

/** Display-only lanes while starting / recording (toggles are rendered while stopped). */
export function liveLanes(i: StatusInput, rec: RecView = recView(i)): LiveLane[] {
  return LANES.map(({ key }) => {
    if (!i.running || !i.runtime) {
      return { key, mod: i.selected[key] ? "starting" : "off", small: null };
    }
    const src = i.runtime[key];
    if (!src.requested) return { key, mod: "off", small: null };
    if (isPrivacyReason(src.reason)) return { key, mod: "error", small: "stopped" };
    if (src.paused) return { key, mod: "paused", small: rec.mod === "running" ? "idle" : null };
    if (src.sessionActive && src.writerActive) return { key, mod: "on", small: null };
    return { key, mod: "starting", small: null };
  });
}

/** Screen permission is refused outright (macOS won't re-prompt) — the lane is blocked. */
export function screenPermissionMissing(i: StatusInput): boolean {
  const screen = i.permissions?.screen;
  return !i.running && i.selected.screen && (screen === "denied" || screen === "restricted");
}

export type FlagKind =
  | "privacy-restart"
  | "privacy-retry"
  | "low-disk"
  | "start-failed"
  | "read-only"
  | "stopped"
  | "permission";
export interface StatusFlag {
  kind: FlagKind;
  tone: "warn" | "danger";
  /** Plain lead text (may be empty); `action` renders bold after it. */
  text: string;
  action: string;
  tip: string | null;
}

function privacySources(runtime: RuntimeSourcesStatus): string[] {
  return (["screen", "systemAudio"] as const)
    .filter((key) => runtime[key].requested && isPrivacyReason(runtime[key].reason))
    .map((key) => (key === "systemAudio" ? "system audio" : key));
}

const joinList = (items: string[]) =>
  items.length <= 1 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;

/** At most ONE flag. Recording: privacy restart > privacy retry > low disk.
 *  Not recording: start failure > read-only > stopped by the system > permission. */
export function statusFlag(i: StatusInput): StatusFlag | null {
  if (i.running) {
    const rt = i.runtime;
    const sources = rt ? privacySources(rt) : [];
    if (rt && sources.length > 0) {
      const names = joinList(sources);
      const mic = rt.microphone.requested ? " Microphone can keep recording." : "";
      const restart = (["screen", "systemAudio"] as const).some(
        (key) => rt[key].reason === "privacy_recovery_restart_required",
      );
      return restart
        ? {
            kind: "privacy-restart",
            tone: "danger",
            text: `restart to resume ${names} ·`,
            action: "Restart",
            tip: `Privacy filter recovery failed. Stop and start recording to resume ${names}.${mic}`,
          }
        : {
            kind: "privacy-retry",
            tone: "warn",
            text: "privacy filter failed ·",
            action: "retrying",
            tip: `Mnema stopped ${names} because the privacy filter could not be applied. It is retrying recovery.${mic}`,
          };
    }
    if (i.lowDisk) {
      return { kind: "low-disk", tone: "warn", text: "free up space ·", action: "resumes on its own", tip: null };
    }
    return null;
  }
  if (i.startError) {
    return { kind: "start-failed", tone: "danger", text: "couldn't start ·", action: "Retry", tip: i.startError };
  }
  if (i.readOnly) return { kind: "read-only", tone: "danger", text: "read-only ·", action: "buy a license", tip: null };
  if (i.stoppedReason === "disk full") {
    return { kind: "stopped", tone: "danger", text: "", action: "Open Storage", tip: null };
  }
  if (screenPermissionMissing(i)) {
    return { kind: "permission", tone: "warn", text: "Screen Recording off ·", action: "Open System Settings", tip: null };
  }
  return null;
}

/** The engine pill: nothing while healthy, off by choice, or not yet checked. */
export function engineFlag(
  status: AiRuntimeStatus | null,
  settings: RecordingSettings["aiRuntime"] | null,
): { label: string; error: boolean; tip: string } | null {
  if (!status) return null;
  const state = engineState(true, status, settings);
  switch (state.kind) {
    case "pitch":
      return { label: "not set up", error: false, tip: "No AI provider set up — Insights shows capture-only views" };
    case "unreachable":
      // ADR 0058: an unreachable provider is "offline", never "reconnect".
      return {
        label: status.reason === "vault_denied" ? "keychain locked" : "offline",
        error: true,
        tip: `${state.text}${status.reason === "vault_denied" ? "" : " Resumes on its own."}`,
      };
    case "fix":
      return state.reconnectProviderId
        ? { label: "needs reconnect", error: true, tip: state.text }
        : { label: "not set up", error: false, tip: state.text };
    default:
      return null;
  }
}

/** "local only" is a claim: drop it once audio or questions leave the Mac. */
export function isLocalOnly(settings: RecordingSettings | null): boolean {
  if (!settings) return true;
  const t = settings.transcription;
  if (t?.enabled && t.provider === "deepgram") return false;
  const ai = settings.aiRuntime;
  if (ai?.enabled && ai.defaultModel) {
    const kind = ai.providers.find((p) => p.id === ai.defaultModel?.provider)?.kind;
    if (kind !== "ollama" && kind !== "llamafile") return false;
  }
  return true;
}

/** "4h 41m" · "12m". */
export function formatTracked(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000);
  const h = Math.floor(minutes / 60);
  return h > 0 ? `${h}h ${minutes % 60}m` : `${minutes}m`;
}

/** "01:39:07" (always with hours) or "00:12" (mm:ss until an hour has passed). */
export function formatClock(ms: number, withHours: boolean): string {
  const s = Math.floor(Math.max(0, ms) / 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  const hh = Math.floor(s / 3600);
  const mmss = `${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`;
  return withHours || hh > 0 ? `${pad(hh)}:${mmss}` : mmss;
}

const STOPPED_REASONS: Record<string, string> = {
  capture_disk_full_stopped: "disk full",
  capture_trial_ended_stopped: "trial ended",
  capture_license_revoked_stopped: "license revoked",
};

/** The system's "Recording stopped — …" notification (ids from
 *  `native_capture/segments.rs`) as a short reason — unless a session started
 *  after it, which makes it history rather than the current state. */
export function stoppedReasonFrom(
  notifications: { id: string; createdAtUnixMs: number }[],
  lastSessionStartMs: number | null,
): string | null {
  const hit = notifications.find(
    (n) => n.id in STOPPED_REASONS && (lastSessionStartMs === null || n.createdAtUnixMs > lastSessionStartMs),
  );
  return hit ? STOPPED_REASONS[hit.id] : null;
}
