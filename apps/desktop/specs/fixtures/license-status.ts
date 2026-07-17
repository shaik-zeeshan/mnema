// One fixture per LicenseStatus wire variant, written with the EXACT serde
// field names from `crates/capture-types/src/licensing.rs` (kind/state tags +
// camelCase fields). This doubles as the TS-side mirror pin: a Rust-only
// variant or field rename surfaces here as a type error or a wrong branch in
// the specs that consume these.

import type { LicenseStatus } from "../../src/lib/licensing";

export const TRIAL_NOT_STARTED: LicenseStatus = { kind: "trialNotStarted", trialDays: 30 };

export function trial(daysLeft: number, trialEndMs = 1_700_000_000_000): LicenseStatus {
  return { kind: "trial", daysLeft, trialEndMs };
}

export const READ_ONLY: LicenseStatus = { kind: "readOnly" };

export const REVOKED: LicenseStatus = { kind: "revoked" };

export function licensed(
  overrides: Partial<Extract<LicenseStatus, { kind: "licensed" }>> = {},
): LicenseStatus {
  return {
    kind: "licensed",
    updateThroughMs: 1_731_536_000_000,
    inWindow: true,
    email: "owner@example.com",
    name: "Ada Lovelace",
    activation: { state: "activated" },
    ...overrides,
  };
}

export function licensedPending(provisionalDaysLeft: number, inWindow = true): LicenseStatus {
  return licensed({ inWindow, activation: { state: "pending", provisionalDaysLeft } });
}

export function licensedOverCap(
  resetUrl = "https://license.example/reset",
  buyUrl = "https://mnema.day/#pricing",
): LicenseStatus {
  return licensed({ activation: { state: "refusedOverCap", resetUrl, buyUrl } });
}

export const LICENSED_LAPSED: LicenseStatus = licensed({ activation: { state: "lapsed" } });

/** Every wire variant once — for exhaustive sweeps (badge/statusLine/etc.). */
export const ALL_VARIANTS: LicenseStatus[] = [
  TRIAL_NOT_STARTED,
  trial(7),
  READ_ONLY,
  REVOKED,
  licensed(),
  licensedPending(5),
  licensedOverCap(),
  LICENSED_LAPSED,
];
