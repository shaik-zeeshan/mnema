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
import { confirm } from "@tauri-apps/plugin-dialog";
import { humanizeError } from "$lib/format-error";
import type {
  AiProviderConfig,
  AiRuntimeStatus,
  AiRuntimeTestResult,
  McpServerConfig,
} from "$lib/types";

export interface AiRuntimeStoreDeps {
  // The current connected provider instances (page draft state).
  getProviders: () => AiProviderConfig[];
  // The current MCP tool connectors (page draft state) — for secret presence
  // refresh and the "still in the list" guard on a late save.
  getMcpServers: () => McpServerConfig[];
  // Is this provider kind a cloud provider (cloud → has a keychain key)?
  isCloudProviderKind: (kind: string) => boolean;
  // Human label for a provider instance id (resolved against the draft list).
  labelForProvider: (id: string) => string;
  // Re-check Ask AI availability after a key save/clear so its readiness pill
  // reflects the fresh runtime state without a manual Refresh. Lives in a
  // sibling store — injected, mirroring the closure contract above.
  loadAskAiAvailability: () => void;
}

export function createAiRuntimeStore(deps: AiRuntimeStoreDeps) {
  let aiRuntimeStatus = $state<AiRuntimeStatus | null>(null);
  let aiRuntimeStatusLoading = $state(false);
  let aiRuntimeStatusError = $state<string | null>(null);
  let aiProviderKeyInputs = $state<Record<string, string>>({});
  let aiProviderKeySavedByProvider = $state<Record<string, boolean>>({});
  // Provider id whose key save/clear is currently in flight (one at a time).
  let aiProviderKeySavingProvider = $state<string | null>(null);
  // Per-id in-flight guard so a rapid double-invoke for the SAME provider id
  // (e.g. save+clear, or two saves) can't race — correctness doesn't depend on
  // the UI `disabled` attribute alone. Non-reactive: it gates re-entry only, the
  // UI reads `aiProviderKeySavingProvider`. Different ids may run concurrently.
  const aiProviderKeyInFlight = new Set<string>();
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
      aiRuntimeStatusError = humanizeError(error);
    } finally {
      aiRuntimeStatusLoading = false;
    }
  }

  // Re-check which connected cloud provider instances have a key in the
  // keychain. Keyed by instance id (the keychain account).
  //
  // A failed probe is TRANSIENT, not an assertion of absence: seed `probed` from
  // the prior presence (only for ids still being probed, so removed providers
  // don't leak stale presence) and, on a probe error, DROP that id from `probed`
  // so the final merge doesn't overlay it — the existing last-known base value
  // (or any fresher value a concurrent save wrote in the meantime) survives,
  // instead of a flaky keychain read flipping a saved key to "no key saved".
  // Keeping the call-start seed would instead let the final `...probed` overlay
  // clobber a concurrent fresh `true` (a lost update). The per-id error is still
  // recorded.
  //
  // Two refreshes can overlap (e.g. two rapid `addAiProvider`s). This call only
  // owns the ids in its OWN probe set, so it MERGES those results into the
  // current map at the end rather than replacing the whole object — an id a
  // concurrent call freshly probed (but this snapshot never saw) is preserved,
  // instead of being clobbered back to absent by whichever call resolves last.
  async function refreshAiProviderKeyPresence() {
    const cloudProviderIds = deps
      .getProviders()
      .filter((p) => deps.isCloudProviderKind(p.kind))
      .map((p) => p.id);
    // Carry over last-known presence ONLY for ids in the current probe set.
    const probed: Record<string, boolean> = {};
    for (const id of cloudProviderIds) {
      if (id in aiProviderKeySavedByProvider) probed[id] = aiProviderKeySavedByProvider[id];
    }
    for (const id of cloudProviderIds) {
      try {
        probed[id] = await invoke<boolean>("ai_runtime_has_provider_key", {
          request: { provider: id },
        });
      } catch (error) {
        // Drop this id from `probed` so the final merge does NOT overlay it: a
        // concurrent save's fresh `true` (or the existing base value) survives.
        delete probed[id];
        aiProviderKeyErrors = {
          ...aiProviderKeyErrors,
          [id]: humanizeError(error),
        };
      }
    }
    // Merge per-id: overwrite only the ids THIS call probed; keep any id another
    // concurrent refresh probed in the meantime. (Removal drops a provider via
    // `clearKeyForRemovedProvider`, not here, so this never resurrects a removed
    // id — its kind is gone from `getProviders()`, so it isn't in `probed`.)
    aiProviderKeySavedByProvider = { ...aiProviderKeySavedByProvider, ...probed };
  }

  // Is this provider instance still in the draft list? A key save that lost the
  // race to a removal must NOT resurrect a key for an instance that is gone.
  function providerStillConnected(provider: string): boolean {
    return deps.getProviders().some((p) => p.id === provider);
  }

  async function saveAiProviderKey(provider: string) {
    // A save/clear for this same id is already in flight — bail so a rapid
    // double-invoke (save+clear, or two saves) can't race on the keychain.
    if (aiProviderKeyInFlight.has(provider)) return;
    const key = (aiProviderKeyInputs[provider] ?? "").trim();
    if (!key) {
      aiProviderKeyErrors = { ...aiProviderKeyErrors, [provider]: "Enter an API key first." };
      return;
    }
    // The provider may have been removed between rendering the Save button and
    // this click landing — bail before touching the keychain so a late save can
    // never write an orphaned key for a removed instance.
    if (!providerStillConnected(provider)) return;
    aiProviderKeyInFlight.add(provider);
    aiProviderKeySavingProvider = provider;
    const { [provider]: _saveErr, ...restSaveErrors } = aiProviderKeyErrors;
    aiProviderKeyErrors = restSaveErrors;
    try {
      await invoke("ai_runtime_set_provider_key", { request: { provider, key } });
      aiProviderKeyInputs = { ...aiProviderKeyInputs, [provider]: "" };
      // The key just changed for this provider, so any prior "Connection
      // succeeded" banner no longer reflects the config that would be tested.
      resetTestResult();
      await refreshAiProviderKeyPresence();
      await loadAiRuntimeStatus();
      deps.loadAskAiAvailability();
    } catch (error) {
      aiProviderKeyErrors = {
        ...aiProviderKeyErrors,
        [provider]: humanizeError(error),
      };
    } finally {
      aiProviderKeyInFlight.delete(provider);
      aiProviderKeySavingProvider = null;
    }
  }

  async function clearAiProviderKey(provider: string) {
    // A save/clear for this same id is already in flight — bail so a rapid
    // double-invoke (save+clear, or two clears) can't race on the keychain.
    if (aiProviderKeyInFlight.has(provider)) return;
    // Clearing deletes the keychain credential with no undo — gate it on an
    // explicit confirm, matching the Remove-provider flow, so a single mis-click
    // can't wipe a saved API key. Arm the in-flight latch BEFORE the awaited
    // dialog so a second click can't open a second dialog mid-confirm; release it
    // if the user cancels.
    aiProviderKeyInFlight.add(provider);
    try {
      const confirmed = await confirm(
        `Deleting the API key for “${deps.labelForProvider(provider)}” removes it from the macOS keychain right away. Any AI feature using this provider will stop working until you enter a new key.`,
        {
          title: "Delete this API key?",
          kind: "warning",
          okLabel: "Delete Key",
          cancelLabel: "Keep Key",
        },
      );
      if (!confirmed) return;
    } catch {
      // A dialog failure must not silently delete the key — bail.
      return;
    } finally {
      // The block below re-arms the latch for the actual keychain call; release
      // it here so a cancel/dialog-error path doesn't leave it stuck.
      aiProviderKeyInFlight.delete(provider);
    }
    if (aiProviderKeyInFlight.has(provider)) return;
    aiProviderKeyInFlight.add(provider);
    aiProviderKeySavingProvider = provider;
    const { [provider]: _clearErr, ...restClearErrors } = aiProviderKeyErrors;
    aiProviderKeyErrors = restClearErrors;
    try {
      await invoke("ai_runtime_clear_provider_key", { request: { provider } });
      aiProviderKeyInputs = { ...aiProviderKeyInputs, [provider]: "" };
      // Clearing the key invalidates any prior "Connection succeeded" banner.
      resetTestResult();
      await refreshAiProviderKeyPresence();
      await loadAiRuntimeStatus();
      deps.loadAskAiAvailability();
    } catch (error) {
      aiProviderKeyErrors = {
        ...aiProviderKeyErrors,
        [provider]: humanizeError(error),
      };
    } finally {
      aiProviderKeyInFlight.delete(provider);
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
  //
  // The error is a single visible slot, so do NOT reset it at the start of every
  // attempt: clearing it on entry would silently wipe an earlier removal's
  // orphaned-key warning the moment a later removal succeeds. Instead clear it
  // only on a SUCCESSFUL clear and set it — naming the affected provider so the
  // warning is identifiable — on failure, keeping a genuine failure visible until
  // it is resolved. The label is resolved BEFORE the await (the instance is
  // already gone from the draft list, so resolve it while it may still match).
  async function clearKeyForRemovedProvider(id: string): Promise<void> {
    const label = deps.labelForProvider(id);
    try {
      await invoke("ai_runtime_clear_provider_key", { request: { provider: id } });
      const { [id]: _cleared, ...rest } = aiProviderKeySavedByProvider;
      aiProviderKeySavedByProvider = rest;
      aiProviderRemovalError = null;
    } catch (error) {
      const message = humanizeError(error);
      aiProviderRemovalError = `Could not clear the saved key for the removed provider ${label} — it may still be in the keychain. ${message}`;
    }
  }

  // ─── MCP connector secrets ──────────────────────────────────────────────────
  // The single optional secret per MCP server, keyed by server instance id (the
  // keychain account). MCP ids are unique slugs assigned once at creation and
  // never reused, so this needs none of the same-kind re-add race handling the
  // provider-key store above carries — a per-id in-flight guard + confirm-on-clear
  // is enough.
  let mcpSecretInputs = $state<Record<string, string>>({});
  let mcpSecretSavedById = $state<Record<string, boolean>>({});
  let mcpSecretSavingId = $state<string | null>(null);
  let mcpSecretErrors = $state<Record<string, string>>({});
  const mcpSecretInFlight = new Set<string>();

  // Re-check which MCP connectors have a secret in the keychain (keyed by id).
  async function refreshMcpServerSecretPresence() {
    const ids = deps.getMcpServers().map((s) => s.id);
    const probed: Record<string, boolean> = {};
    for (const id of ids) {
      try {
        probed[id] = await invoke<boolean>("mcp_has_server_secret", { request: { id } });
      } catch (error) {
        mcpSecretErrors = { ...mcpSecretErrors, [id]: humanizeError(error) };
      }
    }
    mcpSecretSavedById = probed;
  }

  // Probe for a Node runtime on the user's login-shell PATH (local stdio
  // presets spawn via npx). Resolves to the version string ("v22.11.0") or
  // null when Node is missing.
  function checkNode(): Promise<string | null> {
    return invoke<string | null>("mcp_check_node");
  }

  function mcpServerStillPresent(id: string): boolean {
    return deps.getMcpServers().some((s) => s.id === id);
  }

  async function saveMcpServerSecret(id: string) {
    if (mcpSecretInFlight.has(id)) return;
    const secret = (mcpSecretInputs[id] ?? "").trim();
    if (!secret) {
      mcpSecretErrors = { ...mcpSecretErrors, [id]: "Enter a secret first." };
      return;
    }
    // The server may have been removed between render and this click — bail
    // before touching the keychain so a late save can't orphan a secret.
    if (!mcpServerStillPresent(id)) return;
    mcpSecretInFlight.add(id);
    mcpSecretSavingId = id;
    const { [id]: _saveErr, ...restErrors } = mcpSecretErrors;
    mcpSecretErrors = restErrors;
    try {
      await invoke("mcp_set_server_secret", { request: { id, secret } });
      mcpSecretInputs = { ...mcpSecretInputs, [id]: "" };
      await refreshMcpServerSecretPresence();
    } catch (error) {
      mcpSecretErrors = { ...mcpSecretErrors, [id]: humanizeError(error) };
    } finally {
      mcpSecretInFlight.delete(id);
      mcpSecretSavingId = null;
    }
  }

  async function clearMcpServerSecret(id: string) {
    if (mcpSecretInFlight.has(id)) return;
    // Deleting the keychain secret has no undo — gate on an explicit confirm,
    // matching the provider-key flow. Arm the latch before the awaited dialog.
    mcpSecretInFlight.add(id);
    try {
      const confirmed = await confirm(
        `Deleting the saved secret for this connector removes it from the macOS keychain right away. The connector will stop authenticating until you enter a new secret.`,
        {
          title: "Delete this secret?",
          kind: "warning",
          okLabel: "Delete Secret",
          cancelLabel: "Keep Secret",
        },
      );
      if (!confirmed) return;
    } catch {
      return;
    } finally {
      mcpSecretInFlight.delete(id);
    }
    if (mcpSecretInFlight.has(id)) return;
    mcpSecretInFlight.add(id);
    mcpSecretSavingId = id;
    const { [id]: _clearErr, ...restErrors } = mcpSecretErrors;
    mcpSecretErrors = restErrors;
    try {
      await invoke("mcp_clear_server_secret", { request: { id } });
      mcpSecretInputs = { ...mcpSecretInputs, [id]: "" };
      await refreshMcpServerSecretPresence();
    } catch (error) {
      mcpSecretErrors = { ...mcpSecretErrors, [id]: humanizeError(error) };
    } finally {
      mcpSecretInFlight.delete(id);
      mcpSecretSavingId = null;
    }
  }

  // Clear the keychain secret for an MCP connector the user just removed. Best
  // effort: a failure only leaves an orphaned secret (recorded under an id no
  // longer rendered), never blocks the removal.
  async function clearSecretForRemovedMcpServer(id: string): Promise<void> {
    try {
      await invoke("mcp_clear_server_secret", { request: { id } });
    } catch {
      // The secret is orphaned in the keychain; nothing renders this id anymore.
    }
    const { [id]: _cleared, ...rest } = mcpSecretSavedById;
    mcpSecretSavedById = rest;
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
      aiRuntimeTestError = humanizeError(error);
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
    // MCP connector secrets.
    get mcpSecretInputs() { return mcpSecretInputs; },
    setMcpSecretInput(id: string, value: string) {
      mcpSecretInputs = { ...mcpSecretInputs, [id]: value };
    },
    get mcpSecretSavedById() { return mcpSecretSavedById; },
    get mcpSecretSavingId() { return mcpSecretSavingId; },
    get mcpSecretErrors() { return mcpSecretErrors; },
    refreshMcpServerSecretPresence,
    saveMcpServerSecret,
    clearMcpServerSecret,
    clearSecretForRemovedMcpServer,
    checkNode,
  };
}

export type AiRuntimeStore = ReturnType<typeof createAiRuntimeStore>;
