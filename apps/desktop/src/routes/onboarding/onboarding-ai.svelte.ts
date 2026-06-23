// Onboarding Reasoning-Engine setup subsystem (Ask AI provider config).
//
// First-run onboarding can't deep-link into Settings → Intelligence to connect
// an AI provider: Settings is a `/settings` route in the MAIN window, which
// doesn't open until onboarding completes. So onboarding has to let the user
// connect a provider, save its key, and pick a default model INLINE — this
// subsystem owns that state, mirroring the Settings page's Providers panel but
// scoped to the onboarding flow.
//
// It reuses the SAME building blocks the Settings page uses so the two never
// drift: `createAiRuntimeStore` (keychain key save/clear/presence), the shared
// `ModelPoolLoader` + `ModelPickerMenu`, and the pure provider helpers in
// `ai-providers.ts`. The connected-provider LIST + chosen default model are
// draft state here; they are committed as the `aiRuntime` domain of the whole
// `RecordingSettings` at `finish()` (see OnboardingController.buildSettingsRequest).
//
// NOTE on validation surface: `ai_runtime_list_models` lists from the draft
// provider configs passed in the request (it does NOT need the providers
// persisted first), so the model picker validates a saved key live during
// onboarding. `get_ai_runtime_status` / `ai_runtime_test_connection` read the
// PERSISTED settings, which are still empty mid-onboarding — so this subsystem
// deliberately omits the runtime-status / test-connection surfaces. The
// "✓ key in keychain" badge plus a successful model listing are the signal.
import { ModelPoolLoader } from "$lib/insights/modelPool.svelte";
import { createAiRuntimeStore } from "$lib/settings/state/ai-runtime.svelte";
import {
  AI_PROVIDER_KINDS,
  AI_LOCAL_DEFAULT_ENDPOINTS,
  isCloudAiProviderKind,
  aiProviderKindLabel,
  aiProviderKindDescription,
  aiProviderInstanceLabel,
  newAiProviderId,
} from "$lib/settings/state/ai-providers";
import type { AiEngineRef, AiProviderConfig, AiProviderKind } from "$lib/types";

export function createOnboardingAiStore() {
  // ── Draft state (committed as the aiRuntime domain at finish) ─────────────
  let draftAiProviders = $state<AiProviderConfig[]>([]);
  let draftAiDefaultModel = $state<AiEngineRef | null>(null);
  // ModelPickerMenu open state (bind:open).
  let aiModelOpen = $state(false);

  // Shared incremental model-pool loader (one list call per provider).
  const modelLoader = new ModelPoolLoader();

  // ── Provider label resolution (against the live draft list) ───────────────
  function providerById(id: string): AiProviderConfig | undefined {
    return draftAiProviders.find((p) => p.id === id);
  }
  function aiProviderLabelById(id: string): string {
    const provider = providerById(id);
    return provider ? aiProviderInstanceLabel(provider) : aiProviderKindLabel(id);
  }

  // Keychain key save/clear/presence — the same store the Settings page uses,
  // wired against this flow's draft provider list via injected closures.
  const aiRuntime = createAiRuntimeStore({
    getProviders: () => draftAiProviders,
    isCloudProviderKind: (kind) => isCloudAiProviderKind(kind),
    labelForProvider: (id) => aiProviderLabelById(id),
  });

  // ── Provider list mutations (mirror the Settings controller) ──────────────
  function addProvider(kind: AiProviderKind): void {
    const existingIds = draftAiProviders.map((p) => p.id);
    draftAiProviders = [
      ...draftAiProviders,
      { id: newAiProviderId(kind, existingIds), kind, label: "", baseUrl: "" },
    ];
    void aiRuntime.refreshAiProviderKeyPresence();
  }

  function removeProvider(id: string): void {
    const removed = providerById(id);
    draftAiProviders = draftAiProviders.filter((p) => p.id !== id);
    if (draftAiDefaultModel?.provider === id) {
      draftAiDefaultModel = null;
    }
    if (removed && isCloudAiProviderKind(removed.kind)) {
      aiRuntime.clearKeyForRemovedProvider(id);
    }
  }

  async function loadModels(): Promise<void> {
    await modelLoader.load(draftAiProviders);
  }

  // Re-seed the draft list from canonical settings (onboarding round-trips the
  // whole RecordingSettings, so the post-save reload re-syncs through here).
  function syncFromSettings(providers: AiProviderConfig[], defaultModel: AiEngineRef | null): void {
    draftAiProviders = providers.map((p) => ({
      id: p.id && p.id.trim().length > 0 ? p.id : p.kind,
      kind: p.kind,
      label: p.label ?? "",
      baseUrl: p.baseUrl ?? "",
    }));
    draftAiDefaultModel = defaultModel
      ? { provider: defaultModel.provider, model: defaultModel.model }
      : null;
  }

  // Refresh which connected cloud providers already have a saved key (e.g. when
  // the user re-opens onboarding after a partial setup). No-op on a clean run.
  function init(): void {
    void aiRuntime.refreshAiProviderKeyPresence();
  }

  // ── Derived view state ────────────────────────────────────────────────────
  const anyCloudConnected = $derived(draftAiProviders.some((p) => isCloudAiProviderKind(p.kind)));

  // Single source of truth for "Ask AI is usable". Ask AI can only run if a
  // chosen default model can actually resolve: a default model is selected, its
  // provider exists in the draft list, and that provider is itself configured
  // (cloud → key saved; openai_compatible → key saved + base URL; local → fine).
  // `aiConfigMissing` is the PRIMARY computation (returns the short human reason,
  // or null when ready) and `aiConfigReady` is derived from it, so the boolean
  // and the explanation can never drift. Both the attention rule
  // (OnboardingController.featureAttention) and AskAiBody read these — the
  // condition lives ONLY here.
  const aiConfigMissing = $derived.by<string | null>(() => {
    if (draftAiProviders.length === 0) return "Connect a reasoning engine to use Ask AI.";
    const model = draftAiDefaultModel;
    if (!model || model.model.trim().length === 0) return "Choose a default model for Ask AI.";
    const provider = providerById(model.provider);
    if (!provider) return "Pick a default model from a connected provider.";
    if (provider.kind === "openai_compatible" && provider.baseUrl.trim().length === 0) {
      return `Set the base URL for ${aiProviderLabelById(provider.id)}.`;
    }
    if (isCloudAiProviderKind(provider.kind) && !aiRuntime.aiProviderKeySavedByProvider[provider.id]) {
      return `Save the API key for ${aiProviderLabelById(provider.id)}.`;
    }
    return null;
  });
  const aiConfigReady = $derived(aiConfigMissing === null);
  const aiModelValue = $derived.by(() => {
    const ref = draftAiDefaultModel;
    if (!ref || ref.model.trim().length === 0) return "";
    return `${aiProviderLabelById(ref.provider)} · ${ref.model}`;
  });
  // Provider rows that failed to list models last fetch (→ ModelPickerMenu retry).
  const modelFailureRows = $derived(
    modelLoader.failures.map((f) => ({
      provider: f.provider,
      label: aiProviderLabelById(f.provider),
      reason: f.reason,
    })),
  );
  const modelRetryTargets = $derived(
    draftAiProviders.filter((p) => modelLoader.failures.some((f) => f.provider === p.id)),
  );
  const modelsError = $derived(
    modelLoader.failures.length > 0
      ? modelLoader.failures
          .map((f) => `${aiProviderLabelById(f.provider)}: ${f.reason}`)
          .join("; ")
      : null,
  );

  return {
    // Re-exported constants/helpers the markup references verbatim.
    AI_PROVIDER_KINDS,
    AI_LOCAL_DEFAULT_ENDPOINTS,
    isCloudAiProviderKind,
    aiProviderKindLabel,
    aiProviderKindDescription,
    aiProviderInstanceLabel,

    // Stores.
    aiRuntime,
    modelLoader,

    // Bindable draft state.
    get draftAiProviders() { return draftAiProviders; },
    set draftAiProviders(value: AiProviderConfig[]) { draftAiProviders = value; },
    get draftAiDefaultModel() { return draftAiDefaultModel; },
    set draftAiDefaultModel(value: AiEngineRef | null) { draftAiDefaultModel = value; },
    get aiModelOpen() { return aiModelOpen; },
    set aiModelOpen(value: boolean) { aiModelOpen = value; },

    // Derived view state.
    get anyCloudConnected() { return anyCloudConnected; },
    get aiConfigReady() { return aiConfigReady; },
    get aiConfigMissing() { return aiConfigMissing; },
    get aiModelValue() { return aiModelValue; },
    get modelFailureRows() { return modelFailureRows; },
    get modelRetryTargets() { return modelRetryTargets; },
    get modelsError() { return modelsError; },

    // Actions.
    addProvider,
    removeProvider,
    loadModels,
    syncFromSettings,
    init,
    aiProviderLabelById,
  };
}

export type OnboardingAiStore = ReturnType<typeof createOnboardingAiStore>;
