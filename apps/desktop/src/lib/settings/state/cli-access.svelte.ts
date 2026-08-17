// CLI access state: the mnema-cli install status, the standing per-tool access
// permissions, and the brokered-access activity log surfaced in the Access
// settings panel. Owns its own non-draft reactive state (no `draft*` bindables
// here) and the load/install/block invokes.

import { invoke } from "@tauri-apps/api/core";
import { describeError, errorText } from "./format";

// Mirrors `app_infra::brokered_access::BrokerGrant` (camelCase on the wire).
// A permission is standing: no expiry field. It dies 30 days after last use
// (pruned backend-side on load, so this list never holds a lapsed row), or it
// is `blocked` — a standing rejection that stays visible here (ADR 0059).
export type BrokerGrant = {
  id: string;
  label: string;
  normalizedLabel: string;
  createdAtUnixMs: number;
  lastUsedAtUnixMs: number;
  scope: { recent_days: { days: number } } | "all_retained_history" | Record<string, unknown>;
  blocked: boolean;
  blockedAtUnixMs: number | null;
};

type BrokerGrantFile = {
  grants: BrokerGrant[];
};

// Mirrors `app_infra::brokered_access::BrokerAuditEvent`. `outcome` records the
// real result ("success" | "denied" | "scope_rejected"), and Ask AI no longer
// writes here at all, so every row is an external tool.
export type BrokerAuditEvent = {
  toolIdentity: string;
  commandType: string;
  timestampUnixMs: number;
  resultCount: number;
  scopeClass: string;
  outcome: string | null;
};

type BrokerAuditFile = {
  events: BrokerAuditEvent[];
};

/** How many activity rows the panel shows. The file itself is capped at 500. */
export const ACTIVITY_LIMIT = 20;

export type MnemaCliStatus = {
  installPath: string;
  installDir: string;
  bundledCliPath: string;
  bundledCliExists: boolean;
  installed: boolean;
  installDirInPath: boolean;
  existingTarget: string | null;
};

export type GrantStatus = "active" | "blocked";

// ── Pure helpers (label/format) ─────────────────────────────────────────────

export function grantStatus(grant: BrokerGrant): GrantStatus {
  return grant.blocked ? "blocked" : "active";
}

export function formatGrantScope(scope: BrokerGrant["scope"]): string {
  if (scope === "all_retained_history") return "All retained history";
  if (scope && typeof scope === "object" && "recent_days" in scope) {
    const days = (scope as { recent_days?: { days?: number } }).recent_days?.days ?? 0;
    return days <= 1 ? "Last day" : `Last ${days} days`;
  }
  return "Limited scope";
}

// The same scope as a noun phrase for prose sentences. `formatGrantScope`
// returns control labels, and lowercasing one into a sentence yields "will be
// able to read last day again" — the mistake the approval window avoids with its
// own `scopeProse` map. Used by the Enable confirmation, which is the only
// friction left between a blocked tool and a restored no-expiry permission.
export function formatGrantScopeProse(scope: BrokerGrant["scope"]): string {
  if (scope === "all_retained_history") return "your entire retained history";
  if (scope && typeof scope === "object" && "recent_days" in scope) {
    const days = (scope as { recent_days?: { days?: number } }).recent_days?.days ?? 0;
    return days <= 1 ? "your last 24 hours" : `your last ${days} days`;
  }
  return "the history its permission still allows";
}

export function formatGrantTime(unixMs: number, nowMs: number = Date.now()): string {
  const diffMs = unixMs - nowMs;
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(diffMs);
  if (abs < 60 * 60 * 1000) return rtf.format(Math.round(diffMs / 60000), "minute");
  if (abs < 24 * 60 * 60 * 1000) return rtf.format(Math.round(diffMs / 3600000), "hour");
  return rtf.format(Math.round(diffMs / 86400000), "day");
}

/**
 * The row's detail-line time. A blocked row states "Blocked" as a badge on the
 * name line, so this only dates it — and says nothing at all when the block
 * carries no timestamp (`blockedAtUnixMs` is `Option<u64>`; a naive read of a
 * null reads "56 years ago").
 */
export function grantStatusLabel(grant: BrokerGrant, nowMs: number = Date.now()): string {
  if (grant.blocked) {
    // Keep the verb even though the name line carries a BLOCKED badge: an active
    // row reads "Used 3 hours ago", so a bare "5 hours ago" beside it is a
    // timestamp with no referent — the reader cannot tell last-use from
    // block-time. `blockedAtUnixMs` is Option<u64> on the wire, so a row blocked
    // by a build that never stamped it drops the time rather than rendering an
    // epoch date.
    // No stamp (Option<u64> on the wire, so a row blocked by a build that never
    // set it) means there is nothing to date, and the BLOCKED badge on the name
    // line already carries the state — so add nothing rather than repeating it.
    return grant.blockedAtUnixMs
      ? `Blocked ${formatGrantTime(grant.blockedAtUnixMs, nowMs)}`
      : "";
  }
  return `Used ${formatGrantTime(grant.lastUsedAtUnixMs, nowMs)}`;
}

/** Audit `outcome` → the word shown on an activity row. */
export function formatOutcome(outcome: string | null): string {
  if (!outcome || outcome === "success") return "";
  if (outcome === "scope_rejected") return "Out of scope";
  if (outcome === "denied") return "Denied";
  // An outcome this build does not know is still a non-success, and the row
  // styles it as refused — so degrade to a safe word rather than rendering the
  // raw snake_case wire value at the user.
  return "Refused";
}

/** Audit `command_type` → the subcommand the tool actually ran (`show_text` → `show-text`). */
export function formatCommand(commandType: string): string {
  return commandType.replaceAll("_", "-");
}

export function formatActivityDetail(event: BrokerAuditEvent): string {
  const outcome = formatOutcome(event.outcome);
  if (outcome) return outcome;
  return event.resultCount === 1 ? "1 result" : `${event.resultCount} results`;
}

// ── Reactive store ──────────────────────────────────────────────────────────

/** The one backend call shape this store makes — the seam its tests substitute. */
export type CliAccessInvoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

// `invokeFn` is a constructor seam, defaulted to the real `invoke` so every app
// caller is unaffected. Tests must not reach the backend through the module
// import: bun's `mock.module("@tauri-apps/api/core", …)` is PROCESS-wide, so a
// spec file registering its own invoke stub silently replaces this module's
// import in a full `bun test` run — the store then reads `undefined` for every
// response and the suite fails on files it never touched. Same reasoning (and the
// same fix) as `insights/receipt-frames.ts`'s `InvokeFn`.
export function createCliAccessStore(invokeFn: CliAccessInvoke = invoke) {
  let brokerGrants = $state<BrokerGrant[]>([]);
  let brokerGrantLoading = $state(false);
  // Ids of grants whose block/enable is currently in flight, so the panel can
  // spin/disable only those grants' buttons (mirrors aiProviderKeySavingProvider).
  // A Set (not a single slot) so two concurrent writes to different grants each
  // track their own spinner — clearing one never prematurely stops another.
  let brokerGrantSavingIds = $state<Set<string>>(new Set());
  let brokerGrantError = $state<string | null>(null);
  let brokerHistory = $state<BrokerAuditEvent[]>([]);
  let brokerHistoryLoading = $state(false);
  let brokerHistoryError = $state<string | null>(null);
  let mnemaCliStatus = $state<MnemaCliStatus | null>(null);
  let mnemaCliLoading = $state(false);
  let mnemaCliInstalling = $state(false);
  let mnemaCliError = $state<string | null>(null);

  // Newest read wins. The panel fires each loader from four places — its 30 s
  // poll, its window-focus refetch, the Refresh button, and (for grants)
  // `setGrantBlocked`'s reload — so several are routinely in flight at once, and
  // each clears its error slot when it STARTS rather than when it succeeds.
  // Without a ticket an older response that lands last overwrites the newer list,
  // paints its failure over a read that already succeeded, or stops the spinner
  // while the newer read is still running. Plain `let`, not `$state`: this is a
  // guard, nothing renders it.
  let brokerGrantsLoadTicket = 0;
  let brokerHistoryLoadTicket = 0;

  async function loadBrokerGrants() {
    const ticket = ++brokerGrantsLoadTicket;
    brokerGrantLoading = true;
    brokerGrantError = null;
    try {
      const response = await invokeFn<BrokerGrantFile>("list_cli_access_grants");
      if (ticket !== brokerGrantsLoadTicket) return;
      brokerGrants = response.grants ?? [];
    } catch (err) {
      if (ticket !== brokerGrantsLoadTicket) return;
      brokerGrantError = describeError(err);
    } finally {
      if (ticket === brokerGrantsLoadTicket) brokerGrantLoading = false;
    }
  }

  async function loadBrokerHistory() {
    const ticket = ++brokerHistoryLoadTicket;
    brokerHistoryLoading = true;
    brokerHistoryError = null;
    try {
      const response = await invokeFn<BrokerAuditFile>("list_cli_access_history");
      if (ticket !== brokerHistoryLoadTicket) return;
      // The audit file is append-ordered and capped at 500; the panel shows the
      // newest handful. Reverse a copy — `events` is the invoke result, but
      // reversing in place still reads badly next to a re-render.
      brokerHistory = [...(response.events ?? [])].reverse().slice(0, ACTIVITY_LIMIT);
    } catch (err) {
      // Activity is evidence, not a control: a failed read must not blank the
      // block/enable buttons above it, so it shares no error slot with them —
      // but it needs its OWN slot. An unreadable audit file renders as an empty
      // list, and "Nothing yet" on a privacy surface is a claim, not a blank.
      // Logged before the guard: every failed read is worth a line, only the
      // RENDERED slot belongs to the newest read.
      console.error("[cli-access] failed to load activity", err);
      if (ticket !== brokerHistoryLoadTicket) return;
      brokerHistoryError = describeError(err);
    } finally {
      if (ticket === brokerHistoryLoadTicket) brokerHistoryLoading = false;
    }
  }

  async function loadMnemaCliStatus() {
    mnemaCliLoading = true;
    mnemaCliError = null;
    try {
      mnemaCliStatus = await invokeFn<MnemaCliStatus>("get_cli_access_status");
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
      // `install_cli` relinks an existing install, so reinstall is the same call.
      mnemaCliStatus = await invokeFn<MnemaCliStatus>("install_cli");
    } catch (err) {
      mnemaCliError = errorText(err);
    } finally {
      mnemaCliInstalling = false;
    }
  }

  async function setGrantBlocked(grant: BrokerGrant, blocked: boolean) {
    brokerGrantSavingIds = new Set(brokerGrantSavingIds).add(grant.id);
    brokerGrantError = null;
    try {
      await invokeFn<boolean>(blocked ? "block_cli_access_client" : "unblock_cli_access_client", {
        clientName: grant.normalizedLabel,
      });
      await loadBrokerGrants();
    } catch (err) {
      brokerGrantError = describeError(err);
    } finally {
      const next = new Set(brokerGrantSavingIds);
      next.delete(grant.id);
      brokerGrantSavingIds = next;
    }
  }

  return {
    get brokerGrants() { return brokerGrants; },
    get brokerGrantLoading() { return brokerGrantLoading; },
    get brokerGrantSaving() { return brokerGrantSavingIds.size > 0; },
    isGrantSaving(grantId: string) { return brokerGrantSavingIds.has(grantId); },
    get brokerGrantError() { return brokerGrantError; },
    get brokerHistory() { return brokerHistory; },
    get brokerHistoryLoading() { return brokerHistoryLoading; },
    get brokerHistoryError() { return brokerHistoryError; },
    get mnemaCliStatus() { return mnemaCliStatus; },
    get mnemaCliLoading() { return mnemaCliLoading; },
    get mnemaCliInstalling() { return mnemaCliInstalling; },
    get mnemaCliError() { return mnemaCliError; },
    loadBrokerGrants,
    loadBrokerHistory,
    loadMnemaCliStatus,
    installMnemaCli,
    setGrantBlocked,
  };
}

export type CliAccessStore = ReturnType<typeof createCliAccessStore>;
