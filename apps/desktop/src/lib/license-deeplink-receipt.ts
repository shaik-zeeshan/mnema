// Pure state policy for LicenseDeepLinkModal — which license_status result
// maps to which face of the deep-link receipt. Extracted so the mapping is
// bun-testable (specs/license-deeplink-receipt.test.ts); the component only
// renders. `DeepLinkFlow` hand-mirrors `LicenseDeepLinkPayload.flow` in
// `src-tauri/src/lib.rs` (no codegen, per CLAUDE.md).

import type { LicenseStatus } from "./licensing";

export type DeepLinkFlow = "activate" | "claim" | "renewed";

/** Terminal deep-link endings that never produce a `license_status` emit.
 * Hand-mirrors `LicenseDeepLinkDonePayload` in `src-tauri/src/licensing.rs`:
 * `failed` shows the modal's failed face, `closed` closes it silently (the
 * user declined the replacement confirm, or the claim path handed off to its
 * native "check your email" dialog). */
export type LicenseDeepLinkDone =
  | { outcome: "failed"; message: string }
  | { outcome: "closed" };

export type ReceiptFace =
  /** Deep link routed, result not landed yet — spinner, dismissible. */
  | { face: "working" }
  /** Terminal happy path: receipt verified on this machine. */
  | { face: "activated"; owner: string; updateThroughMs: number }
  /** Key installed, device confirmation still in flight (provisional window). */
  | { face: "pending"; owner: string; provisionalDaysLeft: number }
  /** Renewal landed: the update window moved. `wasMs` is the pre-deep-link
   * through-date (null when we never saw one) for the "was …" strikethrough. */
  | { face: "renewed"; updateThroughMs: number; wasMs: number | null }
  /** Actionable failure: at the 3-device cap; links come from the server 409. */
  | { face: "overCap"; resetUrl: string; buyUrl: string }
  /** Terminal failure reported by `license_deep_link_done` (never derived
   * from a status — statuses can only keep waiting or succeed). */
  | { face: "failed"; message: string };

/**
 * `baseline` is the status snapshot when the deep link arrived; `current` is
 * the latest emit. Non-licensed statuses always mean "keep waiting" — the
 * deep link's own result hasn't landed (e.g. the boot gate's trial snapshot
 * racing a cold-start claim).
 */
export function receiptFaceFor(
  flow: DeepLinkFlow,
  baseline: LicenseStatus | null,
  current: LicenseStatus | null,
): ReceiptFace {
  if (current?.kind !== "licensed") return { face: "working" };

  if (current.activation.state === "refusedOverCap") {
    const { resetUrl, buyUrl } = current.activation;
    return { face: "overCap", resetUrl, buyUrl };
  }

  if (flow === "renewed") {
    // The renewal poll keeps emitting until the extended window lands; an
    // unchanged through-date is a pre-extension emit, not the result.
    const wasMs = baseline?.kind === "licensed" ? baseline.updateThroughMs : null;
    if (wasMs !== null && current.updateThroughMs === wasMs) return { face: "working" };
    return { face: "renewed", updateThroughMs: current.updateThroughMs, wasMs };
  }

  const owner = current.name || current.email;
  switch (current.activation.state) {
    case "activated":
      return { face: "activated", owner, updateThroughMs: current.updateThroughMs };
    case "pending":
      return {
        face: "pending",
        owner,
        provisionalDaysLeft: current.activation.provisionalDaysLeft,
      };
    case "lapsed":
      // Unreachable seconds after a deep link (the provisional window just
      // opened); don't invent a face — the working timeout copy covers it.
      return { face: "working" };
  }
}

/** Same long-form date the Settings license panel uses. */
export function fmtReceiptDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}
