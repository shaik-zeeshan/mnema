<!-- Quick Recall search-mode results region, off the search store singleton:
     the first-search skeleton, then the one ranked list — the best moment,
     then one-line rows (frames and audio merged by rank). DOM focus stays on
     the page's search input; this listbox is driven via aria-activedescendant
     (option ids `${OPTION_ID_PREFIX}${i}`, i = index into
     `search.visibleResults`). Empty, failed and paused searches never mount
     it (EmptyStates / the page's paused notice). -->
<script lang="ts">
  import BestMoment from "$lib/quick-recall/BestMoment.svelte";
  import SearchResultCard from "$lib/quick-recall/SearchResultCard.svelte";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
  } from "$lib/quick-recall/searchStore.svelte";
  import { quickLook } from "$lib/quick-recall/detailStore.svelte";
  import { moreRowLabel } from "$lib/quick-recall/result-sections";
  import { parseCapturedAt } from "$lib/format-time";

  let {
    onlook,
    rowsOnly = false,
  }: {
    // Quick Look on the visible result at `index` (QR4 wires it).
    onlook?: (index: number) => void;
    // Under the ask slot's gate / notice: plain "Moments" rows, no best moment.
    rowsOnly?: boolean;
  } = $props();

  // A click selects like ↑/↓ does, so Space then opens Quick Look on it.
  const select = (i: number) => {
    search.selectResultAt(i);
    quickLook.nav = true;
  };

  const best = $derived(search.visibleResults[0]);
  const moreLabel = $derived(moreRowLabel(search.totalResultCount, search.expanded));
  // "N screen · N audio · N days" over everything fetched.
  const breakdown = $derived.by(() => {
    const days = new Set(
      search.ranked.map((r) =>
        parseCapturedAt(
          r.kind === "frame" ? r.frame.groupStartAt : r.audio.absoluteStartAt,
        ).toDateString(),
      ),
    ).size;
    return [
      search.frames.length > 0 && `${search.frames.length} screen`,
      search.audio.length > 0 && `${search.audio.length} audio`,
      `${days} day${days === 1 ? "" : "s"}`,
    ]
      .filter(Boolean)
      .join(" · ");
  });
</script>

<div
  id="quick-recall-results-list"
  class="quick-recall__results"
  class:quick-recall__results--refetching={search.loading && search.hasResults}
  role="listbox"
  aria-label="Search results"
  aria-busy={search.loading}
>
  <!-- No orientation branch: below the minimum query the page shows the
       bare idle bar and does not mount this list. -->
  {#if search.loading && !search.hasResults}
    <!-- First search only (mockup `loading`): the best-moment and row shapes,
         static kit skeletons. A refetch keeps the prior results, dimmed. -->
    <div class="quick-recall__skeletons" aria-hidden="true">
      <div class="quick-recall__skel-hero">
        <div class="mx-skel mx-skel--thumb"></div>
        <div class="mx-skel-lines">
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel" style="width:60%"></div>
          <div class="mx-skel"></div>
          <div class="mx-skel" style="width:40%"></div>
        </div>
      </div>
      {#each [0, 1, 2] as i (i)}
        <div class="mx-skel-row quick-recall__skel-row">
          <div class="mx-skel"></div>
          <div class="mx-skel-lines"><div class="mx-skel"></div></div>
          <div class="mx-skel"></div>
        </div>
      {/each}
    </div>
  {:else}
    <!-- Clicking a row SELECTS it; Enter is the open action. The show-more
         row reveals the already-fetched remainder; it is click-only and NOT
         part of the arrow-key order. -->
    {@const first = rowsOnly ? 0 : 1}
    {#if best && !rowsOnly}
      <BestMoment
        result={best}
        thumbnailUrl={best.kind === "frame"
          ? (search.thumbnailCache.get(best.frame.thumbnailFrameId) ?? null)
          : null}
        id={`${OPTION_ID_PREFIX}0`}
        selected={search.selectedIndex === 0}
        onselect={() => select(0)}
        onlook={onlook && (() => onlook(0))}
      />
    {/if}
    {#if search.visibleResults.length > first}
      <div class="quick-recall__sec" role="presentation">
        {#if rowsOnly}Moments<span class="mono">{search.totalResultCount}</span
          >{:else}More moments<span class="mono">{search.totalResultCount - 1}</span><span
            class="mono">{breakdown}</span
          >{/if}
      </div>
      {#each search.visibleResults.slice(first) as result, k (result.kind === "frame" ? result.frame.groupKey : result.audio.groupKey)}
        <SearchResultCard
          {result}
          id={`${OPTION_ID_PREFIX}${k + first}`}
          selected={search.selectedIndex === k + first}
          onselect={() => select(k + first)}
          onlook={onlook && (() => onlook(k + first))}
        />
      {/each}
    {/if}
    {#if moreLabel !== null}
      <button
        type="button"
        class="quick-recall__more-row"
        tabindex="-1"
        onclick={() => search.toggleExpanded()}
      >
        {moreLabel}
      </button>
    {/if}
  {/if}
</div>

<style>
  .quick-recall__results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  /* The list scrolls; its rows keep their height (34px rows, not squeezed). */
  .quick-recall__results > :global(*) {
    flex: none;
  }

  /* Refetch-in-flight: prior results stay on screen but dim slightly so the
     keystroke-driven refresh reads as "updating" without the surface flashing
     empty between every keystroke. */
  .quick-recall__results--refetching {
    opacity: 0.55;
    transition: opacity 0.12s ease;
  }

  /* Mockup `.rc-sec`: "More moments 7 … 10 screen · 1 audio · 4 days". */
  .quick-recall__sec {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 10px 10px 4px;
    font: 500 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }

  .quick-recall__sec .mono {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .quick-recall__sec .mono:last-child {
    margin-left: auto;
  }

  /* Show-more/show-less toggle: a quiet full-width centered text row that
     reveals the already-fetched overflow. */
  .quick-recall__more-row {
    display: block;
    width: 100%;
    text-align: center;
    font: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    color: var(--app-text-subtle);
    background: none;
    border: none;
    border-radius: 7px;
    padding: 8px 0;
    cursor: pointer;
    transition:
      color 0.12s,
      background 0.12s;
  }

  .quick-recall__more-row:hover {
    color: var(--app-accent);
    background: var(--app-surface-hover);
  }

  .quick-recall__more-row:active {
    background: var(--app-surface-active);
  }

  /* Loading skeleton (mockup `loading`, `.rc-skel-hero`): static kit
     skeletons shaped like the best moment and three rows. */
  .quick-recall__skel-hero {
    display: grid;
    grid-template-columns: 268px minmax(0, 1fr);
    gap: 18px;
    padding: 10px;
  }

  .quick-recall__skel-hero .mx-skel--thumb {
    height: 168px;
    aspect-ratio: auto;
  }

  .quick-recall__skel-hero .mx-skel-lines {
    align-content: start;
    padding-top: 4px;
  }

  .quick-recall__skel-row {
    padding: 6px 10px;
  }

  .quick-recall__skel-row > .mx-skel:first-child {
    width: 18px;
    height: 18px;
  }

  .quick-recall__skel-row > .mx-skel:last-child {
    width: 56px;
  }

  /* Reduced-motion gating for this region's animations/transitions (the rest
     of the surface is gated in the page / sibling components). */
  @media (prefers-reduced-motion: reduce) {
    .quick-recall__more-row,
    .quick-recall__results--refetching {
      transition: none;
    }
  }
</style>
