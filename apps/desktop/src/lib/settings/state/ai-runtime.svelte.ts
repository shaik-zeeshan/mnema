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
  // Error from clearing the keychain key of a provider the user just removed.
  // Recorded against a STILL-VISIBLE surface (not `aiProviderKeyErrors`, whose
  // entry is keyed by an id no longer in the list and would never render), so a
  // genuine clear failure — which orphans the key — is at least seen by the user.
  let aiProviderRemovalError = $state<string | null>(null);

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
  //
  // A failed probe is TRANSIENT, not an assertion of absence: seed `next` from
  // the prior presence (only for ids still being probed, so removed providers
  // don't leak stale presence) and, on a probe error, keep that id's last-known
  // value instead of dropping it — otherwise a provider that genuinely has a
  // saved key would flip to "no key saved" (and the UI prompt to re-add it) on
  // any flaky keychain read. The per-id error is still recorded.
  async function refreshAiProviderKeyPresence() {
    const cloudProviderIds = deps
      .getProviders()
      .filter((p) => deps.isCloudProviderKind(p.kind))
      .map((p) => p.id);
    // Carry over last-known presence ONLY for ids in the current probe set.
    const next: Record<string, boolean> = {};
    for (const id of cloudProviderIds) {
      if (id in aiProviderKeySavedByProvider) next[id] = aiProviderKeySavedByProvider[id];
    }
    for (const id of cloudProviderIds) {
      try {
        next[id] = await invoke<boolean>("ai_runtime_has_provider_key", {
          request: { provider: id },
        });
      } catch (error) {
        // Leave the seeded last-known presence for `id` intact; record the error.
        aiProviderKeyErrors = {
          ...aiProviderKeyErrors,
          [id]: error instanceof Error ? error.message : String(error),
        };
      }
    }
    aiProviderKeySavedByProvider = next;
  }

  // Is this provider instance still in the draft list? A key save that lost the
  // race to a removal must NOT resurrect a key for an instance that is gone.
  function providerStillConnected(provider: string): boolean {
    return deps.getProviders().some((p) => p.id === provider);
  }

  async function saveAiProviderKey(provider: string) {
    const key = (aiProviderKeyInputs[provider] ?? "").trim();
    if (!key) {
      aiProviderKeyErrors = { ...aiProviderKeyErrors, [provider]: "Enter an API key first." };
      return;
    }
    // The provider may have been removed between rendering the Save button and
    // this click landing — bail before touching the keychain so a late save can
    // never write an orphaned key for a removed instance.
    if (!providerStillConnected(provider)) return;
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
  // the draft list, and drop its presence entry. AWAITED (not fire-and-forget):
  // the first instance of a kind reuses the bare kind as its id, so removing then
  // immediately re-adding the same kind must not race an in-flight clear — the
  // re-add probes only after this resolves (the caller awaits). On a genuine
  // FAILURE the key is orphaned in the keychain; we surface that on a visible
  // error so the user can retry, instead of recording it against a now-absent id.
  async function clearKeyForRemovedProvider(id: string): Promise<void> {
    aiProviderRemovalError = null;
    try {
      await invoke("ai_runtime_clear_provider_key", { request: { provider: id } });
      const { [id]: _cleared, ...rest } = aiProviderKeySavedByProvider;
      aiProviderKeySavedByProvider = rest;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      aiProviderRemovalError = `Could not clear the saved key for the removed provider — it may still be in the keychain. ${message}`;
    }
  }

  // Clear the last test-connection banner (result + error). The banner reports
  // the provider/model that was tested; after the user changes the default model
  // or removes the tested provider it no longer reflects the live config, so the
  // callers that mutate either reset it to avoid showing a stale "succeeded" line.
  function resetTestResult() {
    aiRuntimeTestResult = null;
    aiRuntimeTestError = null;
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
    get aiProviderRemovalError() { return aiProviderRemovalError; },
    aiRuntimeReasonLabel,
    loadAiRuntimeStatus,
    refreshAiProviderKeyPresence,
    saveAiProviderKey,
    clearAiProviderKey,
    clearKeyForRemovedProvider,
    resetTestResult,
    runAiRuntimeTestConnection,
  };
}

export type AiRuntimeStore = ReturnType<typeof createAiRuntimeStore>;
