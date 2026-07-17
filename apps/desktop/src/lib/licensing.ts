// Licensing wire types — the frontend mirror of
// `crates/capture-types/src/licensing.rs`. No codegen (per CLAUDE.md); these
// must agree field-for-field with the Rust side, guarded there by a serde
// round-trip test. `readOnly` is the ONLY capture-blocking status.

/** Offline license/trial status, computed by the gate and surfaced to the UI. */
export type LicenseStatus =
  /** Trial clock not yet started (no successful Capture yet). Capture allowed. */
  | { kind: "trialNotStarted"; trialDays: number }
  /** Trial running. Capture allowed. */
  | { kind: "trial"; daysLeft: number; trialEndMs: number }
  /** Trial expired, unlicensed. Capture disabled; reads untouched. */
  | { kind: "readOnly" }
  /** Authentic key on the signed revocation list — capture blocked, history readable. */
  | { kind: "revoked" }
  /** Owns a license. Capture allowed unless `activation` is `lapsed`;
   * `inWindow` gates only new builds. `name` is "" when the key has none. */
  | {
      kind: "licensed";
      updateThroughMs: number;
      inWindow: boolean;
      email: string;
      name: string;
      activation: Activation;
    };

/** Once-per-machine activation state layered onto a `licensed` key (ADR 0053).
 * Only `lapsed` blocks capture; the rest allow it (still inside the window). */
export type Activation =
  /** Receipt verified on this machine — offline forever. */
  | { state: "activated" }
  /** In the Provisional Window, still trying to activate. Capture allowed. */
  | { state: "pending"; provisionalDaysLeft: number }
  /** At the device cap; still in the window (capture allowed), UI shows reset + buy links. */
  | { state: "refusedOverCap"; resetUrl: string; buyUrl: string }
  /** Provisional Window exhausted, never activated. Capture blocked. */
  | { state: "lapsed" };

/** Result of pasting a license key into Settings. */
export interface ActivateLicenseResult {
  status: LicenseStatus;
}

/** Device usage from the server: a COUNT only, never a device list — the
 * privacy commitment ("no device names sent or stored") stays word-for-word. */
export interface LicenseDevices {
  used: number;
  cap: number;
}

/** Result of "Free up my devices". Other refusals surface as the invoke's
 * rejection (a human-readable message). */
export type ResetDevicesOutcome =
  /** Slots emptied; activation is already retrying in the background. */
  | { outcome: "reset" }
  /** Reset cooldown (once per 30 days); `retryAtMs` is when it reopens. */
  | { outcome: "rateLimited"; retryAtMs: number | null };

/** Public Polar checkout link for the one-time Mnema License ($69). Override via VITE_LICENSE_CHECKOUT_URL. */
// `||` not `??`: an unset GitHub Actions `vars.*` reaches the build as "" — treat empty as unset.
export const LICENSE_CHECKOUT_URL =
	import.meta.env.VITE_LICENSE_CHECKOUT_URL ||
	"https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_lMoTLnM0OegXGCtfDMzfFi54ZZ41zhfSL8mvP1BpK1L/redirect";

/** Dedicated checkout link for the $29 renewal — separate from the purchase
 * link because the success URL is a property of the link, and renewals must
 * return via `mnema.day/license/renewed` (purchases via `/license/claim`).
 * Override via VITE_RENEWAL_CHECKOUT_URL. */
export const RENEWAL_CHECKOUT_URL =
	import.meta.env.VITE_RENEWAL_CHECKOUT_URL ||
	"https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_Tihx0GjXAe53ljFFHWh4wxUwb3u0KSVLBvJ5b00hFyi/redirect";
