// Screen / microphone permission rows — the pure half. System audio is not
// here: it has no authorization query and keeps its own inferred hint (ADR 0052).
import type { PermissionStatus } from "$lib/types";

export type PermissionKind = "screen" | "microphone";

/**
 * What a row offers: nothing (allowed), the macOS prompt (never asked), or the
 * System Settings pane (macOS won't re-prompt once denied). `hidden` = this Mac
 * has no such permission.
 */
export type PermissionRowState = "granted" | "ask" | "denied" | "hidden";

export function permissionRowState(status: PermissionStatus | undefined): PermissionRowState {
  if (status === "granted") return "granted";
  if (status === "unsupported") return "hidden";
  if (status === "denied" || status === "restricted") return "denied";
  return "ask";
}

export const PERMISSION_ROWS: { kind: PermissionKind; label: string; pane: string; what: string }[] = [
  { kind: "screen", label: "Screen Recording", pane: "Screen Recording", what: "record the screen" },
  { kind: "microphone", label: "Microphone", pane: "Microphone", what: "record the microphone" },
];
