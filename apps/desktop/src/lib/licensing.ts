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
  /** Owns a license. Capture always allowed; `inWindow` gates only new builds. */
  | { kind: "licensed"; updateThroughMs: number; inWindow: boolean; email: string };

/** Result of pasting a license key into Settings. */
export interface ActivateLicenseResult {
  status: LicenseStatus;
}

/** Public Polar checkout link for the one-time Mnema License ($69). Override via VITE_LICENSE_CHECKOUT_URL. */
export const LICENSE_CHECKOUT_URL =
	import.meta.env.VITE_LICENSE_CHECKOUT_URL ??
	"https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_YHKNSVQFLu5jQdlQvAlupGMvOoH2a5axMrJti4NOEIu/redirect";
