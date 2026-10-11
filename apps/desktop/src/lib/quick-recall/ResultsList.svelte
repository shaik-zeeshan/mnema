<!-- Quick Recall search-mode results region. Renders every results-region
     state branch off the search store singleton: loading skeleton, error +
     Retry, results-paused, no-matches recovery, the semantic hint, and the one
     ranked list — the best moment, then one-line rows (frames and audio merged
     by rank). DOM focus stays on the page's search input; this listbox is
     driven via aria-activedescendant (option ids `${OPTION_ID_PREFIX}${i}`,
     i = index into `search.visibleResults`). -->
<script lang="ts">
  import BestMoment from "$lib/quick-recall/BestMoment.svelte";
  import SearchResultCard from "$lib/quick-recall/SearchResultCard.svelte";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
  } from "$lib/quick-recall/searchStore.svelte";
  import { moreRowLabel } from "$lib/quick-recall/result-sections";
  import { parseCapturedAt } from "$lib/format-time";

  let {
    askAvailable,
    onAskAi,
    onlook,
  }: {
    askAvailable: boolean;
    onAskAi: () => void;
    // Quick Look on the visible result at `index` (QR4 wires it).
    onlook?: (index: number) => void;
  } = $props();

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

<!-- The keyword-only hint, guarded by showSemanticSearchHint, shared by the
     empty and results branches so there is one source of truth. -->
{#snippet semanticHint()}
  {#if search.showSemanticSearchHint}
    <button
      type="button"
      class="quick-recall__semantic-hint"
      onclick={() => void search.openSemanticSearchSettings()}
    >
      Searching keywords only. Turn on meaning-based search in Settings →
      Processing to also find results by meaning.
    </button>
  {/if}
{/snippet}

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
  {:else if search.errorMessage}
    <!-- A backend search failure (mockup state C): centered danger glyph +
         lead + danger detail, with an explicit recovery (re-issue the same
         query) mirroring the Ask AI "Retry" so the path isn't a soft dead end
         the user has to guess at by editing the query. -->
    <div class="quick-recall__state-center">
      <span
        class="quick-recall__state-glyph quick-recall__state-glyph--danger"
        aria-hidden="true">⚠</span
      >
      <p class="quick-recall__state-lead">Search failed</p>
      <p class="quick-recall__state-sub quick-recall__state-sub--danger">
        {search.errorMessage}
      </p>
      <div class="quick-recall__state-actions">
        <button
          type="button"
          class="quick-recall__state-btn quick-recall__state-btn--accent"
          onclick={() => void search.runSearch(search.resultsQuery)}
        >
          Retry
        </button>
      </div>
    </div>
  {:else if search.resultsPaused}
    <!-- Paused-results state. The backend suppressed results for a malformed
         filter, so we render neither stale cards nor the bare "No matches"
         empty state. No mockup panel exists for this branch — it borrows the
         error/no-match centered pattern (warn-tinted glyph: user-fixable, not
         a failure) pointing back at the inline error above. This branch
         precedes showEmpty / the normal results branch so a parse error
         always wins here. Ask AI stays reachable, so the question path is
         open even while search results are paused. -->
    <div class="quick-recall__state-center">
      <span
        class="quick-recall__state-glyph quick-recall__state-glyph--warn"
        aria-hidden="true">⚠</span
      >
      <p class="quick-recall__state-lead">Results paused</p>
      <p class="quick-recall__state-sub">Fix the filter above to search.</p>
    </div>
  {:else if search.showEmpty}
    <!-- No-matches recovery (mockup state D): centered ⌀ + lead + sub, then
         the recovery paths — loosening a filter when chips narrow the query,
         and the accent Ask AI pivot so the empty result isn't a dead end. -->
    <div class="quick-recall__state-center">
      <span class="quick-recall__state-glyph" aria-hidden="true">⌀</span>
      <p class="quick-recall__state-lead">
        No matches for “{search.resultsQuery}”
      </p>
      <p class="quick-recall__state-sub">
        Nothing captured matches all terms{search.activeFilterChips.length > 0
          ? " and filters"
          : ""}.
      </p>
      {#if search.activeFilterChips.length > 0}
        <p class="quick-recall__state-faint">try removing a filter</p>
      {/if}
      {#if askAvailable}
        <div class="quick-recall__state-actions">
          <button
            type="button"
            class="quick-recall__state-btn quick-recall__state-btn--accent"
            onclick={onAskAi}
          >
            Ask AI instead <kbd>⌃↵</kbd>
          </button>
        </div>
      {/if}
      {@render semanticHint()}
    </div>
  {:else}
    {@render semanticHint()}
    <!-- Clicking a row SELECTS it; Enter is the open action. The show-more
         row reveals the already-fetched remainder; it is click-only and NOT
         part of the arrow-key order. -->
    {#if best}
      <BestMoment
        result={best}
        thumbnailUrl={best.kind === "frame"
          ? (search.thumbnailCache.get(best.frame.thumbnailFrameId) ?? null)
          : null}
        id={`${OPTION_ID_PREFIX}0`}
        selected={search.selectedIndex === 0}
        onselect={() => search.selectResultAt(0)}
        onlook={onlook && (() => onlook(0))}
      />
    {/if}
    {#if search.visibleResults.length > 1}
      <div class="quick-recall__sec" role="presentation">
        More moments<span class="mono">{search.totalResultCount - 1}</span><span
          class="mono">{breakdown}</span
        >
      </div>
      {#each search.visibleResults.slice(1) as result, k (result.kind === "frame" ? result.frame.groupKey : result.audio.groupKey)}
        <SearchResultCard
          {result}
          id={`${OPTION_ID_PREFIX}${k + 1}`}
          selected={search.selectedIndex === k + 1}
          onselect={() => search.selectResultAt(k + 1)}
          onlook={onlook && (() => onlook(k + 1))}
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

  /* Snippet matches, kit `mark` (the app kit has no global rule yet). */
  .quick-recall__results :global(mark) {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-radius: 2px;
    padding: 0 2px;
    box-shadow: inset 0 -1px 0 var(--app-accent-border);
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

  /* Shared centered state pattern (mockup `.sp-center`): glyph / lead / sub /
     faint stack with an optional actions row, used by error, results-paused,
     and no-matches. */
  .quick-recall__state-center {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    text-align: center;
    padding: 8px 40px 18px;
  }

  .quick-recall__state-glyph {
    font-size: var(--text-xl);
    line-height: 1;
    color: var(--app-text-subtle);
  }

  .quick-recall__state-glyph--danger {
    color: var(--app-danger-text);
  }

  .quick-recall__state-glyph--warn {
    color: var(--app-warn);
  }

  .quick-recall__state-lead {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.4;
    color: var(--app-text-strong);
  }

  .quick-recall__state-sub {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .quick-recall__state-sub--danger {
    color: var(--app-danger-text);
  }

  .quick-recall__state-faint {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  .quick-recall__state-center kbd {
    font-family: inherit;
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 2px 5px;
    margin: 0 1px;
  }

  .quick-recall__state-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }

  /* Mockup `.sp-btn` / `.sp-btn.accent`: the accent variant carries the
     recovery CTAs (Retry, Ask AI instead) in the mockup's Ask-AI-door idiom. */
  .quick-recall__state-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    padding: 6px 11px;
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      color 0.12s ease,
      box-shadow 0.12s ease;
  }

  .quick-recall__state-btn:hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }

  .quick-recall__state-btn:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__state-btn:active {
    background: var(--app-surface-active);
  }

  .quick-recall__state-btn--accent {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  .quick-recall__state-btn--accent:hover {
    color: var(--app-accent);
    border-color: var(--app-accent-strong);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .quick-recall__state-btn--accent:active {
    background: color-mix(in srgb, var(--app-accent) 14%, var(--app-accent-bg));
  }

  .quick-recall__state-btn--accent kbd {
    color: var(--app-accent);
    background: transparent;
    border-color: var(--app-accent-border);
  }

  /* In-search discoverability hint (issue #125): keyword-only search → Settings. */
  .quick-recall__semantic-hint {
    display: block;
    width: 100%;
    margin: 4px 0 8px;
    padding: 8px 10px;
    text-align: left;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 7px;
    cursor: pointer;
  }

  /* Inside the centered no-matches state the hint is a bounded card below the
     actions rather than a full-width band. */
  .quick-recall__state-center .quick-recall__semantic-hint {
    max-width: 420px;
    margin: 8px 0 0;
  }

  .quick-recall__semantic-hint:hover {
    color: var(--app-text);
    border-color: var(--app-accent);
  }

  .quick-recall__semantic-hint:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__semantic-hint:active {
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
    .quick-recall__state-btn,
    .quick-recall__more-row,
    .quick-recall__results--refetching {
      transition: none;
    }
  }
</style>
