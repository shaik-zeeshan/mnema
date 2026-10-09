// Shared, runtime-free AI-provider helpers (ADR 0034/0035).
//
// The provider KIND catalog plus the pure label / instance-label / new-instance-id
// functions, factored out of the Settings controller so the onboarding flow can
// build the SAME provider list inline (its main-window Settings page isn't open
// yet during first-run). Both surfaces share one source of truth for what a
// provider kind is called, which kinds are cloud (and so carry a keychain key),
// each local kind's default endpoint, and how a connected instance is labeled —
// so the two never drift apart.
//
// Keep this file free of `$state`/`$derived`/`$effect` and of Tauri `invoke`:
// it is plain data + pure functions. Stateful concerns (the keychain
// presence/inputs, the runtime status) live in `ai-runtime.svelte.ts`.

import type { AiProviderConfig, AiProviderKind } from "$lib/types";

/** Every provider kind the Reasoning Engine can connect, in display order. */
export const AI_PROVIDER_KINDS: readonly AiProviderKind[] = [
  "anthropic",
  "openai",
  "chatgpt",
  "openai_compatible",
  "ollama",
  "llamafile",
];

/** Kinds that talk to a hosted API and so store a key in the OS keychain. */
export const CLOUD_AI_PROVIDER_KINDS: readonly AiProviderKind[] = [
  "anthropic",
  "openai",
  "chatgpt",
  "openai_compatible",
];

/** Default localhost endpoint for each local (on-device) provider kind. */
export const AI_LOCAL_DEFAULT_ENDPOINTS: Partial<Record<AiProviderKind, string>> = {
  ollama: "http://localhost:11434",
  llamafile: "http://localhost:8080",
};

/** Is this kind a cloud provider (→ needs a keychain key, no endpoint)? */
export function isCloudAiProviderKind(kind: string): boolean {
  return (CLOUD_AI_PROVIDER_KINDS as readonly string[]).includes(kind);
}

/** Human label for a provider kind (falls back to the raw id). */
export function aiProviderKindLabel(kind: string): string {
  switch (kind) {
    case "anthropic": return "Anthropic";
    case "openai": return "OpenAI";
    case "chatgpt": return "ChatGPT";
    case "openai_compatible": return "OpenAI-compatible";
    case "ollama": return "Ollama";
    case "llamafile": return "Llamafile";
    default: return kind;
  }
}

/** One-line description of a kind (shown on the "+ Add" buttons / tooltips). */
export function aiProviderKindDescription(kind: AiProviderKind): string {
  switch (kind) {
    case "anthropic": return "Claude models — your own API key";
    case "openai": return "GPT models — your own API key";
    case "chatgpt": return "Your ChatGPT Plus/Pro subscription — sign in, no API key";
    case "openai_compatible": return "Fireworks, OpenRouter, Together — custom base URL + key";
    case "ollama": return "Local runtime, default endpoint http://localhost:11434";
    case "llamafile": return "Local OpenAI-compatible server, default http://localhost:8080";
  }
}

/**
 * Human copy for one `AiRuntimeProviderFailure.reason` (the per-provider model
 * listing failure). `classify_listing_failure` already hands back an
 * at-a-glance phrase for every kind EXCEPT the chatgpt reconnect code, which it
 * passes through verbatim so the onboarding card can word it its own way — this
 * is the translation for every other surface that shows a listing failure
 * (ModelPickerMenu rows in Chat/Quick Recall/Settings, and the aggregated
 * `modelsError` lines), which would otherwise print `needs_reconnect:chatgpt`.
 */
export function aiListingFailureCopy(reason: string): string {
  if (reason.startsWith("needs_reconnect:")) return "sign in with ChatGPT again";
  if (reason.startsWith("provider_unreachable:")) return "unreachable";
  return reason;
}

/**
 * Human-facing label for an `AiRuntimeStatus.reason` code (the shared
 * engine-configured prerequisite codes, plus `user_context_disabled`), shared
 * by Settings and Insights. `labelForProvider` names a provider instance id.
 */
export function aiRuntimeReasonLabelFor(
  reason: string | null | undefined,
  labelForProvider: (id: string) => string,
): string {
  if (!reason) return "Unavailable";
  // Case-insensitive: the same codes reach this labeller both RAW (the status
  // snapshot's `reason` field) and via `humanizeError` (a command REJECTION —
  // the test-connection banner), and `humanizeError` upper-cases the first
  // letter of what it tidies, so a `startsWith` test would never match there.
  // The id after the prefix is sliced off the ORIGINAL, which keeps its case.
  const code = reason.toLowerCase();
  const idAfter = (prefix: string) => labelForProvider(reason.slice(prefix.length));
  if (code.startsWith("no_provider_key:")) {
    return `No API key saved for ${idAfter("no_provider_key:")}.`;
  }
  if (code.startsWith("provider_not_connected:")) {
    return `The default model's provider (${idAfter("provider_not_connected:")}) is not connected.`;
  }
  if (code.startsWith("needs_reconnect:")) {
    return `${idAfter("needs_reconnect:")} needs to be reconnected — sign in with ChatGPT again.`;
  }
  if (code.startsWith("provider_unreachable:")) {
    // The sign-in is intact; the auth endpoint just didn't answer. Saying
    // "reconnect" here would push the user toward Disconnect, which destroys
    // a credential that is fine.
    return `Couldn't reach ${idAfter("provider_unreachable:")} — check your connection and try again.`;
  }
  if (code.startsWith("base_url_host_mismatch:")) {
    return `The base URL for ${idAfter("base_url_host_mismatch:")} doesn't point at that provider. Fix it in Settings.`;
  }
  switch (code) {
    case "user_context_disabled": return "Continuous derivation is turned off.";
    case "ai_runtime_disabled": return "AI features are turned off.";
    case "no_providers": return "No AI providers connected yet.";
    case "no_default_model": return "Choose a global default model.";
    case "no_base_url": return "Add the base URL for the OpenAI-compatible provider.";
    case "invalid_base_url": return "The provider's base URL isn't a valid URL. Fix it in Settings.";
    case "invalid_base_url_scheme": return "The provider's base URL must start with http:// or https://.";
    case "vault_denied": return "Mnema couldn't read your saved keys from the keychain.";
    case "local_endpoint_unreachable": return "The local endpoint could not be reached.";
    default: return reason;
  }
}

/** Host portion of a base URL, or the trimmed string if it isn't a URL. */
export function baseUrlHost(baseUrl: string): string {
  const trimmed = baseUrl.trim();
  if (!trimmed) return "";
  try {
    return new URL(trimmed).host || trimmed;
  } catch {
    return trimmed;
  }
}

/**
 * Display label for a connected provider instance: the user's label if set,
 * else `Kind · host`, else `Kind (suffix)` for a 2nd+ same-kind instance.
 */
export function aiProviderInstanceLabel(provider: AiProviderConfig): string {
  const label = provider.label.trim();
  if (label) return label;
  const kindLabel = aiProviderKindLabel(provider.kind);
  const host = baseUrlHost(provider.baseUrl);
  if (host) return `${kindLabel} · ${host}`;
  const suffix = provider.id.startsWith(`${provider.kind}-`)
    ? provider.id.slice(provider.kind.length + 1)
    : "";
  return suffix ? `${kindLabel} (${suffix})` : kindLabel;
}

/**
 * Allocate a fresh instance id for a newly-added provider of `kind`. The first
 * instance of a kind keeps `id === kind` (so keys/pins recorded before instance
 * ids existed still resolve); subsequent ones get `kind-2`, `kind-3`, …
 */
export function newAiProviderId(kind: AiProviderKind, existingIds: readonly string[]): string {
  if (!existingIds.includes(kind)) return kind;
  let suffix = 2;
  let candidate = `${kind}-${suffix}`;
  while (existingIds.includes(candidate)) {
    suffix += 1;
    candidate = `${kind}-${suffix}`;
  }
  return candidate;
}

/**
 * Allocate a stable slug id for a new MCP connector, derived from its label. The
 * charset is LOAD-BEARING: a later slice parses the model-facing
 * `mcp__<id>__<tool>` prefix, so the id must be `[a-z0-9-]` only. Falls back to
 * `connector` when the label has no usable characters, and suffixes on collision
 * (`github`, `github-2`, …). Assigned once — renaming the label does NOT re-slug
 * (the id keys the keychain secret, which must stay stable).
 */
export function newMcpServerId(label: string, existingIds: readonly string[]): string {
  const base =
    label
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "connector";
  if (!existingIds.includes(base)) return base;
  let suffix = 2;
  let candidate = `${base}-${suffix}`;
  while (existingIds.includes(candidate)) {
    suffix += 1;
    candidate = `${base}-${suffix}`;
  }
  return candidate;
}
