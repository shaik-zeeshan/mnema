<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // ModelPickerMenu — the shared model-picker UI: a trigger that opens a search
  // box over a provider-grouped listbox. ONE picker for every place a model is
  // chosen (the Chat composer's per-chat pin and the Settings default-model /
  // Ask AI-override fields), so they never drift apart. Kit look (chat.html ›
  // Model picker): `.mx-pop` + `.mx-search`, provider groups with a cloud/local
  // chip, a per-provider loading skeleton or failure + Retry.
  //
  // Purely presentational: the PARENT owns the pool, the committed selection and
  // what a choice means. A typed id no provider lists is offered explicitly (PER
  // provider when `exactIdPerProvider`) so it never resolves to a guessed
  // provider; `null` (the sentinel row) reports "clear". CSS-positioned relative
  // to the trigger, never portaled (WKWebView measures across Settings' scroll
  // container in a different space). See ADR 0033/0034.
  import { tick, type Snippet } from "svelte";
  import {
    providerLabelById,
    pinnableEnginesFromModelPool,
    shortModelLabel,
  } from "$lib/insights/conversation";
  import type { AiProviderConfig, AiRuntimeModel } from "$lib/types/recording";
  import { providerVia, providerWhere } from "./chat-format";
  import IconSearch from "~icons/lucide/search";
  import IconChev from "~icons/lucide/chevron-down";
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";

  let {
    label,
    title,
    ariaLabel,
    disabled = false,
    block = false,
    placeholder = false,
    up = false,
    glyph = null,
    where = null,
    modelPool,
    providers,
    firstProvider = null,
    sentinelLabel = null,
    sentinelDetail = null,
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
    foot,
  }: {
    /** Short label shown in the trigger (what's currently selected). */
    label: string;
    /** Full, unshortened label for the trigger's tooltip. */
    title: string;
    ariaLabel: string;
    disabled?: boolean;
    /** Full-width form-control trigger (Settings) vs the inline Chat trigger. */
    block?: boolean;
    /** Render the block trigger's label muted (nothing selected yet). */
    placeholder?: boolean;
    /** Inline only: open upward (the composer is docked at the bottom). */
    up?: boolean;
    /** Inline only: the provider letter + cloud/local chip on the trigger. */
    glyph?: string | null;
    where?: "cloud" | "local" | null;
    modelPool: AiRuntimeModel[];
    /** Connected providers — drives group labels/order and the typed-id rows. */
    providers: AiProviderConfig[] | null | undefined;
    /** Provider whose group sorts first (the default's), or null. */
    firstProvider?: string | null;
    /** The sentinel/"clear" row label (e.g. "Default"), or null to hide it. */
    sentinelLabel?: string | null;
    /** A quiet mono detail after the sentinel label (what it resolves to). */
    sentinelDetail?: string | null;
    sentinelTitle?: string | null;
    sentinelSelected?: boolean;
    /** Committed provider id (null → match a row by model id alone). */
    selectedProvider?: string | null;
    selectedModel?: string | null;
    allowExactId?: boolean;
    exactIdPerProvider?: boolean;
    /** True while the pool is still being listed. */
    loading?: boolean;
    /** Providers that failed to list, surfaced with a Retry. */
    failures?: { provider: string; label: string; reason: string }[];
    onretry?: () => void;
    open?: boolean;
    /** Fired when the menu opens, so the parent can (re)load the pool. */
    onopen?: () => void;
    /** Commit a chosen engine, or `null` for the sentinel/"clear" row. */
    onselect: (engine: { provider: string; model: string } | null) => void;
    /** Optional footer line inside the popover. */
    foot?: Snippet;
  } = $props();

  let query = $state("");
  let highlight = $state(0);
  let searchEl = $state<HTMLInputElement | null>(null);
  let triggerEl = $state<HTMLButtonElement | null>(null);
  let closeTimer: ReturnType<typeof setTimeout> | null = null;
  // Per-option refs (flat index) so keyboard nav keeps the row in view.
  let optionEls: Record<number, HTMLButtonElement | null> = {};
  // Block (Settings) flip: open upward when there isn't room below.
  let openUp = $state(false);

  function scrollHighlightIntoView(): void {
    if (!open) return;
    void tick().then(() => optionEls[highlight]?.scrollIntoView({ block: "nearest" }));
  }

  const letter = (id: string) => providerLabelById(providers, id).trim().charAt(0).toUpperCase() || "?";
  const config = (id: string) => providers?.find((p) => p.id === id);

  let pinnableEngines = $derived(pinnableEnginesFromModelPool(modelPool, providers));

  // Provider order: the default's first, then config order, then any pool-only.
  let providerOrder = $derived.by(() => {
    const ids: string[] = [];
    const add = (id: string) => !ids.includes(id) && ids.push(id);
    if (firstProvider !== null) add(firstProvider);
    for (const p of providers ?? []) add(p.id);
    for (const e of pinnableEngines) add(e.provider);
    return ids;
  });

  type PickerOption =
    | { kind: "sentinel" }
    | { kind: "pool"; provider: string; model: string }
    | { kind: "exact"; provider: string; model: string };
  type Row =
    | { type: "header"; key: string; provider: string | null; label: string }
    | { type: "loading"; key: string; label: string }
    | { type: "failure"; key: string; text: string }
    | { type: "option"; key: string; option: PickerOption; glyph: string; main: string; sub: string | null; title: string; selected: boolean; index: number };

  let pickerRows = $derived.by(() => {
    const rows: Row[] = [];
    let index = 0;
    const q = query.trim().toLowerCase();
    const raw = query.trim();
    const addOption = (option: PickerOption, g: string, main: string, sub: string | null, rowTitle: string, selected: boolean) => {
      rows.push({ type: "option", key: `opt-${index}`, option, glyph: g, main, sub, title: rowTitle, selected, index });
      index++;
    };

    if (sentinelLabel !== null && (q.length === 0 || sentinelLabel.toLowerCase().includes(q))) {
      addOption({ kind: "sentinel" }, sentinelDetail && firstProvider ? letter(firstProvider) : "–", sentinelLabel, null, sentinelTitle ?? sentinelLabel, sentinelSelected);
    }

    for (const provider of providerOrder) {
      const groupLabel = providerLabelById(providers, provider);
      const header: Row = { type: "header", key: `hdr-${provider}`, provider, label: groupLabel };
      const failure = failures.find((f) => f.provider === provider);
      const models = pinnableEngines.filter((e) => e.provider === provider).map((e) => e.model);
      if (q.length === 0 && failure) {
        rows.push(header, { type: "failure", key: `fail-${provider}`, text: `${failure.label} — ${failure.reason}` });
        continue;
      }
      if (q.length === 0 && loading && models.length === 0) {
        rows.push(header, { type: "loading", key: `load-${provider}`, label: groupLabel });
        continue;
      }
      const matches = models.filter((m) => q.length === 0 || m.toLowerCase().includes(q) || groupLabel.toLowerCase().includes(q));
      if (matches.length === 0) continue;
      rows.push(header);
      for (const model of matches) {
        const ctx = modelPool.find((m) => m.provider === provider && m.id === model)?.contextWindow ?? null;
        const selected = model === selectedModel && (selectedProvider === null || provider === selectedProvider);
        addOption({ kind: "pool", provider, model }, letter(provider), shortModelLabel(model), ctx ? `${Math.round(ctx / 1000)}k` : null, model, selected);
      }
    }

    // A typed id that isn't an exact pool match, offered per provider.
    if (allowExactId && raw.length > 0 && !pinnableEngines.some((e) => e.model === raw)) {
      const connected = providers ?? [];
      if (exactIdPerProvider) {
        if (connected.length > 0) {
          rows.push({ type: "header", key: "hdr-exact", provider: null, label: "Use as a model id" });
          for (const p of connected) {
            const pl = providerLabelById(providers, p.id);
            addOption({ kind: "exact", provider: p.id, model: raw }, letter(p.id), raw, pl, `${pl} · ${raw}`, false);
          }
        }
      } else {
        rows.push({ type: "header", key: "hdr-exact", provider: null, label: "Use as a model id" });
        addOption({ kind: "exact", provider: connected[0]?.id ?? "", model: raw }, connected[0] ? letter(connected[0].id) : "·", raw, null, raw, false);
      }
    }
    return rows;
  });

  // "Loading models…" sits under the sentinel row (or on top without one).
  let loadingNoteAt = $derived(pickerRows[0]?.type === "option" && pickerRows[0].option.kind === "sentinel" ? 1 : 0);
  let pickerOptions = $derived(pickerRows.filter((row) => row.type === "option"));
  let noMatch = $derived(query.trim().length > 0 && !pickerRows.some((r) => r.type === "option" && r.option.kind !== "exact"));
  let activeDescendantId = $derived(
    pickerOptions.some((row) => row.type === "option" && row.index === highlight) ? `mpm-opt-${highlight}` : null,
  );

  function recomputeOpenDirection(): void {
    if (!block || !triggerEl) {
      openUp = false;
      return;
    }
    const rect = triggerEl.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom;
    openUp = spaceBelow < 340 && rect.top > spaceBelow;
  }

  function openMenu(): void {
    if (closeTimer !== null) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
    open = true;
    query = "";
    highlight = 0;
    recomputeOpenDirection();
    onopen?.();
    void tick().then(() => searchEl?.focus());
  }

  function closeMenu(): void {
    open = false;
    query = "";
  }

  // Keyboard closes (Escape, Enter-commit) return focus to the trigger.
  function closeMenuToTrigger(): void {
    closeMenu();
    void tick().then(() => triggerEl?.focus());
  }

  function toggleMenu(): void {
    if (disabled) return;
    if (open) closeMenu();
    else openMenu();
  }

  // Close shortly after the search blurs, so an option's mousedown→click (kept
  // off the search via preventDefault) still lands first.
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
      scrollHighlightIntoView();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      highlight = Math.max(highlight - 1, 0);
      scrollHighlightIntoView();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const choice = options[highlight];
      if (choice && choice.type === "option") {
        selectOption(choice.option);
        void tick().then(() => triggerEl?.focus());
      }
    } else if (event.key === "Escape") {
      event.stopPropagation();
      closeMenuToTrigger();
    }
  }
</script>

{#snippet whereChip(w: "cloud" | "local")}
  <span class="mx-chip mx-chip--{w === 'local' ? 'accent' : 'info'}">{w}</span>
{/snippet}

<div class="mx-pop-anchor" class:mpm--block={block}>
  {#if block}
    <button
      type="button"
      bind:this={triggerEl}
      class="mpm-trigger"
      class:mpm-trigger--placeholder={placeholder}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label={ariaLabel}
      use:tip={title}
      {disabled}
      onclick={toggleMenu}
    >
      <span class="mpm-current">{label}</span>
      <IconChev width="14" height="14" aria-hidden="true" />
    </button>
  {:else}
    <button
      type="button"
      bind:this={triggerEl}
      class="mx-btn mx-btn--ghost ch-model"
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label={ariaLabel}
      use:tip={title}
      {disabled}
      onclick={toggleMenu}
    >
      {#if glyph}<span class="ch-glyph" aria-hidden="true">{glyph}</span>{/if}
      <span class="mono">{label}</span>
      {#if where}{@render whereChip(where)}{/if}
      <IconChev width="12" height="12" aria-hidden="true" />
    </button>
  {/if}
  {#if open}
    <div class="mx-pop ch-pop" class:ch-pop--block={block} class:ch-pop--up={block ? openUp : up} data-open role="dialog" aria-label="Choose model">
      <label class="mx-search">
        <IconSearch width="13" height="13" aria-hidden="true" />
        <input
          bind:this={searchEl}
          bind:value={query}
          type="text"
          role="combobox"
          aria-expanded="true"
          aria-controls="mpm-list"
          aria-activedescendant={activeDescendantId}
          aria-label="Search models"
          placeholder="Search models or type an id"
          spellcheck="false"
          autocomplete="off"
          oninput={() => (highlight = 0)}
          onkeydown={onSearchKeydown}
          onblur={closeSoon}
        />
      </label>
      <div id="mpm-list" class="ch-opts" role="listbox" aria-label="Model">
        {#if noMatch}
          <p class="mx-inline ch-opts__note">No models match “{query.trim()}”.</p>
        {/if}
        {#each pickerRows as row, ri (row.key)}
          {#if ri === loadingNoteAt && loading && query.trim().length === 0}
            <p class="mx-inline ch-opts__note"><span class="mx-spin mx-spin--sm"></span>Loading models…</p>
          {/if}
          {#if row.type === "header"}
            {@const p = row.provider ? config(row.provider) : undefined}
            <h6>{row.label}{#if p}{@render whereChip(providerWhere(p))}<small>{providerVia(p)}</small>{/if}</h6>
          {:else if row.type === "loading"}
            <div class="ch-opts__pad" aria-busy="true">
              <span class="mx-sr">Loading {row.label} models</span>
              <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
            </div>
          {:else if row.type === "failure"}
            <div class="ch-opts__pad">
              <span class="mx-inline" data-tone="warn">
                <IconAlert width="13" height="13" aria-hidden="true" />{row.text}
                {#if onretry}
                  <button
                    type="button"
                    class="mx-btn mx-btn--ghost mx-btn--sm"
                    disabled={loading}
                    onmousedown={(e) => e.preventDefault()}
                    onclick={() => onretry?.()}>{loading ? "Retrying…" : "Retry"}</button
                  >
                {/if}
              </span>
            </div>
          {:else}
            <button
              type="button"
              bind:this={optionEls[row.index]}
              id={`mpm-opt-${row.index}`}
              class="ch-opt"
              class:ch-opt--default={row.option.kind === "sentinel"}
              class:ch-opt--cursor={row.index === highlight}
              role="option"
              aria-selected={row.selected}
              use:tip={row.title}
              onmousedown={(e) => e.preventDefault()}
              onmouseenter={() => (highlight = row.index)}
              onclick={() => selectOption(row.option)}
            >
              <span class="ch-glyph" aria-hidden="true">{row.glyph}</span>
              <span class="ch-opt__main"
                >{row.main}{#if row.option.kind === "sentinel" && sentinelDetail}<em>{shortModelLabel(sentinelDetail)}</em>{/if}</span
              >
              <small>{row.sub ?? ""}</small>
              <IconCheck class="ch-tick" width="13" height="13" aria-hidden="true" />
            </button>
          {/if}
        {/each}
        {#if !loading && pickerRows.length === 0}
          <p class="mx-inline ch-opts__note">No models yet.</p>
        {/if}
      </div>
      {#if foot}<div class="ch-pop__foot">{@render foot()}</div>{/if}
    </div>
  {/if}
</div>

<style>
  .mpm--block { width: 100%; }
  /* Settings' block trigger: a full-width form control (the .mx-input idiom). */
  .mpm-trigger {
    width: 100%;
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: var(--h-md);
    padding: 0 var(--s-3);
    border: 1px solid var(--app-border-strong);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    font: 400 12px/1 var(--font-mono);
    cursor: pointer;
    transition: border-color var(--t-fast) var(--ease-quart), box-shadow var(--t-med) var(--ease-quart);
  }
  .mpm-trigger:hover:not(:disabled) { border-color: var(--app-border-hover); }
  .mpm-trigger:focus-visible,
  .mpm-trigger[aria-expanded="true"] { outline: none; border-color: var(--app-accent-border); box-shadow: var(--app-ring); }
  .mpm-trigger:disabled { cursor: default; opacity: 0.6; }
  .mpm-trigger--placeholder .mpm-current { color: var(--app-text-faint); }
  .mpm-trigger :global(svg) { flex: none; color: var(--app-text-subtle); transition: transform var(--t-med) var(--ease-expo); }
  .mpm-trigger[aria-expanded="true"] :global(svg) { transform: rotate(180deg); }
  .mpm-current { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }

  /* Chat's inline trigger: a ghost button with the provider glyph + chip. */
  .ch-model { --_px: 8px; }
  .ch-model[aria-expanded="true"] { --_bg: var(--mx-wash-strong); --_fg: var(--app-text-strong); }
  .ch-model .mono { font: 500 var(--text-sm)/1 var(--font-mono); max-width: 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ch-model > :global(svg:last-child) { color: var(--app-text-subtle); }
  .ch-glyph {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    font: 600 10px/1 var(--font-sans);
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
    box-shadow: inset 0 0 0 1px var(--app-border-strong);
  }
  .ch-model .mx-chip,
  .ch-opts h6 .mx-chip { height: 18px; padding: 0 5px; }

  .ch-pop {
    left: 0;
    right: auto;
    top: calc(100% + 6px);
    width: 360px;
    z-index: 60;
    transform-origin: top left;
    background: var(--app-surface-raised);
    border-color: var(--app-border-strong);
  }
  .ch-pop--block { width: 100%; min-width: 300px; }
  .ch-pop--up { top: auto; bottom: calc(100% + 6px); transform-origin: bottom left; }
  .ch-pop .mx-search { margin: var(--s-2) var(--s-2) 0; }
  .ch-opts { max-height: 290px; overflow: auto; padding: 4px; }
  .ch-opts h6 { display: flex; align-items: center; gap: 6px; margin: 10px 8px 4px; font: 600 var(--text-sm)/1 var(--font-sans); color: var(--app-text-muted); }
  .ch-opts h6 small { margin-left: auto; font: 400 var(--text-xs)/1 var(--font-sans); color: var(--app-text-subtle); }
  .ch-opts__note { padding: 6px 8px 2px; font-size: var(--text-sm); }
  .ch-opts__pad { padding: 4px 8px 6px; }
  .ch-opt {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto 14px;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    cursor: pointer;
    text-align: left;
    color: var(--app-text);
    font: 500 var(--text-sm)/1.2 var(--font-mono);
  }
  .ch-opt:hover,
  .ch-opt--cursor { background: var(--app-surface-hover); color: var(--app-text-strong); }
  .ch-opt__main { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ch-opt small { font: 400 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); }
  .ch-opt :global(.ch-tick) { color: var(--app-accent); visibility: hidden; }
  .ch-opt[aria-selected="true"] { color: var(--app-text-strong); }
  .ch-opt[aria-selected="true"] :global(.ch-tick) { visibility: visible; }
  .ch-opt--default .ch-opt__main { font-family: var(--font-sans); }
  .ch-opt--default em { font-style: normal; color: var(--app-text-subtle); font-family: var(--font-mono); font-size: var(--text-xs); margin-left: 6px; }
  .ch-pop__foot {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 9px 12px;
    border-top: 1px solid var(--app-overlay-border);
    font: 400 var(--text-sm)/1.4 var(--font-sans);
    color: var(--app-text-subtle);
  }
</style>
