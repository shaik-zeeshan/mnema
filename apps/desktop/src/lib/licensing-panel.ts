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

/** Scannable state chip after the Status label: copy + kit `mx-chip` tone
 * (`plain` = the bare chip). */
export function badgeFor(
  status: LicenseStatus | null,
): { label: string; variant: "accent" | "warn" | "info" | "plain" } | null {
  switch (status?.kind) {
    case "licensed":
      switch (status.activation.state) {
        case "pending":
          return { label: "activating…", variant: "plain" };
        case "refusedOverCap":
          return { label: "device limit", variant: "warn" };
        case "lapsed":
          return { label: "not activated", variant: "warn" };
        default:
          return { label: "licensed", variant: "accent" };
      }
    case "trial":
      return { label: "trial", variant: "info" };
    case "trialNotStarted":
      return { label: "trial ready", variant: "plain" };
    case "readOnly":
      return { label: "read-only", variant: "warn" };
    case "revoked":
      return { label: "revoked", variant: "warn" };
    default:
      return null;
  }
}

export function fmtLicenseDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function days(n: number): string {
  return `${n} ${n === 1 ? "day" : "days"}`;
}

/** The Status row's one composite description line, or `null` when the state
 * speaks only through its inline / notice (read-only, revoked, lapsed, device limit). */
export function statusHintFor(
  status: LicenseStatus | null,
  devices: { used: number; cap: number } | null,
): string | null {
  switch (status?.kind) {
    case "trialNotStarted":
      return `Free trial — starts when you first record (${days(status.trialDays)}).`;
    case "trial":
      return `${days(status.daysLeft)} left · ends ${fmtLicenseDate(status.trialEndMs)}. After that Mnema switches to Read-Only Mode — your history stays searchable, only new recording pauses until you buy.`;
    case "licensed": {
      const who = status.name || status.email;
      const act = status.activation;
      if (act.state === "pending") {
        return `Licensed to ${who}. Finishing activation… (${days(act.provisionalDaysLeft)} to connect).`;
      }
      if (act.state !== "activated") return null;
      const parts = [`Licensed to ${who}`];
      if (status.inWindow) parts.push(`updates included through ${fmtLicenseDate(status.updateThroughMs)}`);
      // A count only, never a list: no device names are sent or stored.
      if (devices) parts.push(`${devices.used} of ${devices.cap} devices activated`);
      return `${parts.join(" · ")}.`;
    }
    default:
      return null;
  }
}

/** The Status row's one-line inline message (tone + icon key + copy), if any. */
export function statusInlineFor(
  status: LicenseStatus | null,
): { tone: "warn" | "danger"; icon: "lock" | "alert" | "cloud" | "info"; text: string } | null {
  switch (status?.kind) {
    case "readOnly":
      return { tone: "warn", icon: "lock", text: "Trial ended — Read-Only Mode. History stays searchable; buy to resume recording." };
    case "revoked":
      return { tone: "danger", icon: "alert", text: "This license has been revoked — Read-Only Mode. History stays searchable; new recording is paused." };
    case "licensed":
      if (status.activation.state === "lapsed") {
        return { tone: "warn", icon: "cloud", text: "We couldn’t confirm your license — connect to the internet once to finish activation. New recording is paused until then." };
      }
      if (!status.inWindow) {
        return { tone: "warn", icon: "info", text: `Update window lapsed ${fmtLicenseDate(status.updateThroughMs)} — you keep this version forever; renew for new builds.` };
      }
      return null;
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
