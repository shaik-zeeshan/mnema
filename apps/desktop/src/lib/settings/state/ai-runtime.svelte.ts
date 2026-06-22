// Reasoning Engine availability + per-provider API key state (ADR 0034/0035).
//
// The connected-provider LIST (`draftAiProviders`) and the default model are
// draft state that binds in the page markup and autosaves through the engine —
// they stay in the page. This store owns everything that is NOT a draft: the
// runtime availability snapshot, the keychain presence/inputs/errors per
// provider instance, and the connection-test result.
//
// Because key presence + label resolution need the current provider list, the
// store takes them as INJECTED closures (`getProviders`, `isCloudProviderKind`,
// `labelForProvider`) rather than importing draft state — mirroring the engine's
// injected-closure contract. When the provider list later moves out of the page
// into its own draft module, only the injected closures change.

import { invoke } from "@tauri-apps/api/core";
import type {
  AiProviderConfig,
  AiRuntimeStatus,
  AiRuntimeTestResult,
} from "$lib/types";

export interface AiRuntimeStoreDeps {
  // The current connected provider instances (page draft state).
  getProviders: () => AiProviderConfig[];
  // Is this provider kind a cloud provider (cloud → has a keychain key)?
  isCloudProviderKind: (kind: string) => boolean;
  // Human label for a provider instance id (resolved against the draft list).
  labelForProvider: (id: string) => string;
}

export function createAiRuntimeStore(deps: AiRuntimeStoreDeps) {
  let aiRuntimeStatus = $state<AiRuntimeStatus | null>(null);
  let aiRuntimeStatusLoading = $state(false);
  let aiRuntimeStatusError = $state<string | null>(null);
  let aiProviderKeyInputs = $state<Record<string, string>>({});
  let aiProviderKeySavedByProvider = $state<Record<string, boolean>>({});
  // Provider id whose key save/clear is currently in flight (one at a time).
  let aiProviderKeySavingProvider = $state<string | null>(null);
  let aiProviderKeyErrors = $state<Record<string, string>>({});
  let aiRuntimeTestRunning = $state(false);
  let aiRuntimeTestResult = $state<AiRuntimeTestResult | null>(null);
  let aiRuntimeTestError = $state<string | null>(null);

  // Human-facing label for an AiRuntimeStatus.reason code (the shared
  // engine-configured prerequisite codes, plus user_context_disabled).
  function aiRuntimeReasonLabel(reason: string | null | undefined): string {
    if (!reason) return "Unavailable";
    if (reason.startsWith("no_provider_key:")) {
      const provider = reason.slice("no_provider_key:".length);
      return `No API key saved for ${deps.labelForProvider(provider)}.`;
    }
    if (reason.startsWith("provider_not_connected:")) {
      const provider = reason.slice("provider_not_connected:".length);
      return `The default model's provider (${deps.labelForProvider(provider)}) is not connected.`;
    }
    switch (reason) {
      case "user_context_disabled": return "Continuous derivation is turned off.";
      case "ai_runtime_disabled": return "AI features are turned off.";
      case "no_providers": return "No AI providers connected yet.";
      case "no_default_model": return "Choose a global default model.";
      case "no_base_url": return "Add the base URL for the OpenAI-compatible provider.";
      case "local_endpoint_unreachable": return "The local endpoint could not be reached.";
      default: return reason;
    }
  }

  async function loadAiRuntimeStatus() {
    aiRuntimeStatusLoading = true;
    aiRuntimeStatusError = null;
    try {
      aiRuntimeStatus = await invoke<AiRuntimeStatus>("get_ai_runtime_status");
    } catch (error) {
      aiRuntimeStatusError = error instanceof Error ? error.message : String(error);
    } finally {
      aiRuntimeStatusLoading = false;
    }
  }

  // Re-check which connected cloud provider instances have a key in the
  // keychain. Keyed by instance id (the keychain account).
  async function refreshAiProviderKeyPresence() {
    const cloudProviderIds = deps
      .getProviders()
      .filter((p) => deps.isCloudProviderKind(p.kind))
      .map((p) => p.id);
    const next: Record<string, boolean> = {};
    for (const id of cloudProviderIds) {
      try {
        next[id] = await invoke<boolean>("ai_runtime_has_provider_key", {
          request: { provider: id },
        });
      } catch (error) {
        aiProviderKeyErrors = {
          ...aiProviderKeyErrors,
          [id]: error instanceof Error ? error.message : String(error),
        };
      }
    }
    aiProviderKeySavedByProvider = next;
  }

  async function saveAiProviderKey(provider: string) {
    const key = (aiProviderKeyInputs[provider] ?? "").trim();
    if (!key) {
      aiProviderKeyErrors = { ...aiProviderKeyErrors, [provider]: "Enter an API key first." };
      return;
    }
    aiProviderKeySavingProvider = provider;
    const { [provider]: _saveErr, ...restSaveErrors } = aiProviderKeyErrors;
    aiProviderKeyErrors = restSaveErrors;
    try {
      await invoke("ai_runtime_set_provider_key", { request: { provider, key } });
      aiProviderKeyInputs = { ...aiProviderKeyInputs, [provider]: "" };
      await refreshAiProviderKeyPresence();
      await loadAiRuntimeStatus();
    } catch (error) {
      aiProviderKeyErrors = {
        ...aiProviderKeyErrors,
        [provider]: error instanceof Error ? error.message : String(error),
      };
    } finally {
      aiProviderKeySavingProvider = null;
    }
  }

  async function clearAiProviderKey(provider: string) {
    aiProviderKeySavingProvider = provider;
    const { [provider]: _clearErr, ...restClearErrors } = aiProviderKeyErrors;
    aiProviderKeyErrors = restClearErrors;
    try {
      await invoke("ai_runtime_clear_provider_key", { request: { provider } });
      aiProviderKeyInputs = { ...aiProviderKeyInputs, [provider]: "" };
      await refreshAiProviderKeyPresence();
      await loadAiRuntimeStatus();
    } catch (error) {
      aiProviderKeyErrors = {
        ...aiProviderKeyErrors,
        [provider]: error instanceof Error ? error.message : String(error),
      };
    } finally {
      aiProviderKeySavingProvider = null;
    }
  }

  // Clear the keychain key for a provider instance the user just removed from
  // the draft list, and drop its presence entry. Best-effort: a clear failure
  // shouldn't block the remove (the caller already removed it from the draft).
  function clearKeyForRemovedProvider(id: string) {
    void invoke("ai_runtime_clear_provider_key", { request: { provider: id } })
      .then(() => {
        const { [id]: _orphaned, ...rest } = aiProviderKeySavedByProvider;
        aiProviderKeySavedByProvider = rest;
      })
      .catch((error) => {
        aiProviderKeyErrors = {
          ...aiProviderKeyErrors,
          [id]: error instanceof Error ? error.message : String(error),
        };
      });
  }

  async function runAiRuntimeTestConnection() {
    aiRuntimeTestRunning = true;
    aiRuntimeTestError = null;
    aiRuntimeTestResult = null;
    try {
      aiRuntimeTestResult = await invoke<AiRuntimeTestResult>("ai_runtime_test_connection");
    } catch (error) {
      aiRuntimeTestError = error instanceof Error ? error.message : String(error);
    } finally {
      aiRuntimeTestRunning = false;
      void loadAiRuntimeStatus();
    }
  }

  return {
    get aiRuntimeStatus() { return aiRuntimeStatus; },
    get aiRuntimeStatusLoading() { return aiRuntimeStatusLoading; },
    get aiRuntimeStatusError() { return aiRuntimeStatusError; },
    get aiProviderKeyInputs() { return aiProviderKeyInputs; },
    setProviderKeyInput(provider: string, value: string) {
      aiProviderKeyInputs = { ...aiProviderKeyInputs, [provider]: value };
    },
    get aiProviderKeySavedByProvider() { return aiProviderKeySavedByProvider; },
    get aiProviderKeySavingProvider() { return aiProviderKeySavingProvider; },
    get aiProviderKeyErrors() { return aiProviderKeyErrors; },
    get aiRuntimeTestRunning() { return aiRuntimeTestRunning; },
    get aiRuntimeTestResult() { return aiRuntimeTestResult; },
    get aiRuntimeTestError() { return aiRuntimeTestError; },
    aiRuntimeReasonLabel,
    loadAiRuntimeStatus,
    refreshAiProviderKeyPresence,
    saveAiProviderKey,
    clearAiProviderKey,
    clearKeyForRemovedProvider,
    runAiRuntimeTestConnection,
  };
}

export type AiRuntimeStore = ReturnType<typeof createAiRuntimeStore>;
