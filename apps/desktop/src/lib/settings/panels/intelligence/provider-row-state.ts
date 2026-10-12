// Pure status-word mapping for one provider row (mockup `.st-prov__state`):
// the word + tone beside its Manage button. Only facts the settings page
// already holds feed it — key presence, the runtime status reason (which
// names an instance id, ADR 0035), and the reachability evidence: the row's
// own `verify_ai_provider` check plus the model-list probe.

import type { AiProviderConfig } from "$lib/types";

export type RowTone = "ok" | "warn" | "danger" | "";

export interface ProviderRowInput {
  provider: Pick<AiProviderConfig, "id" | "kind" | "baseUrl">;
  cloud: boolean;
  keySaved: boolean;
  /** `AiRuntimeStatus.reason` — only codes ending in this row's id apply. */
  runtimeReason: string | null | undefined;
  /** The row's verify check (or the model-list probe) failed for this instance. */
  listFailed: boolean;
  /** The row's verify check (or the model-list probe) answered live for this
   *  instance — the evidence of reachability. */
  listed: boolean;
}

export function providerRowState(i: ProviderRowInput): { text: string; tone: RowTone } {
  const { provider: p, cloud, keySaved } = i;
  const reason = (i.runtimeReason ?? "").toLowerCase();
  const id = p.id.toLowerCase();
  if (reason === `needs_reconnect:${id}`) return { text: "needs reconnect", tone: "danger" };
  // The sign-in is intact; only the endpoint didn't answer (ADR 0058).
  if (reason === `provider_unreachable:${id}`) return { text: "unreachable", tone: "warn" };
  if (p.kind === "openai_compatible" && !p.baseUrl.trim()) return { text: "needs base URL", tone: "warn" };
  if (p.kind === "chatgpt") return keySaved ? { text: "connected", tone: "ok" } : { text: "not connected", tone: "" };
  if (cloud && !keySaved) return { text: "no key", tone: "" };
  if (i.listFailed) return { text: "not reachable", tone: "warn" };
  if (cloud) return i.listed ? { text: "reachable", tone: "ok" } : { text: "key saved", tone: "ok" };
  return i.listed ? { text: "reachable", tone: "ok" } : { text: "on this Mac", tone: "" };
}

/** Tone of the AI runtime line for an unavailable status reason. */
export function runtimeReasonTone(reason: string | null | undefined): "warn" | "danger" {
  const code = (reason ?? "").toLowerCase();
  return /^(needs_reconnect|no_provider_key|base_url_host_mismatch|invalid_base_url|vault_denied)/.test(code)
    ? "danger"
    : "warn";
}
