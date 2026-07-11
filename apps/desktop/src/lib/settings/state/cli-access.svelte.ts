// CLI access state: the mnema-cli install status and the broker access grants
// surfaced in the Access settings panel. Owns its own non-draft reactive state
// (no `draft*` bindables here) and the load/install/revoke invokes.

import { invoke } from "@tauri-apps/api/core";
import { describeError, errorText } from "./format";

export type BrokerGrant = {
  id: string;
  label: string;
  createdAtUnixMs: number;
  expiresAtUnixMs: number;
  revoked: boolean;
  scope: { recent_days: { days: number } } | "all_retained_history" | Record<string, unknown>;
};

type BrokerGrantFile = {
  grants: BrokerGrant[];
};

export type MnemaCliStatus = {
  installPath: string;
  installDir: string;
  bundledCliPath: string;
  bundledCliExists: boolean;
  installed: boolean;
  installDirInPath: boolean;
  existingTarget: string | null;
};

export type GrantStatus = "active" | "expired" | "revoked";

// ── Pure helpers (label/format) ─────────────────────────────────────────────

export function grantStatus(grant: BrokerGrant, nowMs: number = Date.now()): GrantStatus {
  if (grant.revoked) return "revoked";
  if (grant.expiresAtUnixMs <= nowMs) return "expired";
  return "active";
}

export function formatGrantScope(scope: BrokerGrant["scope"]): string {
  if (scope === "all_retained_history") return "All retained history";
  if (scope && typeof scope === "object" && "recent_days" in scope) {
    const days = (scope as { recent_days?: { days?: number } }).recent_days?.days ?? 0;
    return days <= 1 ? "Last day" : `Last ${days} days`;
  }
  return "Limited scope";
}

export function formatGrantTime(unixMs: number, nowMs: number = Date.now()): string {
  const diffMs = unixMs - nowMs;
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(diffMs);
  if (abs < 60 * 60 * 1000) return rtf.format(Math.round(diffMs / 60000), "minute");
  if (abs < 24 * 60 * 60 * 1000) return rtf.format(Math.round(diffMs / 3600000), "hour");
  return rtf.format(Math.round(diffMs / 86400000), "day");
}

export function grantStatusLabel(grant: BrokerGrant, nowMs: number = Date.now()): string {
  const status = grantStatus(grant, nowMs);
  if (status === "revoked") return "Revoked";
  if (status === "expired") return `Expired ${formatGrantTime(grant.expiresAtUnixMs, nowMs)}`;
  return `Expires ${formatGrantTime(grant.expiresAtUnixMs, nowMs)}`;
}

// ── Reactive store ──────────────────────────────────────────────────────────

export function createCliAccessStore() {
  let brokerGrants = $state<BrokerGrant[]>([]);
  let brokerGrantLoading = $state(false);
  // Ids of grants whose revoke is currently in flight, so the panel can
  // spin/disable only those grants' buttons (mirrors aiProviderKeySavingProvider).
  // A Set (not a single slot) so two concurrent revokes of different grants each
  // track their own spinner — clearing one never prematurely stops another.
  let brokerGrantSavingIds = $state<Set<string>>(new Set());
  let brokerGrantError = $state<string | null>(null);
  let mnemaCliStatus = $state<MnemaCliStatus | null>(null);
  let mnemaCliLoading = $state(false);
  let mnemaCliInstalling = $state(false);
  let mnemaCliError = $state<string | null>(null);

  async function loadBrokerGrants() {
    brokerGrantLoading = true;
    brokerGrantError = null;
    try {
      const response = await invoke<BrokerGrantFile>("list_cli_access_grants");
      brokerGrants = response.grants ?? [];
    } catch (err) {
      brokerGrantError = describeError(err);
    } finally {
      brokerGrantLoading = false;
    }
  }

  async function loadMnemaCliStatus() {
    mnemaCliLoading = true;
    mnemaCliError = null;
    try {
      mnemaCliStatus = await invoke<MnemaCliStatus>("get_cli_access_status");
    } catch (err) {
      mnemaCliError = errorText(err);
    } finally {
      mnemaCliLoading = false;
    }
  }

  async function installMnemaCli() {
    mnemaCliInstalling = true;
    mnemaCliError = null;
    try {
      mnemaCliStatus = await invoke<MnemaCliStatus>(
        mnemaCliStatus?.installed ? "reinstall_cli" : "install_cli",
      );
    } catch (err) {
      mnemaCliError = errorText(err);
    } finally {
      mnemaCliInstalling = false;
    }
  }

  async function revokeAgentBrokerGrant(grantId: string) {
    brokerGrantSavingIds = new Set(brokerGrantSavingIds).add(grantId);
    brokerGrantError = null;
    try {
      await invoke<boolean>("revoke_cli_access_grant", { grantId });
      await loadBrokerGrants();
    } catch (err) {
      brokerGrantError = describeError(err);
    } finally {
      const next = new Set(brokerGrantSavingIds);
      next.delete(grantId);
      brokerGrantSavingIds = next;
    }
  }

  return {
    get brokerGrants() { return brokerGrants; },
    get brokerGrantLoading() { return brokerGrantLoading; },
    get brokerGrantSaving() { return brokerGrantSavingIds.size > 0; },
    isGrantRevoking(grantId: string) { return brokerGrantSavingIds.has(grantId); },
    get brokerGrantError() { return brokerGrantError; },
    get mnemaCliStatus() { return mnemaCliStatus; },
    get mnemaCliLoading() { return mnemaCliLoading; },
    get mnemaCliInstalling() { return mnemaCliInstalling; },
    get mnemaCliError() { return mnemaCliError; },
    loadBrokerGrants,
    loadMnemaCliStatus,
    installMnemaCli,
    revokeAgentBrokerGrant,
  };
}

export type CliAccessStore = ReturnType<typeof createCliAccessStore>;
