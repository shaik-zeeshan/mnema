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
  "openai_compatible",
  "ollama",
  "llamafile",
];

/** Kinds that talk to a hosted API and so store a key in the OS keychain. */
export const CLOUD_AI_PROVIDER_KINDS: readonly AiProviderKind[] = [
  "anthropic",
  "openai",
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
    case "openai_compatible": return "Fireworks, OpenRouter, Together — custom base URL + key";
    case "ollama": return "Local runtime, default endpoint http://localhost:11434";
    case "llamafile": return "Local OpenAI-compatible server, default http://localhost:8080";
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
