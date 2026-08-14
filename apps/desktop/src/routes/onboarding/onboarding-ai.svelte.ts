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
// NOTE on validation surface: `verify_ai_provider` and `ai_runtime_list_models`
// both accept the DRAFT provider configs in the request (they do not need the
// providers persisted first), so a provider can be verified live during
// onboarding. `get_ai_runtime_status` / `ai_runtime_test_connection` read the
// PERSISTED settings, which are still empty mid-onboarding — so this subsystem
// deliberately omits the runtime-status / test-connection surfaces.
//
// READINESS (slice 11): "a string reached the keychain" is NOT configured. A
// provider counts only once `verify_ai_provider` listed its models live THIS
// session and that listing still contains the chosen model id — no kind is
// exempt, so an Ollama that is not answering blocks exactly like a rejected
// cloud key. The rule itself is the pure `$lib/onboarding/ai-readiness`.
import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import { ModelPoolLoader } from "$lib/insights/modelPool.svelte";
import {
  aiReadinessMissing,
  type AiVerification,
} from "$lib/onboarding/ai-readiness";
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
  // True while a provider removal (incl. its awaited keychain clear) is in
  // flight. The add-provider control reads this and stays disabled so a new
  // provider can't be added mid-clear and race a same-kind id re-add (ADR 0035)
  // into a false "key in keychain" probe.
  let aiProviderRemoving = $state(false);
  // Live verification result per provider instance id (ADR 0035). Session-only
  // and deliberately NOT persisted: it is evidence that the engine answered a
  // moment ago, so editing a key/endpoint or re-seeding the draft drops it.
  let aiVerifications = $state<Record<string, AiVerification>>({});
  // Visible reason a persisted default model was cleared on re-entry (the
  // provider no longer lists it). Null on a clean run.
  let aiRestoredModelNote = $state<string | null>(null);

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
    // Onboarding doesn't configure MCP connectors — no server list to expose.
    getMcpServers: () => [],
    isCloudProviderKind: (kind) => isCloudAiProviderKind(kind),
    labelForProvider: (id) => aiProviderLabelById(id),
    // Onboarding has no Ask AI readiness pill (settings aren't persisted yet, so
    // its status surface is deliberately omitted — see file header), so nothing
    // to refresh after a key save/clear here.
    loadAskAiAvailability: () => {},
  });

  // ── Provider list mutations (mirror the Settings controller) ──────────────
  function addProvider(kind: AiProviderKind, baseUrl = ""): string | null {
    // Guarded by aiProviderRemoving at the call site (control is disabled while a
    // clear is in flight); this guard is the defensive backstop.
    if (aiProviderRemoving) return null;
    const existingIds = draftAiProviders.map((p) => p.id);
    const id = newAiProviderId(kind, existingIds);
    draftAiProviders = [...draftAiProviders, { id, kind, label: "", baseUrl }];
    void aiRuntime.refreshAiProviderKeyPresence();
    return id;
  }

  // One monotonic ticket per instance id. `verify_ai_provider` is a network
  // round trip and it is RE-RUNNABLE (a card's Re-check, and every key/base-URL
  // edit), so two probes for the same id can be in flight at once — a rejected
  // key that times out settles long after a good one that answered in 300 ms.
  // Only the newest ticket may write, otherwise the stale probe's verdict wins
  // and `aiConfigReady` describes a config the user has already replaced.
  // Plain Map, not `$state`: nothing renders off it.
  const verifyTicket = new Map<string, number>();
  function nextVerifyTicket(id: string): number {
    const ticket = (verifyTicket.get(id) ?? 0) + 1;
    verifyTicket.set(id, ticket);
    return ticket;
  }

  /** Drop a provider's verification — its config changed, so the old proof is
   *  no longer evidence of anything. Bumping the ticket also stops a probe
   *  still in flight from landing its (now meaningless) verdict afterwards. */
  function invalidateVerification(id: string): void {
    nextVerifyTicket(id);
    const { [id]: _dropped, ...rest } = aiVerifications;
    aiVerifications = rest;
  }

  /**
   * THE proof. Lists the provider's models with the key the vault already holds
   * (the key is never sent from here and never comes back), against the DRAFT
   * provider list so a provider still being typed can verify mid-onboarding.
   */
  async function verifyProvider(id: string): Promise<void> {
    const ticket = nextVerifyTicket(id);
    aiVerifications = { ...aiVerifications, [id]: { status: "checking" } };
    // Superseded by a newer probe (or by a re-seed / removal) while the round
    // trip was in flight, or the provider is gone — either way this verdict is
    // about a config that no longer exists.
    const stale = (): boolean => verifyTicket.get(id) !== ticket || !providerById(id);
    try {
      const result = await invoke<{ models: string[]; latencyMs: number }>("verify_ai_provider", {
        request: { provider: id, providers: $state.snapshot(draftAiProviders) },
      });
      if (stale()) return;
      aiVerifications = {
        ...aiVerifications,
        [id]: { status: "live", models: result.models, latencyMs: result.latencyMs },
      };
    } catch (error) {
      if (stale()) return;
      // The chatgpt kind verifies against its vault token set; the backend's
      // `needs_reconnect:<id>` reason code reads as line noise in the card
      // pill, so translate it at this edge (Settings maps it in its own store).
      const raw = humanizeError(error);
      // Case-insensitive: `humanizeError` upper-cases the first letter of what
      // it tidies, so a `startsWith("needs_reconnect:")` test never matches.
      const reason = /^needs_reconnect:/i.test(raw)
        ? "sign in with ChatGPT to finish connecting"
        : raw;
      aiVerifications = {
        ...aiVerifications,
        [id]: { status: "error", reason },
      };
    }
  }

  /**
   * One-shot reachability probe for an endpoint that is NOT (yet) a connected
   * provider — the "scan this Mac" ladder. Same command, an ephemeral instance
   * id that never enters the draft list, so nothing is added until the user
   * says so. `null` means nothing answered there.
   */
  async function probeEndpoint(
    kind: AiProviderKind,
    baseUrl: string,
  ): Promise<{ models: string[]; latencyMs: number } | null> {
    const id = `scan-${baseUrl}`;
    try {
      return await invoke<{ models: string[]; latencyMs: number }>("verify_ai_provider", {
        request: { provider: id, providers: [{ id, kind, label: "", baseUrl }] },
      });
    } catch {
      return null;
    }
  }

  /**
   * Reaching the provider IS the save: store the typed key in the app vault,
   * then verify. The caller hands the string over once and forgets it — nothing
   * here keeps it (`saveAiProviderKey` clears its own input on success).
   */
  async function saveKeyAndVerify(id: string, key: string): Promise<void> {
    aiRuntime.setProviderKeyInput(id, key);
    await aiRuntime.saveAiProviderKey(id);
    aiRuntime.setProviderKeyInput(id, "");
    await verifyProvider(id);
  }

  async function removeProvider(id: string): Promise<void> {
    const removed = providerById(id);
    draftAiProviders = draftAiProviders.filter((p) => p.id !== id);
    invalidateVerification(id);
    if (draftAiDefaultModel?.provider === id) {
      draftAiDefaultModel = null;
    }
    if (removed && isCloudAiProviderKind(removed.kind)) {
      // AWAIT the keychain clear (ADR 0035: a same-kind re-add reuses the bare
      // kind id, so it must re-probe only after the clear resolves) and disable
      // the add-provider control for the duration so a new provider can't be
      // added mid-clear.
      aiProviderRemoving = true;
      try {
        await aiRuntime.clearKeyForRemovedProvider(id);
      } finally {
        aiProviderRemoving = false;
      }
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
    // A re-seed replaces every provider config, so last session's proofs are
    // void — nothing is ready again until it answers again. Bump every ticket
    // too, or a probe still in flight re-adds the verdict this just cleared.
    for (const known of [...verifyTicket.keys()]) nextVerifyTicket(known);
    aiVerifications = {};
    aiRestoredModelNote = null;
    const restored = defaultModel
      ? { provider: defaultModel.provider, model: defaultModel.model }
      : null;
    draftAiDefaultModel = restored;
    // A persisted default the provider has since dropped (renamed, deprecated,
    // un-pulled locally) would otherwise survive a reload looking valid. Verify
    // it against the FRESH listing and clear it with a reason the user can see.
    if (!restored) return;
    void verifyProvider(restored.provider).then(() => {
      const verification = aiVerifications[restored.provider];
      if (verification?.status !== "live") return;
      if (verification.models.includes(restored.model)) return;
      if (draftAiDefaultModel?.model !== restored.model) return;
      aiRestoredModelNote = `${aiProviderLabelById(restored.provider)} no longer lists ${restored.model} — choose another default model.`;
      draftAiDefaultModel = null;
    });
  }

  // Refresh which connected cloud providers already have a saved key (e.g. when
  // the user re-opens onboarding after a partial setup). No-op on a clean run.
  function init(): void {
    void aiRuntime.refreshAiProviderKeyPresence();
  }

  // ── Derived view state ────────────────────────────────────────────────────
  const anyCloudConnected = $derived(draftAiProviders.some((p) => isCloudAiProviderKind(p.kind)));

  // Single source of truth for "Ask AI is usable". Ask AI can only run if a
  // chosen default model can actually answer: the model's provider VERIFIED LIVE
  // this session and that listing still contains this exact model id. A stored
  // key is not evidence and no kind is exempt — see `$lib/onboarding/ai-readiness`.
  // `aiConfigMissing` is the PRIMARY computation (returns the short human reason,
  // or null when ready) and `aiConfigReady` is derived from it, so the boolean
  // and the explanation can never drift. Both the attention rule
  // (OnboardingController.featureAttention) and AskAiBody read these — the
  // condition lives ONLY here.
  const aiConfigMissing = $derived(
    aiReadinessMissing({
      providers: draftAiProviders.map((p) => ({
        id: p.id,
        label: aiProviderInstanceLabel(p),
      })),
      verifications: aiVerifications,
      defaultModel: draftAiDefaultModel,
    }),
  );
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
  // NOTE: there is deliberately no `aiUnverifiedNote`. It used to be a soft hint
  // ("key saved but unverified") alongside an already-green `aiConfigReady`.
  // Unverified is now a hard failure of `aiConfigMissing` itself, so ready
  // implies verified and there is nothing left to caveat.

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
    get aiProviderRemoving() { return aiProviderRemoving; },
    get aiVerifications() { return aiVerifications; },
    // Flat alias of the keychain-presence map (the AiSetup component's one
    // reason to know a key was stored: an error card still says so).
    get aiProviderKeySaved() { return aiRuntime.aiProviderKeySavedByProvider; },
    get aiRestoredModelNote() { return aiRestoredModelNote; },

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
    verifyProvider,
    invalidateVerification,
    probeEndpoint,
    saveKeyAndVerify,
    loadModels,
    syncFromSettings,
    init,
    aiProviderLabelById,
  };
}

export type OnboardingAiStore = ReturnType<typeof createOnboardingAiStore>;
