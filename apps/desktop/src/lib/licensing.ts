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

/** Public Polar checkout link for the one-time Mnema License ($69). Override via VITE_LICENSE_CHECKOUT_URL. */
export const LICENSE_CHECKOUT_URL =
	import.meta.env.VITE_LICENSE_CHECKOUT_URL ??
	"https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_YHKNSVQFLu5jQdlQvAlupGMvOoH2a5axMrJti4NOEIu/redirect";

/** Polar preselects a product on a multi-product checkout link via the
 * `product_id` query param — append it with the right `?`/`&` join. */
export function renewalCheckoutUrl(baseCheckoutUrl: string, productId: string): string {
	return `${baseCheckoutUrl}${baseCheckoutUrl.includes("?") ? "&" : "?"}product_id=${productId}`;
}

/** Checkout link for the $29 renewal. The default reuses the license link with
 * the (sandbox) renewal product preselected — the link must have the renewal
 * product attached in the Polar dashboard.
 * Override via VITE_RENEWAL_CHECKOUT_URL (e.g. a dedicated renewal link). */
export const RENEWAL_CHECKOUT_URL =
	import.meta.env.VITE_RENEWAL_CHECKOUT_URL ??
	renewalCheckoutUrl(LICENSE_CHECKOUT_URL, "adb6fc3d-a1c7-41d3-8568-3c1789b8b1f6");
