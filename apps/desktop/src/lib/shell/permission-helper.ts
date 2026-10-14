// The permission helper window (`AppWindow::PermissionHelper`): every
// "Open System Settings" goes through `openPermissionHelper`, which opens the
// right Privacy & Security pane AND floats the small always-on-top card that
// watches for the grant (`routes/permission-helper`).
import { invoke } from "@tauri-apps/api/core";
import type { PermissionsMap } from "$lib/types";

export type PermissionHelperKind = "screen" | "microphone" | "systemAudio" | "accessibility";

/** The System Settings list each kind lives in — the helper's title names it. */
export const PERMISSION_HELPER_LIST: Record<PermissionHelperKind, string> = {
  screen: "Screen Recording",
  microphone: "Microphone",
  systemAudio: "Screen & System Audio Recording",
  accessibility: "Accessibility",
};

export function permissionHelperKind(raw: string | null): PermissionHelperKind {
  return raw !== null && Object.hasOwn(PERMISSION_HELPER_LIST, raw) ? (raw as PermissionHelperKind) : "screen";
}

/**
 * Has the user flipped the switch? System audio has no authorization query
 * (ADR 0052): `assumed_working` only appears once a tap has delivered sound, so
 * its helper usually waits until dismissed — it never claims a grant it can't see.
 */
export function permissionHelperGranted(
  kind: PermissionHelperKind,
  permissions: PermissionsMap | null,
  accessibilityTrusted: boolean,
): boolean {
  if (kind === "accessibility") return accessibilityTrusted;
  const value = permissions?.[kind];
  return value === "granted" || value === "assumed_working";
}

/** Open System Settings for `kind` and float the helper beside it. */
export function openPermissionHelper(kind: PermissionHelperKind): Promise<void> {
  return invoke("show_permission_helper", { kind });
}
