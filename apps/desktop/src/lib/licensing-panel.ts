// Pure presentation policy for the Settings License panel and the onboarding
// license body — extracted from License.svelte / LicenseBody.svelte so the
// money path (buy vs renew), badge, and status copy are bun-testable
// (specs/licensing-panel.test.ts). The components only render.

import type { LicenseStatus } from "./licensing";
import { LICENSE_CHECKOUT_URL, RENEWAL_CHECKOUT_URL } from "./licensing";

/** Lapsed owner (out of update window) → the Renew variant. */
export function licensedOutOfWindow(status: LicenseStatus | null): boolean {
  return status?.kind === "licensed" && !status.inWindow;
}

/** The Buy/Renew row is hidden only for an in-window owner. */
export function showBuyFor(status: LicenseStatus | null): boolean {
  return !(status?.kind === "licensed" && status.inWindow);
}

/** Money path: lapsed owners renew ($29); everyone else buys the license ($69). */
export function checkoutUrlFor(status: LicenseStatus | null): string {
  return licensedOutOfWindow(status) ? RENEWAL_CHECKOUT_URL : LICENSE_CHECKOUT_URL;
}

/** Scannable state signifier leading the Status row. */
export function badgeFor(
  status: LicenseStatus | null,
): { label: string; variant: "ok" | "neutral" | "warn" } | null {
  switch (status?.kind) {
    case "licensed":
      switch (status.activation.state) {
        case "pending":
          return { label: "Activating…", variant: "neutral" };
        case "refusedOverCap":
          return { label: "Device limit", variant: "warn" };
        case "lapsed":
          return { label: "Not activated", variant: "warn" };
        default:
          return { label: "Licensed", variant: "ok" };
      }
    case "trial":
      return { label: "Trial", variant: "neutral" };
    case "trialNotStarted":
      return { label: "Trial ready", variant: "neutral" };
    case "readOnly":
      return { label: "Read-only", variant: "warn" };
    case "revoked":
      return { label: "Revoked", variant: "warn" };
    default:
      return null;
  }
}

/** Server-provided links (reset/buy on an over-cap 409) are opened in the OS
 * browser only when they are real `https:` URLs — never `file:`, `mnema:`, or
 * anything else a compromised/misconfigured server could hand us. */
export function safeExternalUrl(url: string): string | null {
  try {
    return new URL(url).protocol === "https:" ? url : null;
  } catch {
    return null;
  }
}

/** Onboarding license body: the one-line live reflection of the user's state.
 * `null` (gate hasn't run, or a genuine first run) → just the generic explainer. */
export function statusLineFor(status: LicenseStatus | null): string | null {
  if (!status) return null;
  switch (status.kind) {
    case "trial":
      return `You're on the free trial — ${status.daysLeft} ${status.daysLeft === 1 ? "day" : "days"} left.`;
    case "trialNotStarted":
      return "Your free trial starts the moment you first record.";
    case "readOnly":
      return "Your trial has ended — you're in Read-Only Mode. Everything you recorded stays browsable; buy once to record again.";
    case "revoked":
      return "This license has been revoked — you're in Read-Only Mode. Everything you recorded stays browsable; buy once to record again.";
    case "licensed":
      // A lapsed activation blocks recording (same state License.svelte and
      // LicenseBanner distinguish) — don't promise "never pauses" there.
      return status.activation.state === "lapsed"
        ? "You own Mnema, but activation hasn't finished — connect to the internet once to resume recording."
        : "You own Mnema — thank you. Recording never pauses.";
  }
}
