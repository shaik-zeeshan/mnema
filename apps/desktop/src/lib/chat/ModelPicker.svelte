<script lang="ts">
  // ModelPicker — the per-thread model pin in the Chat composer.
  //
  // A controlled wrapper around the shared <ModelPickerMenu> UI: the PARENT owns
  // the per-thread pin (provider+model) and the aiRuntime settings snapshot (it
  // refreshes that on settings events); this component owns the discovered model
  // pool (listed lazily, one provider at a time) and the default-label
  // resolution, and reports a chosen engine back via `onselect`.
  //
  // `null` (the menu's "Default" row) clears the pin back to the global default
  // model; the trigger label shows what "Default" resolves to. See ADR 0033/0034.
  import { untrack } from "svelte";
  import ModelPickerMenu from "./ModelPickerMenu.svelte";
  import { openSettings } from "$lib/surface-windows";
  import { providerWhere } from "./chat-format";
  import { ModelPoolLoader } from "$lib/insights/modelPool.svelte";
  import {
    defaultEnginePinProvider,
    defaultEngineModel,
    providerLabelById,
    shortModelLabel,
  } from "$lib/insights/conversation";
  import type { AiRuntimeSettings } from "$lib/types/recording";

  let {
    aiRuntime,
    askAiModelOverride,
    pinProvider,
    pinModel,
    open = $bindable(false),
    up = false,
    onselect,
  }: {
    /** The aiRuntime settings snapshot backing default/provider labels. */
    aiRuntime: AiRuntimeSettings | null;
    /** Ask AI model override (access.askAiModel): what "Default" resolves to. */
    askAiModelOverride: string | null;
    /** The active thread's pinned provider (null → unpinned / default). */
    pinProvider: string | null;
    /** The active thread's pinned model id (null → unpinned / default). */
    pinModel: string | null;
    /** Open state — bindable so the parent can close on thread switch. */
    open?: boolean;
    /** Open the menu upward (the composer is docked at the bottom). */
    up?: boolean;
    /** Commit a chosen engine, or `null` to clear the pin to default. */
    onselect: (engine: { provider: string; model: string } | null) => void;
  } = $props();

  // The default-model provider/id, resolved from settings. Precedence matches
  // the backend resolver: Ask AI override (access.askAiModel) wins over the
  // global default model for the unpinned label.
  let defaultProvider = $derived(
    aiRuntime === null ? null : defaultEnginePinProvider(aiRuntime),
  );
  let resolvedDefaultModel = $derived(
    askAiModelOverride !== null
      ? askAiModelOverride
      : aiRuntime === null
        ? null
        : defaultEngineModel(aiRuntime),
  );

  // The trigger shows the SHORT id of the model answering this chat (pin, else
  // what Default resolves to) with its provider letter and cloud/local chip; the
  // full id rides on the tooltip.
  let activeProvider = $derived(pinProvider ?? defaultProvider);
  let activeModel = $derived(pinModel ?? resolvedDefaultModel);
  let activeConfig = $derived(aiRuntime?.providers.find((p) => p.id === activeProvider));
  let activeModelLabel = $derived(activeModel === null ? "Default" : shortModelLabel(activeModel));
  let activeModelTitle = $derived.by(() => {
    const model = activeModel ?? "Default";
    const head = pinProvider === null ? "Default · " : "";
    return activeProvider === null ? `${head}${model}` : `${head}${providerLabelById(aiRuntime?.providers, activeProvider)} · ${model}`;
  });

  // The merged provider-tagged model pool, discovered ONE PROVIDER AT A TIME so
  // a fast provider's models show immediately instead of waiting on the slowest
  // (see ModelPoolLoader). Loaded lazily on first menu open; invalidated when
  // the connected-provider set changes (see the effect below).
  const loader = new ModelPoolLoader();

  // Failure labels resolved against the current provider set.
  let failureRows = $derived(
    loader.failures.map((f) => ({
      provider: f.provider,
      label: providerLabelById(aiRuntime?.providers, f.provider),
      reason: f.reason,
    })),
  );
  // The provider configs that failed last fetch — the targets a Retry re-lists
  // (so the already-loaded providers don't flicker).
  let retryTargets = $derived(
    (aiRuntime?.providers ?? []).filter((p) =>
      loader.failures.some((f) => f.provider === p.id),
    ),
  );

  // When the connected-provider set changes, the cached pool is stale: drop it
  // so the next open re-discovers against the current providers (refresh in
  // place if the menu is open right now).
  let providerSignature = $derived(
    (aiRuntime?.providers ?? []).map((p) => `${p.id}:${p.baseUrl ?? ""}`).join("|"),
  );
  $effect(() => {
    providerSignature; // track
    untrack(() => {
      loader.reset();
      if (open) void loader.load(aiRuntime?.providers ?? []);
    });
  });

  // First open lists everyone; a reopen only re-lists the providers that failed
  // last time (a LAN endpoint that was asleep shows up once it's back, no
  // restart) — the working providers stay put.
  function handleOpen(): void {
    if (!loader.loaded) void loader.load(aiRuntime?.providers ?? []);
    else if (retryTargets.length > 0) void loader.load(retryTargets);
  }
</script>

<!-- Per-thread model pin: choose which model answers this thread — a pooled
     model, a typed id attributed to a provider, or "Default" to clear the pin.
     The label shows what "Default" resolves to when unpinned. -->
{#snippet foot()}
  applies to this chat only
  <button type="button" class="ch-pop__link" onmousedown={(e) => e.preventDefault()} onclick={() => void openSettings("intelligence")}>Settings → AI ↗</button>
{/snippet}

<ModelPickerMenu
  label={activeModelLabel}
  title={activeModelTitle}
  ariaLabel="Choose model"
  glyph={activeProvider === null ? null : providerLabelById(aiRuntime?.providers, activeProvider).charAt(0).toUpperCase()}
  where={activeProvider === null ? null : providerWhere(activeConfig)}
  {up}
  {foot}
  modelPool={loader.pool}
  providers={aiRuntime?.providers ?? []}
  firstProvider={defaultProvider}
  sentinelLabel="Default"
  sentinelDetail={resolvedDefaultModel}
  sentinelTitle={resolvedDefaultModel === null ? "Default" : `Default · ${resolvedDefaultModel}`}
  sentinelSelected={pinProvider === null}
  selectedProvider={pinProvider}
  selectedModel={pinModel}
  loading={loader.loading}
  failures={failureRows}
  onretry={() => void loader.load(retryTargets)}
  bind:open
  onopen={handleOpen}
  {onselect}
/>

<style>
  .ch-pop__link { margin-left: auto; padding: 0; border: 0; background: none; cursor: pointer; font: inherit; color: var(--app-text-muted); }
  .ch-pop__link:hover { color: var(--app-text-strong); }
</style>
