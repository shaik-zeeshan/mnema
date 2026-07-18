// Pure banner policy for the app-shell licensing banner — extracted from
// LicenseBanner.svelte so precedence, thresholds, tone, and dismissal keying
// are bun-testable (specs/licensing-banner.test.ts). The component only renders.

import type { LicenseStatus } from "./licensing";

export function days(n: number): string {
  return `${n} ${n === 1 ? "day" : "days"}`;
}

/** What the shell banner shows, in precedence order. `dismissKey` is `null` for
 * the firm (non-dismissible) states; for dismissible ones it's the current
 * day-count, so a fresh escalation (count dropping a tier) re-surfaces a
 * previously dismissed banner. */
export type LicenseBanner =
  | { kind: "readOnly"; dismissKey: null }
  | { kind: "revoked"; dismissKey: null }
  | { kind: "lapsed"; dismissKey: null }
  | { kind: "provisional"; daysLeft: number; dismissKey: number }
  | {
      kind: "trial";
      daysLeft: number;
      tone: "info" | "warn" | "urgent";
      message: string;
      dismissKey: number;
    };

/** The banner for a status, or `null` when nothing should show. Precedence:
 * readOnly → revoked → lapsed activation → provisional nudge (≤3 days) →
 * final-week trial (≤7 days). */
export function bannerFor(status: LicenseStatus | null): LicenseBanner | null {
  if (!status) return null;
  if (status.kind === "readOnly") return { kind: "readOnly", dismissKey: null };
  if (status.kind === "revoked") return { kind: "revoked", dismissKey: null };

  if (status.kind === "licensed") {
    if (status.activation.state === "lapsed") {
      return { kind: "lapsed", dismissKey: null };
    }
    // Final-days provisional (≤3) is the soft "connect soon" nudge.
    // refusedOverCap is deliberately Settings-only — capture still works, no nag.
    if (
      status.activation.state === "pending" &&
      status.activation.provisionalDaysLeft <= 3
    ) {
      const daysLeft = status.activation.provisionalDaysLeft;
      return { kind: "provisional", daysLeft, dismissKey: daysLeft };
    }
    return null;
  }

  // Final-week trial banner: only when a trial is running with ≤7 days left.
  if (status.kind === "trial" && status.daysLeft <= 7) {
    const daysLeft = status.daysLeft;
    const tone = daysLeft <= 1 ? "urgent" : daysLeft <= 3 ? "warn" : "info";
    const lead =
      daysLeft <= 1
        ? "Your free trial ends today."
        : `Free trial ends in ${days(daysLeft)}.`;
    const message = `${lead} After that, Mnema switches to Read-Only Mode — your recorded history stays fully searchable; only new recording pauses until you buy.`;
    return { kind: "trial", daysLeft, tone, message, dismissKey: daysLeft };
  }

  return null;
}

/** Dismissal check: a banner shows unless it's dismissible and was dismissed at
 * exactly this day-count (so 3 → 2 re-surfaces it). */
export function bannerVisible(
  banner: LicenseBanner | null,
  dismissedAtKey: number | null,
): boolean {
  if (!banner) return false;
  return banner.dismissKey === null || banner.dismissKey !== dismissedAtKey;
}
