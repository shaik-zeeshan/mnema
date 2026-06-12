<script lang="ts">
  // ModelPicker — the per-thread model picker pill in the Chat composer.
  //
  // A controlled component: the PARENT owns the per-thread pin (provider+model)
  // and the aiRuntime settings snapshot (it refreshes that on settings events);
  // this component owns its own UI (open/search/highlight) and the discovered
  // model pool, and reports a chosen engine back via `onselect`.
  //
  // The picker is a combobox: a search box filters a list grouped by provider
  // so multi-provider users can see (and disambiguate) which provider a model
  // belongs to. A typed id that no provider advertises is offered explicitly
  // PER provider ("Use … as a model id"), so a pasted id never silently
  // resolves to the wrong provider. `null` (the "Default" row) clears the pin
  // back to the global default model. See ADR 0033/0034.
  import { tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    defaultEnginePinProvider,
    defaultEngineModel,
    providerLabelById,
    pinnableEnginesFromModelPool,
    shortModelLabel,
  } from "$lib/insights/conversation";
  import type {
    AiProviderConfig,
    AiRuntimeModel,
    AiRuntimeModelsResult,
    AiRuntimeProviderFailure,
    AiRuntimeSettings,
  } from "$lib/types/recording";

  let {
    aiRuntime,
    askAiModelOverride,
    pinProvider,
    pinModel,
    open = $bindable(false),
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

  // The trigger shows a SHORT, legible model label (the `/`-tail) so a long
  // routing path isn't cut off mid-id by the pill's ellipsis; the full,
  // unshortened label rides on the trigger's `title=` (activeModelTitle).
  let activeModelLabel = $derived.by(() => {
    if (pinProvider === null || pinModel === null) {
      return resolvedDefaultModel === null
        ? "Default"
        : `Default · ${shortModelLabel(resolvedDefaultModel)}`;
    }
    if (defaultProvider !== null && pinProvider === defaultProvider) {
      return shortModelLabel(pinModel);
    }
    // A non-default-provider pin keeps its provider context.
    return `${providerLabelById(aiRuntime?.providers, pinProvider)} · ${shortModelLabel(pinModel)}`;
  });
  let activeModelTitle = $derived.by(() => {
    if (pinProvider === null || pinModel === null) {
      return resolvedDefaultModel === null
        ? "Default"
        : `Default · ${resolvedDefaultModel}`;
    }
    if (defaultProvider !== null && pinProvider === defaultProvider) {
      return pinModel;
    }
    return `${providerLabelById(aiRuntime?.providers, pinProvider)} · ${pinModel}`;
  });

  // The merged provider-tagged model pool. Discovered ONE PROVIDER AT A TIME
  // (a fan-out of single-provider `ai_runtime_list_models` calls) and merged in
  // as each resolves, so a fast provider's models show immediately instead of
  // waiting on the slowest (a 10s-timeout/unreachable provider no longer blocks
  // the rest). Loaded lazily on first picker open; invalidated when the
  // connected-provider set changes (see the effect below).
  let modelPool = $state<AiRuntimeModel[]>([]);
  let modelsLoaded = $state(false);
  // True while ANY provider's listing is still in flight.
  let modelsLoading = $state(false);
  // Providers that failed to list last fetch (unreachable, missing key). Shown
  // as a "⚠ <provider> — <reason>  Retry" row so a smaller pool is explained
  // and a transiently-down endpoint can be re-listed without a restart.
  let modelFailures = $state<AiRuntimeProviderFailure[]>([]);
  // Failure labels resolved against the current provider set.
  let failureRows = $derived(
    modelFailures.map((f) => ({
      provider: f.provider,
      label: providerLabelById(aiRuntime?.providers, f.provider),
      reason: f.reason,
    })),
  );
  // The provider configs that failed last fetch — the targets a Retry re-lists
  // (so the already-loaded providers don't flicker).
  let retryTargets = $derived(
    (aiRuntime?.providers ?? []).filter((p) =>
      modelFailures.some((f) => f.provider === p.id),
    ),
  );
  let pinnableEngines = $derived(
    pinnableEnginesFromModelPool(modelPool, aiRuntime?.providers),
  );

  // Search/keyboard state.
  let modelQuery = $state("");
  let modelHighlight = $state(0);
  let modelSearchEl = $state<HTMLInputElement | null>(null);

  // Connected providers, in config order, with their display labels.
  let connectedProviders = $derived(
    (aiRuntime?.providers ?? []).map((p) => ({
      id: p.id,
      label: providerLabelById(aiRuntime?.providers, p.id),
    })),
  );

  // The discovered pool grouped by provider, default provider's group first so
  // the common case sits at the top.
  let groupedPool = $derived.by(() => {
    const groups = new Map<
      string,
      { provider: string; label: string; models: string[] }
    >();
    const ensure = (id: string) => {
      let g = groups.get(id);
      if (!g) {
        g = {
          provider: id,
          label: providerLabelById(aiRuntime?.providers, id),
          models: [],
        };
        groups.set(id, g);
      }
      return g;
    };
    // Seed groups in provider order (default first) for a stable layout.
    if (defaultProvider !== null) ensure(defaultProvider);
    for (const p of connectedProviders) ensure(p.id);
    for (const e of pinnableEngines) ensure(e.provider).models.push(e.model);
    return [...groups.values()].filter((g) => g.models.length > 0);
  });

  // A picker option is either the unpinned default, a discovered pool model, or
  // a typed exact id attributed to one specific provider.
  type PickerOption =
    | { kind: "default" }
    | { kind: "pool"; provider: string; model: string }
    | { kind: "exact"; provider: string; model: string };

  // Display rows: non-selectable group headers interleaved with selectable
  // options. Each option carries a flat `index` for keyboard navigation.
  let pickerRows = $derived.by(() => {
    type Row =
      | { type: "header"; key: string; label: string }
      | {
          type: "option";
          key: string;
          option: PickerOption;
          main: string;
          sub: string | null;
          title: string;
          selected: boolean;
          index: number;
        };
    const rows: Row[] = [];
    let index = 0;
    const query = modelQuery.trim().toLowerCase();
    const raw = modelQuery.trim();
    const addOption = (
      option: PickerOption,
      main: string,
      sub: string | null,
      title: string,
      selected: boolean,
    ) => {
      rows.push({ type: "option", key: `opt-${index}`, option, main, sub, title, selected, index });
      index++;
    };

    // Default — always selectable; hidden only when a query excludes it.
    if (
      query.length === 0 ||
      "default".includes(query) ||
      (resolvedDefaultModel ?? "").toLowerCase().includes(query)
    ) {
      const main =
        resolvedDefaultModel === null
          ? "Default"
          : `Default · ${shortModelLabel(resolvedDefaultModel)}`;
      addOption({ kind: "default" }, main, null, activeModelTitle, pinProvider === null);
    }

    // Pool, grouped by provider.
    for (const group of groupedPool) {
      const matches = group.models.filter(
        (m) =>
          query.length === 0 ||
          m.toLowerCase().includes(query) ||
          group.label.toLowerCase().includes(query),
      );
      if (matches.length === 0) continue;
      rows.push({ type: "header", key: `hdr-${group.provider}`, label: group.label });
      for (const model of matches) {
        const selected = group.provider === pinProvider && model === pinModel;
        addOption(
          { kind: "pool", provider: group.provider, model },
          shortModelLabel(model),
          null,
          model,
          selected,
        );
      }
    }

    // Exact-id fallback: a typed id that isn't an exact pool match is offered
    // per provider, so it resolves to a chosen provider rather than a guess.
    if (raw.length > 0 && !pinnableEngines.some((e) => e.model === raw)) {
      if (connectedProviders.length > 0) {
        rows.push({ type: "header", key: "hdr-exact", label: "Use as a model id" });
        for (const p of connectedProviders) {
          addOption({ kind: "exact", provider: p.id, model: raw }, raw, p.label, `${p.label} · ${raw}`, false);
        }
      }
    }

    return rows;
  });

  // The selectable subset, for keyboard navigation bounds.
  let pickerOptions = $derived(pickerRows.filter((row) => row.type === "option"));

  // When the connected-provider set changes, the cached pool is stale: drop it
  // so the next open re-discovers against the current providers (refresh in
  // place if the picker is open right now).
  let providerSignature = $derived(
    (aiRuntime?.providers ?? []).map((p) => `${p.id}:${p.baseUrl ?? ""}`).join("|"),
  );
  $effect(() => {
    providerSignature; // track
    untrack(() => {
      modelsLoaded = false;
      modelPool = [];
      modelFailures = [];
      if (open) void loadModelList();
    });
  });

  // List the given providers (default: all connected), ONE CALL PER PROVIDER in
  // parallel, merging each provider's result into the pool the moment it lands.
  // Each provider's slice is replaced wholesale, so a Retry of just the failed
  // providers leaves the already-loaded ones untouched.
  async function loadModelList(
    targets: readonly AiProviderConfig[] = aiRuntime?.providers ?? [],
  ): Promise<void> {
    if (modelsLoading) return;
    if (targets.length === 0) {
      modelsLoaded = true;
      return;
    }
    modelsLoading = true;
    await Promise.all(
      targets.map(async (provider) => {
        let models: AiRuntimeModel[] = [];
        let failure: AiRuntimeProviderFailure | null = null;
        try {
          // A single-element provider list lists JUST this provider, so its
          // result arrives independently of the others.
          const result = await invoke<AiRuntimeModelsResult>("ai_runtime_list_models", {
            request: { providers: [provider] },
          });
          models = result.models;
          failure = result.failures[0] ?? null;
        } catch {
          failure = { provider: provider.id, reason: "couldn't list models" };
        }
        // Replace this provider's slice (incremental: triggers a re-render as
        // soon as THIS provider resolves, without waiting on its peers).
        modelPool = [
          ...modelPool.filter((m) => m.provider !== provider.id),
          ...models,
        ];
        modelFailures = [
          ...modelFailures.filter((f) => f.provider !== provider.id),
          ...(failure ? [failure] : []),
        ];
      }),
    );
    modelsLoading = false;
    modelsLoaded = true;
  }

  function toggleModelPicker(): void {
    open = !open;
    if (open) {
      modelQuery = "";
      modelHighlight = 0;
      // First open lists everyone; a reopen only re-lists the providers that
      // failed last time (a LAN endpoint that was asleep shows up once it's
      // back, no restart) — the working providers stay put.
      if (!modelsLoaded) void loadModelList();
      else if (retryTargets.length > 0) void loadModelList(retryTargets);
      void tick().then(() => modelSearchEl?.focus());
    }
  }

  function selectOption(option: PickerOption): void {
    open = false;
    modelQuery = "";
    onselect(option.kind === "default" ? null : { provider: option.provider, model: option.model });
  }

  function onModelSearchKeydown(event: KeyboardEvent): void {
    const options = pickerOptions;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      modelHighlight = Math.min(modelHighlight + 1, options.length - 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      modelHighlight = Math.max(modelHighlight - 1, 0);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const choice = options[modelHighlight];
      if (choice && choice.type === "option") selectOption(choice.option);
    } else if (event.key === "Escape") {
      event.stopPropagation();
      open = false;
      modelQuery = "";
    }
  }
</script>

<!-- Per-thread model pin: choose which model answers this thread — a pooled
     model, a typed id attributed to a provider, or "Default" to clear the pin.
     The label shows what "Default" resolves to when unpinned. -->
<div class="engine-pick-menu">
  <button
    type="button"
    class="engine-pick-trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label="Model for this thread"
    title={activeModelTitle}
    onclick={toggleModelPicker}
  >
    <span class="engine-pick-current">{activeModelLabel}</span>
    <span class="engine-pick-caret" aria-hidden="true">▾</span>
  </button>
  {#if open}
    <div class="engine-pick-pop">
      <input
        bind:this={modelSearchEl}
        bind:value={modelQuery}
        class="engine-pick-search"
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls="engine-pick-list"
        aria-label="Search models"
        placeholder="Search or paste a model id…"
        spellcheck="false"
        autocomplete="off"
        oninput={() => (modelHighlight = 0)}
        onkeydown={onModelSearchKeydown}
      />
      <ul id="engine-pick-list" class="engine-pick-list" role="listbox" aria-label="Model">
        {#each pickerRows as row (row.key)}
          {#if row.type === "header"}
            <li role="presentation" class="engine-pick-group">{row.label}</li>
          {:else}
            <li role="presentation">
              <button
                type="button"
                class="engine-pick-option"
                class:engine-pick-option--active={row.selected}
                class:engine-pick-option--cursor={row.index === modelHighlight}
                role="option"
                aria-selected={row.selected}
                title={row.title}
                onmouseenter={() => (modelHighlight = row.index)}
                onclick={() => selectOption(row.option)}
              >
                <span class="engine-pick-option-main">{row.main}</span>
                {#if row.sub}
                  <span class="engine-pick-option-sub">{row.sub}</span>
                {/if}
                {#if row.selected}
                  <span class="engine-pick-check" aria-hidden="true">✓</span>
                {/if}
              </button>
            </li>
          {/if}
        {/each}
        {#if modelsLoading}
          <li role="presentation">
            <span class="engine-pick-note">Loading models…</span>
          </li>
        {:else if pickerRows.length === 0}
          <li role="presentation">
            <span class="engine-pick-note">No matches</span>
          </li>
        {/if}
      </ul>
      {#if failureRows.length > 0}
        <!-- Providers that couldn't list — surfaced (not silently dropped) so a
             smaller pool is explained; Retry re-lists every provider. -->
        <div class="engine-pick-failures">
          {#each failureRows as failure (failure.provider)}
            <div class="engine-pick-failure">
              <span class="engine-pick-failure-text" title={`${failure.label}: ${failure.reason}`}>
                <span class="engine-pick-failure-warn" aria-hidden="true">⚠</span>
                {failure.label} — {failure.reason}
              </span>
            </div>
          {/each}
          <button
            type="button"
            class="engine-pick-retry"
            disabled={modelsLoading}
            onclick={() => void loadModelList(retryTargets)}
          >
            {modelsLoading ? "Retrying…" : "Retry"}
          </button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .engine-pick-menu {
    position: relative;
  }
  .engine-pick-trigger {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font: inherit;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 4px 10px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .engine-pick-trigger:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .engine-pick-caret {
    font-size: 8px;
  }
  /* Long custom model ids stay on one line inside the pill. */
  .engine-pick-current {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The open popover: search box pinned on top of a scrolling, grouped list. */
  .engine-pick-pop {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 0;
    display: flex;
    flex-direction: column;
    width: 280px;
    padding: 4px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-raised);
    box-shadow: 0 8px 24px var(--app-shadow, rgba(0, 0, 0, 0.25));
    z-index: 20;
  }
  .engine-pick-search {
    width: 100%;
    font: inherit;
    font-size: 11px;
    padding: 6px 9px;
    margin-bottom: 4px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
    color: var(--app-text);
  }
  .engine-pick-search:focus {
    outline: none;
    border-color: var(--app-border-hover);
  }
  .engine-pick-list {
    min-width: 0;
    max-height: 260px;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  /* Provider group header — small, muted, uppercase. */
  .engine-pick-group {
    padding: 7px 9px 3px;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .engine-pick-option {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    font: inherit;
    font-size: 11px;
    text-align: left;
    padding: 6px 9px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    transition: background 0.12s ease;
  }
  /* The model id takes the row; provider sub-label and check sit at the end. */
  .engine-pick-option-main {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .engine-pick-option-sub {
    flex: 0 0 auto;
    font-size: 10px;
    color: var(--app-text-muted);
  }
  /* Keyboard-cursor row (distinct from the committed-selection accent). */
  .engine-pick-option--cursor {
    background: var(--app-surface-hover);
  }
  .engine-pick-option:hover {
    background: var(--app-surface-hover);
  }
  .engine-pick-option--active {
    color: var(--app-accent-strong);
  }
  .engine-pick-check {
    flex: 0 0 auto;
    color: var(--app-accent-strong);
    font-size: 10px;
  }
  /* Muted one-liner inside the dropdown (loading / discovery failure). */
  .engine-pick-note {
    display: block;
    font-size: 11px;
    padding: 6px 9px;
    color: var(--app-text-faint);
  }
  /* Failed-provider footer: each provider + reason, then a Retry button. */
  .engine-pick-failures {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--app-border);
  }
  .engine-pick-failure {
    padding: 4px 9px;
  }
  .engine-pick-failure-text {
    display: block;
    font-size: 10.5px;
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .engine-pick-failure-warn {
    color: var(--app-warn, #c9920a);
  }
  .engine-pick-retry {
    width: 100%;
    margin-top: 2px;
    font: inherit;
    font-size: 11px;
    padding: 5px 9px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
    color: var(--app-text);
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .engine-pick-retry:hover:not(:disabled) {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .engine-pick-retry:disabled {
    cursor: default;
    color: var(--app-text-faint);
  }
</style>
