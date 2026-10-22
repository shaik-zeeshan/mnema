<!-- What Quick Recall says instead of results (recall.html first run / search
     failed / no results) and the one notice line that rides above results when
     they may be partial (indexing behind, meaning off or building, audio still
     being transcribed, not recording). The page mounts the component in place
     of the results list while `search.states.emptyState` is set, and renders
     the module-level `noticeLine` snippet above the results. No results offers
     ways out as rows with counts — the same search with one filter loosened —
     and ↑/↓ + ↵ walk them; ↵ on Search failed retries. -->
<script module lang="ts">
  import IconInfo from "~icons/lucide/info";
  import IconBan from "~icons/lucide/ban";
  import IconAlert from "~icons/lucide/triangle-alert";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { searchNotice } from "./recall-states";

  const notice = () =>
    searchNotice(
      search.states.indexStatus,
      { semanticDegraded: search.states.semanticDegraded, untranscribed: search.states.untranscribed },
      search.states.recording,
      Date.now(),
    );

  export { noticeLine };
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import IconInbox from "~icons/lucide/inbox";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconSparkle from "~icons/lucide/sparkle";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import { pillLabel } from "./filter-chips";
  import { countLabel, waysOut, type CountLabel, type WayOut } from "./recall-states";

  let { askAvailable, onAskAi }: { askAvailable: boolean; onAskAi: () => void } = $props();

  const states = search.states;
  const filters = search.filters;
  let rows = $state<{ way: WayOut; n: CountLabel | null }[]>([]);
  let selected = $state(0);

  // The ways out for this query: each loosened search counted, empty ones dropped.
  $effect(() => {
    rows = [];
    if (states.emptyState !== "none") return;
    const ways = waysOut(search.naturalParse.residual, search.effectivePills, askAvailable);
    let live = true;
    void Promise.all(
      ways.map((way) => (way.kind === "ask" ? null : states.countFacet(way.call))),
    ).then((counts) => {
      if (!live) return;
      rows = ways.flatMap((way, i): typeof rows => {
        const c = counts[i];
        if (way.kind === "ask") return [{ way, n: null }];
        return c && c.total > 0 ? [{ way, n: countLabel(c.total, c.capped) }] : [];
      });
      selected = 0;
    });
    return () => (live = false);
  });

  function apply(i: number): void {
    const way = rows[i]?.way;
    if (!way) return;
    if (way.kind === "ask") return onAskAi();
    search.query = search.naturalParse.residual;
    search.pills = way.pills;
    search.inputEl?.focus();
  }

  function onKey(event: KeyboardEvent): void {
    if (event.isComposing || event.metaKey || event.ctrlKey || event.altKey) return;
    if (filters.popoverOpen || filters.pillIndex !== null) return;
    if (states.emptyState === "error" && event.key === "Enter") {
      void search.runSearch(search.resultsQuery);
    } else if (states.emptyState === "none" && rows.length > 0 && event.key === "ArrowDown") {
      selected = Math.min(rows.length - 1, selected + 1);
    } else if (states.emptyState === "none" && event.key === "ArrowUp") {
      selected = Math.max(0, selected - 1);
    } else if (states.emptyState === "none" && rows.length > 0 && event.key === "Enter") {
      apply(selected);
    } else return;
    event.preventDefault();
    event.stopPropagation();
  }

  async function openMnema(): Promise<void> {
    await invoke("toggle_main_window_visibility_command").catch(() => {});
    await closeCurrentWindow();
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#snippet noticeLine(always = false)}
  {@const n = always || (search.hasResults && !search.states.emptyState) ? notice() : null}
  {#if n}
    <div class="rc-note">
      <span class="mx-inline" data-tone={n.tone ?? undefined}>
        {#if n.icon === "info"}<IconInfo width="13" height="13" aria-hidden="true" />
        {:else if n.icon === "off"}<IconBan width="13" height="13" aria-hidden="true" />
        {:else}<IconAlert width="13" height="13" aria-hidden="true" />{/if}
        {n.text}
        {#if n.settings}
          <button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm"
            onclick={() => void search.openSemanticSearchSettings()}>Turn on in Settings →</button
          >
        {/if}
      </span>
    </div>
  {/if}
{/snippet}

<div>
  {#if states.emptyState === "first"}
    <div class="mx-empty mx-empty--center">
      <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
      <h4 class="mx-empty__title">Nothing captured yet</h4>
      <p class="mx-empty__text">
        Quick Recall searches what Mnema records. Start recording and this fills in as you work.
      </p>
      <div class="mx-empty__acts">
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => void openMnema()}>Open Mnema</button>
      </div>
    </div>
  {:else if states.emptyState === "error"}
    <div class="mx-empty mx-empty--center" data-tone="danger" role="alert">
      <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
      <h4 class="mx-empty__title">Search failed</h4>
      <p class="mx-empty__text">The search index didn’t answer. Nothing is lost — try again.</p>
      <div class="mx-empty__acts">
        <button
          type="button"
          class="mx-btn mx-btn--sm"
          onclick={() => void search.runSearch(search.resultsQuery)}
          ><IconRetry width="13" height="13" aria-hidden="true" />Retry</button
        >
      </div>
    </div>
  {:else if states.emptyState === "none"}
    {@const text = search.naturalParse.residual.trim()}
    <div class="rc-none">
      <p>
        Nothing {#if text}matches <b>“{text}”</b>{:else}was captured{/if}{#if search.effectivePills.length}{" "}in
          {#each search.effectivePills as pill, i (pill.kind)}{i ? " · " : ""}<b>{pillLabel(pill)}</b>{/each}{/if}.{rows.length
          ? " Closest ways out:"
          : ""}
      </p>
    </div>
    {@render noticeLine(true)}
    <div role="listbox" aria-label="Ways out">
      {#each rows as { way, n }, i (way.label)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="rc-row mx-reveal"
          role="option"
          tabindex="-1"
          aria-selected={selected === i}
          style:--i={i}
          onmousemove={() => (selected = i)}
          onclick={() => apply(i)}
        >
          {#if way.kind === "ask"}<IconSparkle width="14" height="14" aria-hidden="true" />
          {:else}<IconArrow width="14" height="14" aria-hidden="true" />{/if}
          <span class="rc-row__t">{way.label}</span>
          <time>{n === null ? "⇥" : `${n} moment${n === 1 ? "" : "s"}`}</time>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .mx-empty--center {
    padding: var(--s-5) var(--s-4);
  }
  .rc-note {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 30px;
    padding: 2px 10px 6px;
  }
  .rc-note .mx-inline {
    font-size: var(--text-sm);
  }
  .rc-none {
    padding: 14px 12px 4px;
  }
  .rc-none p {
    margin: 0;
    font: 400 14px / 1.5 var(--font-sans);
    color: var(--app-text-muted);
  }
  .rc-none b {
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .rc-row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--r-md);
    cursor: default;
    color: var(--app-text-subtle);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .rc-row:hover {
    background: var(--mx-wash);
  }
  .rc-row[aria-selected="true"] {
    background: var(--mx-selected);
  }
  .rc-row__t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text-strong);
  }
  time {
    min-width: 64px;
    text-align: right;
    font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .rc-row[aria-selected="true"] time {
    color: var(--app-text);
  }
</style>
