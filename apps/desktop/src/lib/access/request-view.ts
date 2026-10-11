// Pure view logic for the CLI Access request window
// (routes/access/request/+page.svelte, drawn in docs/mockups/redesign/access.html).

export type Scope = "lastDay" | "last7Days" | "allRetained";

export type PendingCliAccessRequest = {
  requestId: string;
  client: { label: string; source: string };
  command: string;
  minimumScope: Scope;
  preferredScope: Scope;
  /// What this tool's standing permission already carries, absent when it holds
  /// none. An approval SETS the row's scope, so a selection below this one takes
  /// access away — see narrowsExistingAccess in the page.
  currentScope: Scope | null;
  createdAt: string;
};

// Narrow → wide. Anything below the request's stated minimum is disabled.
export const SCOPE_RANK: Record<Scope, number> = { lastDay: 0, last7Days: 1, allRetained: 2 };

export const SCOPE_LABEL: Record<Scope, string> = {
  lastDay: "Last day",
  last7Days: "Last 7 days",
  allRetained: "All retained",
};

// Noun-phrase wording for prose sentences, where the bare label ("Last day")
// would read as "your Last day text".
export const SCOPE_PROSE: Record<Scope, string> = {
  lastDay: "your last 24 hours",
  last7Days: "your last 7 days",
  allRetained: "your entire retained history",
};

export const SCOPE_DESC: Record<Scope, string> = {
  lastDay: "Only text captured in the last 24 hours.",
  last7Days: "Text captured in the last 7 days.",
  allRetained: "Your entire retained capture history.",
};

export const SCOPES: Scope[] = ["lastDay", "last7Days", "allRetained"];

/// Identity provenance → [chip, sentence, warn]. The chip is a qualifier on the
/// tool name; the sentence lives in the tooltip / screen-reader text. Weaker
/// provenances (inferred / none) are warn-tinted; "explicit" is never greened —
/// even a declared identity stays unverifiable.
export function identitySource(source: string): { chip: string; detail: string; warn: boolean } {
  switch (source) {
    case "explicit":
      return { chip: "Declared", detail: "Identity declared by the tool", warn: false };
    case "env":
      return { chip: "From env", detail: "Identity from an environment variable", warn: false };
    case "inferred":
      return { chip: "Inferred", detail: "Identity inferred from the process", warn: true };
    default:
      return { chip: "Unnamed", detail: "No identity provided", warn: true };
  }
}

/// A scope widen: the tool already holds a permission and this request asks for
/// more than it carries. Allowing mutates that row in place (ADR 0059), so the
/// opaque ids it already issued keep working.
export function isWiden(r: PendingCliAccessRequest): boolean {
  return r.currentScope !== null && SCOPE_RANK[r.currentScope] < SCOPE_RANK[r.preferredScope];
}

/// The quiet tag after each option title: what it already has, what it asked
/// for, and — when anything is greyed out — the minimum that works.
export function scopeTag(r: PendingCliAccessRequest, scope: Scope): string {
  if (scope === r.currentScope) return "current";
  if (scope === r.preferredScope && r.preferredScope !== r.minimumScope) return "asked for";
  if (scope === r.minimumScope && (r.currentScope !== null || SCOPE_RANK[r.minimumScope] > 0)) return "needed";
  return "";
}

export function scopeDisabled(r: PendingCliAccessRequest, scope: Scope): boolean {
  return SCOPE_RANK[scope] < SCOPE_RANK[r.minimumScope];
}

/// The muted line under the scope list ("" = none). The pre-selection is the
/// minimum, never the tool's ask, so the ask is named here instead.
export function scopeHint(r: PendingCliAccessRequest, selected: Scope): string {
  if (isWiden(r) && r.currentScope) return `Deny keeps ${SCOPE_LABEL[r.currentScope]} — this command fails.`;
  if (SCOPE_RANK[selected] < SCOPE_RANK[r.preferredScope]) {
    return `${r.client.label} asked for ${SCOPE_PROSE[r.preferredScope]}. Start narrow — it asks again if it needs more.`;
  }
  return "";
}

/// Allow names the consent it grants: full history escalates, a widen names
/// the new window.
export function allowLabel(r: PendingCliAccessRequest, selected: Scope): string {
  if (selected === "allRetained") return "Allow full-history access";
  if (isWiden(r)) return `Allow ${SCOPE_LABEL[selected].toLowerCase()}`;
  return "Allow access";
}

/// "mnema search" (bold) + the rest of the command line.
export function splitCommand(command: string): { head: string; rest: string } {
  const trimmed = command.trim();
  const space = trimmed.search(/\s/);
  if (space < 0) return { head: `mnema ${trimmed}`.trim(), rest: "" };
  return { head: `mnema ${trimmed.slice(0, space)}`, rest: trimmed.slice(space + 1) };
}

/// How long the request has been waiting. The hard timeout is CLI-side (~120 s),
/// so this is an honesty cue, not a countdown.
export function requestAge(createdAt: string, now: number): string | null {
  const createdMs = new Date(createdAt).getTime();
  if (Number.isNaN(createdMs)) return null;
  const ageSeconds = Math.max(0, Math.round((now - createdMs) / 1000));
  if (ageSeconds < 5) return "just now";
  if (ageSeconds < 60) return `Requested ${ageSeconds}s ago`;
  return `Requested ${Math.round(ageSeconds / 60)} min ago`;
}

/// The window height that fits the dialog's natural height, or `null` to leave
/// it. A new view (loading → request, request → receipt…) fits exactly, short
/// views included; within one view it only grows, so Allow/Deny never move up
/// under the pointer when a note collapses.
export function fitAccessWindowHeight(
  current: number,
  natural: number,
  sameView: boolean,
  max: number,
): number | null {
  const target = Math.round(Math.min(max, sameView ? Math.max(current, natural) : natural));
  return Math.abs(target - current) < 2 ? null : target;
}
