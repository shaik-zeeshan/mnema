<script lang="ts">
  // ModelPickerMenu — the shared model-picker UI: a trigger that opens a search
  // box over a provider-grouped listbox. ONE picker surface for every place a
  // model is chosen (the Chat composer's per-thread pin and the Settings
  // default-model / Ask AI-override fields), so the three never drift apart.
  //
  // Purely presentational + interactive: the PARENT owns the model pool, the
  // committed selection, and what a choice means (it maps the chosen engine back
  // onto its own draft). This component owns only the open/search/highlight UI,
  // row building (group headers + selectable options + a typed-id fallback), and
  // keyboard navigation. A typed id no provider advertises is offered explicitly
  // (PER provider when `exactIdPerProvider`) so a pasted id never silently
  // resolves to the wrong provider; `null` (the sentinel row) reports "clear".
  //
  // Positioning is always CSS-relative to the trigger (never portaled): the
  // `block` Settings menu pins directly below the trigger, the inline composer
  // pill opens upward. Body-portaling was dropped — across Settings' inner
  // scroll container the trigger rect is measured in a different coordinate
  // space in the Tauri WKWebView, which floated the menu off-screen; the cards
  // are built without `overflow:hidden` so an inline menu shows fine. This is
  // the same approach as Select/Combobox. See ADR 0033/0034.
  import { tick } from "svelte";
  import {
    providerLabelById,
    pinnableEnginesFromModelPool,
    shortModelLabel,
  } from "$lib/insights/conversation";
  import type {
    AiProviderConfig,
    AiRuntimeModel,
  } from "$lib/types/recording";

  let {
    label,
    title,
    ariaLabel,
    disabled = false,
    block = false,
    placeholder = false,
    modelPool,
    providers,
    firstProvider = null,
    sentinelLabel = null,
    sentinelTitle = null,
    sentinelSelected = false,
    selectedProvider = null,
    selectedModel = null,
    allowExactId = true,
    exactIdPerProvider = true,
    loading = false,
    failures = [],
    onretry,
    open = $bindable(false),
    onopen,
    onselect,
  }: {
    /** Short label shown in the trigger (what's currently selected). */
    label: string;
    /** Full, unshortened label for the trigger's `title=`. */
    title: string;
    /** Accessible name for the trigger. */
    ariaLabel: string;
    /** Disable the trigger (e.g. no providers connected / feature off). */
    disabled?: boolean;
    /** Full-width form-control trigger (Settings) vs an inline pill (Chat). */
    block?: boolean;
    /** Render the trigger label muted (nothing selected yet). */
    placeholder?: boolean;
    /** The merged provider-tagged model pool to show. */
    modelPool: AiRuntimeModel[];
    /** Connected providers — drives group labels/order and the typed-id rows. */
    providers: AiProviderConfig[] | null | undefined;
    /** Provider whose group sorts first (the default's), or null. */
    firstProvider?: string | null;
    /** The sentinel/"clear" row label (e.g. "Default"), or null to hide it. */
    sentinelLabel?: string | null;
    /** Full title for the sentinel row; falls back to `sentinelLabel`. */
    sentinelTitle?: string | null;
    /** Whether the sentinel row is the committed selection. */
    sentinelSelected?: boolean;
    /** Committed provider id (null → match a row by model id alone). */
    selectedProvider?: string | null;
    /** Committed model id (null → only the sentinel can be selected). */
    selectedModel?: string | null;
    /** Offer a typed id that no provider advertises. */
    allowExactId?: boolean;
    /** Offer the typed id per connected provider vs as a single row. */
    exactIdPerProvider?: boolean;
    /** True while the pool is still being listed. */
    loading?: boolean;
    /** Providers that failed to list, surfaced with a Retry. */
    failures?: { provider: string; label: string; reason: string }[];
    /** Re-list the failed providers. */
    onretry?: () => void;
    /** Open state — bindable so the parent can close it externally. */
    open?: boolean;
    /** Fired when the menu opens, so the parent can (re)load the pool. */
    onopen?: () => void;
    /** Commit a chosen engine, or `null` for the sentinel/"clear" row. */
    onselect: (engine: { provider: string; model: string } | null) => void;
  } = $props();

  // Search/keyboard state.
  let query = $state("");
  let highlight = $state(0);
  let searchEl = $state<HTMLInputElement | null>(null);
  let triggerEl = $state<HTMLButtonElement | null>(null);
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

  // Connected providers, in config order, with their display labels.
  let connectedProviders = $derived(
    (providers ?? []).map((p) => ({
      id: p.id,
      label: providerLabelById(providers, p.id),
    })),
  );

  let pinnableEngines = $derived(
    pinnableEnginesFromModelPool(modelPool, providers),
  );

  // The discovered pool grouped by provider, the first-provider's group first so
  // the common case sits at the top.
  let groupedPool = $derived.by(() => {
    const groups = new Map<
      string,
      { provider: string; label: string; models: string[] }
    >();
    const ensure = (id: string) => {
      let g = groups.get(id);
      if (!g) {
        g = { provider: id, label: providerLabelById(providers, id), models: [] };
        groups.set(id, g);
      }
      return g;
    };
    if (firstProvider !== null) ensure(firstProvider);
    for (const p of connectedProviders) ensure(p.id);
    for (const e of pinnableEngines) ensure(e.provider).models.push(e.model);
    return [...groups.values()].filter((g) => g.models.length > 0);
  });

  // A picker option is the sentinel, a discovered pool model, or a typed exact
  // id attributed to one specific provider.
  type PickerOption =
    | { kind: "sentinel" }
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
    const q = query.trim().toLowerCase();
    const raw = query.trim();
    const addOption = (
      option: PickerOption,
      main: string,
      sub: string | null,
      rowTitle: string,
      selected: boolean,
    ) => {
      rows.push({ type: "option", key: `opt-${index}`, option, main, sub, title: rowTitle, selected, index });
      index++;
    };

    // Sentinel ("Default"/"clear") — hidden only when a query excludes it.
    if (
      sentinelLabel !== null &&
      (q.length === 0 || sentinelLabel.toLowerCase().includes(q))
    ) {
      addOption({ kind: "sentinel" }, sentinelLabel, null, sentinelTitle ?? sentinelLabel, sentinelSelected);
    }

    // Pool, grouped by provider.
    for (const group of groupedPool) {
      const matches = group.models.filter(
        (m) =>
          q.length === 0 ||
          m.toLowerCase().includes(q) ||
          group.label.toLowerCase().includes(q),
      );
      if (matches.length === 0) continue;
      rows.push({ type: "header", key: `hdr-${group.provider}`, label: group.label });
      for (const model of matches) {
        const selected =
          model === selectedModel &&
          (selectedProvider === null || group.provider === selectedProvider);
        addOption(
          { kind: "pool", provider: group.provider, model },
          shortModelLabel(model),
          null,
          model,
          selected,
        );
      }
    }

    // Exact-id fallback: a typed id that isn't an exact pool match is offered so
    // it resolves to a chosen provider rather than a guess.
    if (allowExactId && raw.length > 0 && !pinnableEngines.some((e) => e.model === raw)) {
      if (exactIdPerProvider) {
        if (connectedProviders.length > 0) {
          rows.push({ type: "header", key: "hdr-exact", label: "Use as a model id" });
          for (const p of connectedProviders) {
            addOption({ kind: "exact", provider: p.id, model: raw }, raw, p.label, `${p.label} · ${raw}`, false);
          }
        }
      } else {
        rows.push({ type: "header", key: "hdr-exact", label: "Use as a model id" });
        addOption({ kind: "exact", provider: connectedProviders[0]?.id ?? "", model: raw }, raw, null, raw, false);
      }
    }

    return rows;
  });

  // The selectable subset, for keyboard navigation bounds.
  let pickerOptions = $derived(pickerRows.filter((row) => row.type === "option"));

  function openMenu(): void {
    if (closeTimer !== null) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
    open = true;
    query = "";
    highlight = 0;
    onopen?.();
    void tick().then(() => searchEl?.focus());
  }

  function closeMenu(): void {
    open = false;
    query = "";
  }

  function toggleMenu(): void {
    if (disabled) return;
    if (open) closeMenu();
    else openMenu();
  }

  // Close shortly after the search loses focus, so an option's mousedown→click
  // (which is preventDefault'd to keep focus on the search) still lands first.
  function closeSoon(): void {
    closeTimer = setTimeout(() => {
      open = false;
      query = "";
      closeTimer = null;
    }, 120);
  }

  function selectOption(option: PickerOption): void {
    closeMenu();
    onselect(option.kind === "sentinel" ? null : { provider: option.provider, model: option.model });
  }

  function onSearchKeydown(event: KeyboardEvent): void {
    const options = pickerOptions;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      highlight = Math.min(highlight + 1, options.length - 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      highlight = Math.max(highlight - 1, 0);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const choice = options[highlight];
      if (choice && choice.type === "option") selectOption(choice.option);
    } else if (event.key === "Escape") {
      event.stopPropagation();
      closeMenu();
    }
  }
</script>

{#snippet popover()}
  <div class="mpm-pop" class:mpm-pop--block={block} class:mpm-pop--inline={!block}>
    <div class="mpm-search">
      <svg
        class="mpm-search-icon"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <circle cx="11" cy="11" r="7" />
        <path d="M21 21l-4.3-4.3" />
      </svg>
      <input
        bind:this={searchEl}
        bind:value={query}
        class="mpm-search-input"
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls="mpm-list"
        aria-label="Search models"
        placeholder="Search or paste a model id…"
        spellcheck="false"
        autocomplete="off"
        oninput={() => (highlight = 0)}
        onkeydown={onSearchKeydown}
        onblur={closeSoon}
      />
    </div>
    <ul id="mpm-list" class="mpm-list" role="listbox" aria-label="Model">
      {#each pickerRows as row (row.key)}
        {#if row.type === "header"}
          <li role="presentation" class="mpm-group">{row.label}</li>
        {:else}
          <li role="presentation">
            <button
              type="button"
              class="mpm-option"
              class:mpm-option--active={row.selected}
              class:mpm-option--cursor={row.index === highlight}
              role="option"
              aria-selected={row.selected}
              title={row.title}
              onmousedown={(event) => event.preventDefault()}
              onmouseenter={() => (highlight = row.index)}
              onclick={() => selectOption(row.option)}
            >
              <span class="mpm-option-main">{row.main}</span>
              {#if row.sub}
                <span class="mpm-option-sub">{row.sub}</span>
              {/if}
              {#if row.selected}
                <span class="mpm-check" aria-hidden="true">✓</span>
              {/if}
            </button>
          </li>
        {/if}
      {/each}
      {#if loading}
        <li role="presentation">
          <span class="mpm-note">Loading models…</span>
        </li>
      {:else if pickerRows.length === 0}
        <li role="presentation">
          <span class="mpm-note">No matches</span>
        </li>
      {/if}
    </ul>
    {#if failures.length > 0}
      <!-- Providers that couldn't list — surfaced (not silently dropped) so a
           smaller pool is explained; Retry re-lists the failed providers. -->
      <div class="mpm-failures">
        {#each failures as failure (failure.provider)}
          <div class="mpm-failure">
            <span class="mpm-failure-text" title={`${failure.label}: ${failure.reason}`}>
              <span class="mpm-failure-warn" aria-hidden="true">⚠</span>
              {failure.label} — {failure.reason}
            </span>
          </div>
        {/each}
        {#if onretry}
          <button
            type="button"
            class="mpm-retry"
            disabled={loading}
            onmousedown={(event) => event.preventDefault()}
            onclick={() => onretry?.()}
          >
            {loading ? "Retrying…" : "Retry"}
          </button>
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

<div class="mpm-menu" class:mpm-menu--block={block}>
  <button
    type="button"
    bind:this={triggerEl}
    class="mpm-trigger"
    class:mpm-trigger--block={block}
    class:mpm-trigger--placeholder={placeholder}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label={ariaLabel}
    {title}
    {disabled}
    onclick={toggleMenu}
  >
    <span class="mpm-current">{label}</span>
    <svg class="mpm-chevron" viewBox="0 0 24 24" aria-hidden="true">
      <path d="m6 9 6 6 6-6" />
    </svg>
  </button>
  {#if open}
    {@render popover()}
  {/if}
</div>

<style>
  .mpm-menu {
    position: relative;
  }
  .mpm-menu--block {
    width: 100%;
  }
  /* Inline trigger: the small pill used in the Chat composer. */
  .mpm-trigger {
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
  .mpm-trigger:hover:not(:disabled) {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .mpm-trigger:disabled {
    cursor: default;
    opacity: 0.6;
  }
  /* Block trigger: a full-width form control matching the settings `.text-input`
     idiom, with the current selection on the left and the caret on the right. */
  .mpm-trigger--block {
    width: 100%;
    justify-content: space-between;
    gap: 8px;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    letter-spacing: normal;
    padding: 7px 10px;
    border-radius: 8px;
    border-color: var(--app-border-strong);
    background: var(--app-surface);
    color: var(--app-text);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .mpm-trigger--block:hover:not(:disabled) {
    border-color: var(--app-border-hover);
  }
  .mpm-trigger--block:focus-visible {
    border-color: var(--app-accent);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25), 0 0 0 3px var(--app-accent-glow);
    outline: none;
  }
  .mpm-trigger--block[aria-expanded="true"] {
    border-color: var(--app-accent);
  }
  .mpm-trigger--block.mpm-trigger--placeholder .mpm-current {
    color: var(--app-text-faint);
  }
  /* Chevron — shared look with the Select/Combobox family. */
  .mpm-chevron {
    display: block;
    width: 14px;
    height: 14px;
    flex: 0 0 auto;
    fill: none;
    stroke: var(--app-text-muted);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 0.15s, stroke 0.15s;
  }
  /* The compact composer pill uses a smaller chevron. */
  .mpm-trigger:not(.mpm-trigger--block) .mpm-chevron {
    width: 11px;
    height: 11px;
  }
  .mpm-trigger[aria-expanded="true"] .mpm-chevron {
    transform: rotate(180deg);
    stroke: var(--app-accent);
  }
  /* Long custom model ids stay on one line inside the trigger. */
  .mpm-current {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mpm-trigger--block .mpm-current {
    flex: 1 1 auto;
    min-width: 0;
    max-width: none;
    text-align: left;
  }
  /* The open popover: search box pinned on top of a scrolling, grouped list.
     CSS-positioned relative to the trigger — `--block` (Settings) opens down,
     `--inline` (Chat pill) opens up. */
  .mpm-pop {
    display: flex;
    flex-direction: column;
    padding: 4px;
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    background: var(--app-surface-raised);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
    z-index: 100;
  }
  /* Settings: full-width menu pinned directly below the trigger — same as the
     Select/Combobox family (CSS-positioned, never portaled, so it can't drift
     off-screen across the settings scroll container). */
  .mpm-pop--block {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    width: 100%;
  }
  /* Chat composer pill: opens upward (the pill sits at the bottom of the view). */
  .mpm-pop--inline {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 0;
    width: 280px;
    z-index: 9999;
  }
  /* Search row — mirrors the Combobox search (icon + borderless input, pinned
     to the top of the menu). */
  .mpm-search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 9px;
    margin: -4px -4px 4px;
    background: var(--app-surface);
    border-bottom: 1px solid var(--app-border);
    border-radius: 6px 6px 0 0;
  }
  .mpm-search-icon {
    width: 13px;
    height: 13px;
    flex: 0 0 13px;
    color: var(--app-text-muted);
  }
  .mpm-search-input {
    flex: 1 1 auto;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    color: var(--app-text);
    padding: 0;
  }
  .mpm-search-input::placeholder {
    color: var(--app-text-subtle);
  }
  .mpm-list {
    min-width: 0;
    max-height: 260px;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  /* Provider group header — small, muted, uppercase. */
  .mpm-group {
    padding: 7px 9px 3px;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .mpm-option {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    text-align: left;
    padding: 6px 10px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    transition: background 0.1s, color 0.1s;
  }
  /* The model id takes the row; provider sub-label and check sit at the end. */
  .mpm-option-main {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mpm-option-sub {
    flex: 0 0 auto;
    font-size: 10px;
    color: var(--app-text-muted);
  }
  /* Keyboard-cursor row (distinct from the committed-selection accent). */
  .mpm-option:hover,
  .mpm-option--cursor {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  .mpm-option--active {
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .mpm-check {
    flex: 0 0 auto;
    color: var(--app-accent);
    font-size: 10px;
  }
  /* Muted one-liner inside the dropdown (loading / no matches). */
  .mpm-note {
    display: block;
    font-size: 11px;
    padding: 6px 9px;
    color: var(--app-text-faint);
  }
  /* Failed-provider footer: each provider + reason, then a Retry button. */
  .mpm-failures {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--app-border);
  }
  .mpm-failure {
    padding: 4px 9px;
  }
  .mpm-failure-text {
    display: block;
    font-size: 10.5px;
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mpm-failure-warn {
    color: var(--app-warn, #c9920a);
  }
  .mpm-retry {
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
  .mpm-retry:hover:not(:disabled) {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .mpm-retry:disabled {
    cursor: default;
    color: var(--app-text-faint);
  }
</style>
