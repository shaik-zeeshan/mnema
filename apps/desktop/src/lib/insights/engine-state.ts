// What Insights says about the Reasoning Engine (Direction A, slice 2): one pure
// reading of `AiRuntimeStatus` + `AskAiAvailability` shared by the page gate, the
// rail footer, Chat's composer slot and Chat's failed-turn line, so no surface
// shows a raw reason code or pitches setup to someone whose engine merely broke.

import type { AiRuntimeSettings, AiRuntimeStatus } from "$lib/types/recording";
import {
  AI_LOCAL_DEFAULT_ENDPOINTS,
  aiProviderInstanceLabel,
  aiProviderKindLabel,
  aiRuntimeReasonLabelFor,
  baseUrlHost,
} from "$lib/settings/state/ai-providers";
import type { TurnErrorKind } from "$lib/insights/conversation";

export type EngineState =
  | { kind: "loading" }
  /** Never set up — the full-page pitch. */
  | { kind: "pitch" }
  /** AI turned off on purpose — a slim line, content stays. */
  | { kind: "off" }
  | { kind: "on" }
  /** Couldn't reach the engine (or the keychain) — Retry. `short` is the rail's. */
  | { kind: "unreachable"; text: string; short: string }
  /** A named fix. `reconnectProviderId` is set for a rejected ChatGPT login. */
  | { kind: "fix"; text: string; short: string; reconnectProviderId: string | null };

function labelFor(settings: AiRuntimeSettings | null) {
  return (id: string): string => {
    const provider = settings?.providers.find((p) => p.id === id);
    return provider ? aiProviderInstanceLabel(provider) : aiProviderKindLabel(id);
  };
}

/** "Ollama at localhost:11434", named from the default model's provider. */
export function localEndpointName(settings: AiRuntimeSettings | null): string {
  const id = settings?.defaultModel?.provider;
  const provider = settings?.providers.find((p) => p.id === id);
  if (!provider) return "the local engine";
  const host = baseUrlHost(provider.baseUrl || AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] || "");
  const kind = aiProviderKindLabel(provider.kind);
  return host ? `${kind} at ${host}` : kind;
}

/** Copy for a reason code that isn't a plain "on/off" (shared with Chat). */
export function reasonCopy(
  reason: string,
  settings: AiRuntimeSettings | null,
): Exclude<EngineState, { kind: "loading" | "pitch" | "off" | "on" }> {
  const label = labelFor(settings);
  if (reason === "local_endpoint_unreachable") {
    const name = localEndpointName(settings);
    return {
      kind: "unreachable",
      text: `Can't reach ${name}. Make sure it's running.`,
      short: `can't reach ${name.split(" at ")[0].toLowerCase()}`,
    };
  }
  if (reason.startsWith("provider_unreachable:")) {
    const name = label(reason.slice("provider_unreachable:".length));
    return {
      kind: "unreachable",
      text: `Can't reach ${name} right now — your sign-in is fine.`,
      short: `can't reach ${name.toLowerCase()}`,
    };
  }
  if (reason === "vault_denied") {
    return {
      kind: "unreachable",
      text: "Mnema couldn't read your saved keys from the keychain. Allow access, then try again.",
      short: "keychain locked",
    };
  }
  if (reason.startsWith("needs_reconnect:")) {
    const id = reason.slice("needs_reconnect:".length);
    return {
      kind: "fix",
      text: `${label(id)} signed you out. Sign in again to keep going.`,
      short: label(id).toLowerCase(),
      reconnectProviderId: id,
    };
  }
  return {
    kind: "fix",
    text: aiRuntimeReasonLabelFor(reason, label),
    short: "engine needs attention",
    reconnectProviderId: null,
  };
}

/** The 4-step page rule (independent of `configured`). */
export function engineState(
  loaded: boolean,
  status: AiRuntimeStatus | null,
  settings: AiRuntimeSettings | null,
): EngineState {
  if (!loaded) return { kind: "loading" };
  if (status === null) {
    return { kind: "unreachable", text: "Couldn't check the engine's status.", short: "status unknown" };
  }
  if (!status.hasProviders) return { kind: "pitch" };
  if (!status.enabled) return { kind: "off" };
  if (status.available || !status.reason) return { kind: "on" };
  return reasonCopy(status.reason, settings);
}

/** The one action a failed Chat turn offers. */
export type TurnErrorAction = "reconnect" | "retry" | "settings" | "new_chat";

/** Map a failed turn's message (maybe a raw resolve code) + optional `kind`
 *  to plain copy and one action. Never returns a raw code. */
export function turnErrorCopy(
  message: string | null,
  kind: TurnErrorKind | null | undefined,
  settings: AiRuntimeSettings | null,
): { text: string; action: TurnErrorAction; reconnectProviderId: string | null } {
  const raw = (message ?? "").trim();
  // Raw resolve codes: `snake_case` (optionally `:<id>`), no spaces.
  if (/^[a-z_]+(:\S+)?$/.test(raw)) {
    const copy = reasonCopy(raw, settings);
    if (copy.kind === "unreachable") return { text: copy.text, action: "retry", reconnectProviderId: null };
    if (copy.reconnectProviderId !== null) {
      return {
        text: `${labelFor(settings)(copy.reconnectProviderId)} signed you out. Sign in again to keep chatting — retrying won't help until then.`,
        action: "reconnect",
        reconnectProviderId: copy.reconnectProviderId,
      };
    }
    return { text: copy.text, action: "settings", reconnectProviderId: null };
  }
  const text = raw || "The engine couldn't answer.";
  switch (kind) {
    case "settings":
    case "auth":
      return { text, action: "settings", reconnectProviderId: null };
    case "context_too_long":
      return { text, action: "new_chat", reconnectProviderId: null };
    default:
      return { text, action: "retry", reconnectProviderId: null };
  }
}
