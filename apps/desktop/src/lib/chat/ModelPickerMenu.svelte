<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // ModelPickerMenu — the shared model-picker UI: a trigger that opens a search
  // box over the model list. ONE picker for every place a model is chosen (the
  // Chat composer's per-chat pin and the Settings default-model / Ask AI-override
  // fields), so they never drift apart. Kit look (chat.html › Model picker):
  // `.mx-pop` + `.mx-search`, provider groups with a cloud/local chip, a
  // per-provider loading skeleton or failure + Retry.
  //
  // Favorites first (docs/mockups/redesign/model-picker.html): it opens on a
  // short list — Default, the starred models (keys 1-9) and the last couple
  // picked — with every provider behind "Browse all" as folding groups; typing
  // searches everything. With nothing starred or recent it opens on Browse.
  // Rows live in `model-picker-rows.ts` (pure); this file renders + handles keys.
  //
  // Purely presentational: the PARENT owns the pool, the committed selection,
  // the starred/recent lists and what a choice means. A typed id no provider
  // lists is offered explicitly (PER provider when `exactIdPerProvider`) so it
  // never resolves to a guessed provider; `null` (the sentinel row) reports
  // "clear". CSS-positioned relative to the trigger, never portaled (WKWebView
  // measures across Settings' scroll container in a different space).
  import { tick, type Snippet } from "svelte";
  import { providerLabelById, pinnableEnginesFromModelPool, modelDisplayName } from "$lib/insights/conversation";
  import type { AiEngineRef, AiProviderConfig, AiRuntimeModel } from "$lib/types/recording";
  import { providerVia, providerWhere } from "./chat-format";
  import { buildPickerRows, hasQuickView, type PickerMode, type PickerOption } from "./model-picker-rows";
  import IconSearch from "~icons/lucide/search";
  import IconChev from "~icons/lucide/chevron-down";
  import IconRight from "~icons/lucide/chevron-right";
  import IconBack from "~icons/lucide/arrow-left";
  import IconCheck from "~icons/lucide/check";
  import IconStar from "~icons/lucide/star";
  import IconClock from "~icons/lucide/clock";
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
    favorites = [],
    recents = [],
    onfavorite,
    defaultEngine = null,
    onmakedefault,
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
    /** Starred models in star order (the first nine get keys 1-9). */
    favorites?: AiEngineRef[];
    /** Recently picked models, most recent first. */
    recents?: AiEngineRef[];
    /** Star/unstar a model; omit to hide the stars. */
    onfavorite?: (engine: AiEngineRef) => void;
    /** The current global default (hides "Make default" on that row). */
    defaultEngine?: AiEngineRef | null;
    /** Make a model the global default; omit to hide the action. */
    onmakedefault?: (engine: AiEngineRef) => void;
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
  let mode = $state<PickerMode>("quick");
  let groupOpen = $state<Record<string, boolean>>({});
  let searchEl = $state<HTMLInputElement | null>(null);
  let triggerEl = $state<HTMLButtonElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  let closeTimer: ReturnType<typeof setTimeout> | null = null;
  // Per-row refs (flat index) so keyboard nav keeps the row in view.
  let rowEls: Record<number, HTMLElement | null> = {};
  // Block (Settings) flip: open upward when there isn't room below.
  let openUp = $state(false);

  function scrollHighlightIntoView(): void {
    if (!open) return;
    void tick().then(() => rowEls[highlight]?.scrollIntoView({ block: "nearest" }));
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

  let pickerRows = $derived(
    buildPickerRows({
      query,
      mode,
      groupOpen,
      providerOrder,
      engines: pinnableEngines,
      contextWindow: (provider, model) => modelPool.find((m) => m.provider === provider && m.id === model)?.contextWindow ?? null,
      providerLabel: (id) => providerLabelById(providers, id),
      letter,
      connected: (providers ?? []).map((p) => p.id),
      sentinel:
        sentinelLabel === null
          ? null
          : {
              label: sentinelLabel,
              detail: sentinelDetail,
              title: sentinelTitle ?? sentinelLabel,
              selected: sentinelSelected,
              glyph: sentinelDetail && firstProvider ? letter(firstProvider) : "–",
            },
      selectedProvider,
      selectedModel,
      favorites,
      recents,
      canStar: onfavorite !== undefined,
      makeDefaultFor: onmakedefault ? { current: defaultEngine } : null,
      loading,
      failures,
      allowExactId,
      exactIdPerProvider,
    }),
  );
  let navRows = $derived(pickerRows.filter((r) => r.type === "option" || r.type === "group" || r.type === "action"));
  let activeDescendantId = $derived(navRows.some((r) => "index" in r && r.index === highlight) ? `mpm-row-${highlight}` : null);
  let showLoadingNote = $derived(loading && query.trim().length === 0 && mode === "quick" && hasQuickView(favorites, recents));

  function recomputeOpenDirection(): void {
    if (!block || !triggerEl) {
      openUp = false;
      return;
    }
    const rect = triggerEl.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom;
    openUp = spaceBelow < 340 && rect.top > spaceBelow;
  }

  function setMode(next: PickerMode): void {
    mode = next;
    highlight = 0;
    if (listEl) listEl.scrollTop = 0;
  }

  function openMenu(): void {
    if (closeTimer !== null) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
    open = true;
    query = "";
    highlight = 0;
    groupOpen = {};
    mode = hasQuickView(favorites, recents) ? "quick" : "browse";
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

  // Close shortly after the search blurs, so a row's mousedown→click (kept off
  // the search via preventDefault) still lands first.
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

  function toggleGroup(provider: string, isOpen: boolean): void {
    groupOpen = { ...groupOpen, [provider]: !isOpen };
  }

  // Run a nav row's action (click or Enter).
  function activate(row: (typeof navRows)[number], viaKeyboard: boolean): void {
    if (row.type === "group") toggleGroup(row.provider, row.open);
    else if (row.type === "action") setMode(row.action === "browse" ? "browse" : "quick");
    else {
      selectOption(row.option);
      if (viaKeyboard) void tick().then(() => triggerEl?.focus());
    }
  }

  function starRow(row: (typeof navRows)[number] | undefined): void {
    if (row?.type === "option" && row.option.kind === "pool" && row.starred !== null) {
      onfavorite?.({ provider: row.option.provider, model: row.option.model });
    }
  }

  function onSearchKeydown(event: KeyboardEvent): void {
    const current = navRows.find((r) => r.index === highlight);
    if (/^[1-9]$/.test(event.key) && !event.metaKey && !event.ctrlKey && !event.altKey && query.length === 0 && mode === "quick") {
      // Number keys pick a favorite while the search is empty (so typing "4o" still searches).
      const fav = favorites[Number(event.key) - 1];
      if (!fav) return;
      event.preventDefault();
      selectOption({ kind: "pool", provider: fav.provider, model: fav.model });
      void tick().then(() => triggerEl?.focus());
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      highlight = Math.min(highlight + 1, navRows.length - 1);
      scrollHighlightIntoView();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      highlight = Math.max(highlight - 1, 0);
      scrollHighlightIntoView();
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (current) activate(current, true);
    } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "d") {
      event.preventDefault();
      starRow(current);
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
      <div id="mpm-list" class="ch-opts" role="listbox" aria-label="Model" bind:this={listEl}>
        {#each pickerRows as row, ri (row.key)}
          {#if ri === 1 && showLoadingNote}
            <p class="mx-inline ch-opts__note"><span class="mx-spin mx-spin--sm"></span>Loading models…</p>
          {/if}
          {#if row.type === "header"}
            {@const p = row.provider ? config(row.provider) : undefined}
            <h6>
              {#if row.icon === "star"}<IconStar width="11" height="11" aria-hidden="true" />{:else if row.icon === "clock"}<IconClock width="11" height="11" aria-hidden="true" />{/if}
              {row.label}{#if p}{@render whereChip(providerWhere(p))}<small>{providerVia(p)}</small>{:else if row.hint}<small>{row.hint}</small>{/if}
            </h6>
          {:else if row.type === "group"}
            {@const p = config(row.provider)}
            <div
              bind:this={rowEls[row.index]}
              id={`mpm-row-${row.index}`}
              class="mpm-group"
              class:is-cursor={row.index === highlight}
              role="option"
              aria-selected="false"
              aria-label={`${row.label}, ${row.open ? "expanded" : "collapsed"}`}
              data-open={row.open}
              tabindex="-1"
              onmousedown={(e) => e.preventDefault()}
              onmouseenter={() => (highlight = row.index)}
              onclick={() => activate(row, false)}
              onkeydown={() => {}}
            >
              <IconRight class="mpm-chev" width="12" height="12" aria-hidden="true" />
              {row.label}{#if p}{@render whereChip(providerWhere(p))}{/if}
              <small>{row.count === null ? "" : `${row.count} model${row.count === 1 ? "" : "s"}`}</small>
            </div>
          {:else if row.type === "action"}
            {#if row.action === "browse"}<div class="mpm-sep" aria-hidden="true"></div>{/if}
            <div
              bind:this={rowEls[row.index]}
              id={`mpm-row-${row.index}`}
              class="mpm-act"
              class:is-cursor={row.index === highlight}
              role="option"
              aria-selected="false"
              tabindex="-1"
              onmousedown={(e) => e.preventDefault()}
              onmouseenter={() => (highlight = row.index)}
              onclick={() => activate(row, false)}
              onkeydown={() => {}}
            >
              {#if row.action === "back"}<IconBack width="12" height="12" aria-hidden="true" />{/if}
              {row.label}
              {#if row.detail}<small>{row.detail}</small>{/if}
              {#if row.action === "browse"}<IconRight width="12" height="12" aria-hidden="true" />{/if}
            </div>
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
          {:else if row.type === "note"}
            <p class="mx-inline ch-opts__note">{row.text}</p>
          {:else}
            {@const engine = row.option.kind === "sentinel" ? null : { provider: row.option.provider, model: row.option.model }}
            <div
              bind:this={rowEls[row.index]}
              id={`mpm-row-${row.index}`}
              class="ch-opt"
              class:ch-opt--default={row.option.kind === "sentinel"}
              class:is-cursor={row.index === highlight}
              role="option"
              aria-selected={row.selected}
              tabindex="-1"
              use:tip={row.title}
              onmousedown={(e) => e.preventDefault()}
              onmouseenter={() => (highlight = row.index)}
              onclick={() => activate(row, false)}
              onkeydown={() => {}}
            >
              <span class="ch-glyph" aria-hidden="true">{row.glyph}</span>
              <span class="ch-opt__main"
                >{row.main.before}{#if row.main.match}<mark>{row.main.match}</mark>{row.main.after}{/if}{#if row.option.kind === "sentinel" && sentinelDetail}<em>{modelDisplayName(sentinelDetail)}</em>{/if}</span
              >
              <span class="ch-opt__meta">
                {#if row.canMakeDefault && engine}
                  <button
                    type="button"
                    class="mpm-mkdef"
                    tabindex="-1"
                    onmousedown={(e) => e.preventDefault()}
                    onclick={(e) => {
                      e.stopPropagation();
                      onmakedefault?.(engine);
                    }}>Make default</button
                  >
                {/if}
                {#if row.num !== null}<kbd class="mpm-num">{row.num}</kbd>{:else if row.sub}<small>{row.sub}</small>{/if}
              </span>
              {#if row.starred !== null && engine}
                <button
                  type="button"
                  class="mpm-star"
                  tabindex="-1"
                  aria-pressed={row.starred}
                  aria-label={row.starred ? `Unstar ${engine.model}` : `Star ${engine.model}`}
                  onmousedown={(e) => e.preventDefault()}
                  onclick={(e) => {
                    e.stopPropagation();
                    onfavorite?.(engine);
                  }}><IconStar width="12" height="12" aria-hidden="true" /></button
                >
              {:else}
                <span></span>
              {/if}
              <IconCheck class="ch-tick" width="13" height="13" aria-hidden="true" />
            </div>
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
  .ch-opts h6 .mx-chip,
  .mpm-group .mx-chip { height: 18px; padding: 0 5px; }

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
  .ch-opts { max-height: 320px; overflow: auto; padding: 4px; }
  .ch-opts h6 { display: flex; align-items: center; gap: 6px; margin: 10px 8px 4px; font: 600 var(--text-sm)/1 var(--font-sans); color: var(--app-text-muted); }
  .ch-opts h6 :global(svg) { color: var(--app-text-subtle); }
  .ch-opts h6 small { margin-left: auto; font: 400 var(--text-xs)/1 var(--font-sans); color: var(--app-text-subtle); }
  .ch-opts__note { padding: 6px 8px 2px; font-size: var(--text-sm); }
  .ch-opts__pad { padding: 4px 8px 6px; }
  .ch-opt {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto 20px 14px;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border-radius: var(--r-sm);
    cursor: pointer;
    text-align: left;
    color: var(--app-text);
    font: 500 var(--text-sm)/1.2 var(--font-mono);
  }
  .ch-opt:hover,
  .ch-opt.is-cursor,
  .mpm-group.is-cursor,
  .mpm-act.is-cursor { background: var(--app-surface-hover); color: var(--app-text-strong); }
  .ch-opt__main { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ch-opt__main mark { background: none; color: var(--app-accent); font-weight: 700; }
  .ch-opt__meta { display: inline-flex; align-items: center; gap: 6px; }
  .ch-opt small { font: 400 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); white-space: nowrap; }
  .ch-opt :global(.ch-tick) { color: var(--app-accent); visibility: hidden; }
  .ch-opt[aria-selected="true"] { color: var(--app-text-strong); }
  .ch-opt[aria-selected="true"] :global(.ch-tick) { visibility: visible; }
  .ch-opt--default .ch-opt__main { font-family: var(--font-sans); }
  .ch-opt--default em { font-style: normal; color: var(--app-text-subtle); font-family: var(--font-mono); font-size: var(--text-xs); margin-left: 6px; }

  /* Star: shown on hover / keyboard cursor, always once starred. */
  .mpm-star {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 5px;
    background: none;
    cursor: pointer;
    color: var(--app-text-subtle);
    visibility: hidden;
  }
  .ch-opt:hover .mpm-star,
  .ch-opt.is-cursor .mpm-star,
  .mpm-star[aria-pressed="true"] { visibility: visible; }
  .mpm-star:hover { background: var(--mx-wash-strong); color: var(--app-text-strong); }
  .mpm-star[aria-pressed="true"] { color: var(--app-warn); }
  .mpm-star[aria-pressed="true"] :global(svg *) { fill: currentColor; }
  .mpm-num {
    display: inline-grid;
    place-items: center;
    min-width: 16px;
    height: 16px;
    padding: 0 3px;
    font: 500 10px/1 var(--font-mono);
  }
  /* "Make default" replaces the meta on hover / keyboard cursor. */
  .mpm-mkdef {
    display: none;
    padding: 2px 6px;
    border: 0;
    border-radius: 4px;
    background: none;
    cursor: pointer;
    font: 500 var(--text-xs)/1.2 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .ch-opt:hover .mpm-mkdef,
  .ch-opt.is-cursor .mpm-mkdef { display: inline-block; }
  .ch-opt:hover .mpm-mkdef ~ :is(small, .mpm-num),
  .ch-opt.is-cursor .mpm-mkdef ~ :is(small, .mpm-num) { display: none; }
  .mpm-mkdef:hover { background: var(--mx-wash-strong); color: var(--app-text-strong); }

  /* Browse: folding provider header, pinned while its models scroll under it. */
  .mpm-group {
    position: sticky;
    top: -4px;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    padding: 8px;
    border-radius: var(--r-sm);
    background: var(--app-surface-raised);
    cursor: pointer;
    font: 600 var(--text-sm)/1 var(--font-sans);
    color: var(--app-text-muted);
  }
  .mpm-group:hover { color: var(--app-text-strong); }
  .mpm-group :global(.mpm-chev) { color: var(--app-text-subtle); transition: transform var(--t-fast) var(--ease-quart); }
  .mpm-group[data-open="true"] :global(.mpm-chev) { transform: rotate(90deg); }
  .mpm-group small { margin-left: auto; font: 400 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); }
  .mpm-act {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border-radius: var(--r-sm);
    cursor: pointer;
    font: 500 var(--text-sm)/1.2 var(--font-sans);
    color: var(--app-text-muted);
  }
  .mpm-act:hover { background: var(--app-surface-hover); color: var(--app-text-strong); }
  .mpm-act small { margin-left: auto; font: 400 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); }
  .mpm-sep { height: 1px; margin: 6px 4px; background: var(--app-overlay-border); }
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
