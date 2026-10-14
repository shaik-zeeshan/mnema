// The Permissions screen's gate predicates.
//
// Extracted from `PermissionsScreen.svelte` so they can be tested. A denial
// gates nothing (see the header of `gates.ts`); the ONE hold is the relaunch a
// screen grant made in this process needs. It lives outside `gates.ts` because
// it reads live OS state rather than resolved settings.
//
// The `"unsupported"` case is the one that matters most and is easy to lose: a
// machine that cannot report Screen Recording at all must pass, not block. The
// old `onboarding-feature-gate.test.ts` covered exactly that and was deleted
// with the "attention item" concept; this is where that coverage now lives.
import type { PermissionValue } from "../../routes/onboarding/onboarding-attention";

/**
 * Does this permission count as granted for gating purposes?
 *
 * `assumed_working` and `unsupported` both count. `unsupported` is NOT a denial:
 * it means the OS has nothing to report, and blocking on it would trap every
 * user on such a machine with no in-app recovery — macOS never re-prompts.
 */
export function isGranted(value: PermissionValue | undefined): boolean {
  return value === "granted" || value === "assumed_working" || value === "unsupported";
}

export function isDenied(value: PermissionValue | undefined): boolean {
  return value === "denied" || value === "restricted";
}

export interface ScreenGate {
  /** May the user continue past Permissions? */
  ready: boolean;
  /** What the primary button says when it cannot simply continue. */
  primaryLabel: string;
}

/**
 * The Screen Recording moment's exit.
 *
 * Only `needsRelaunch` holds: the permission reads granted while
 * ScreenCaptureKit *in this process* still cannot use it, so continuing would
 * only reach the Finale and fail there on a stale stream. A denial continues —
 * the session runs audio-only (or idle) and the Finale says so.
 */
export function screenGate(
  value: PermissionValue | undefined,
  needsRelaunch: boolean,
): ScreenGate {
  if (needsRelaunch) return { ready: false, primaryLabel: "Relaunch to continue" };
  if (isDenied(value)) return { ready: true, primaryLabel: "Continue without screen recording" };
  return { ready: true, primaryLabel: "Continue" };
}
