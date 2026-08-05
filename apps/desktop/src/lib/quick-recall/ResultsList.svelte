<!-- Quick Look results region (redesign slice 7, frames 08 + 10). The media
     surface: naked 349×196 cells in a 3-up grid, temporal day sections as
     --t-label lines with spacing (no fills, no rules), screen and audio in ONE
     grid. Also owns the full-width states: empty orientation, first-search
     skeleton, error, results-paused, and no-match (frame 10). DOM focus stays
     on the page's search input; this listbox is driven via
     aria-activedescendant.

     Grid arithmetic (fixed 1120×720 window, §3 density-rule exemption): 3×349
     cells + 2×16 gutters = 1079; insets 20 left + 20 right = 1119 ≤ 1120 (the
     1px slack rides the right inset; the 8px overlay scrollbar paints inside
     it). Inset 20 > gutter 16 is the one sanctioned exemption. -->
<script lang="ts">
  import ResultCell from "$lib/quick-recall/ResultCell.svelte";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
  } from "$lib/quick-recall/searchStore.svelte";

  let {
    askAvailable,
    onAskAi,
  }: {
    askAvailable: boolean;
    onAskAi: () => void;
  } = $props();

  // Frame 10's no-match widening: name the narrowing scope and offer the one
  // useful widening (drop the date scope).
  let dateScopeLabel = $derived(
    search.dateScope === "today"
      ? "Today"
      : search.dateScope === "week"
        ? "This week"
        : search.dateScope === "custom"
          ? "a date range"
          : null,
  );

  function thumbnailFor(index: number): string | null {
    const item = search.gridItems[index];
    if (item === undefined || item.kind !== "frame") return null;
    return search.thumbnailCache.get(item.frame.thumbnailFrameId) ?? null;
  }
</script>

<!-- The keyword-only hint, shared by the no-match and results branches. -->
{#snippet semanticHint()}
  {#if search.showSemanticSearchHint}
    <button
      type="button"
      class="ql-hint"
      onclick={() => void search.openSemanticSearchSettings()}
    >
      Searching keywords only. Turn on meaning-based search in Settings →
      Processing to also find results by meaning.
    </button>
  {/if}
{/snippet}

<div
  id="quick-recall-results-list"
  class="ql-body"
  class:ql-body--refetching={search.loading && search.hasResults}
  role="listbox"
  aria-label="Search results"
  aria-busy={search.loading}
>
  {#if search.belowMinimum}
    <!-- Empty — orientation (frame 10): the two verbs this window knows. -->
    <div class="ql-state">
      <p class="ql-state__read">
        Type to search your screen, mic and system audio.
        {#if askAvailable}<span class="is-mono">⌃⏎</span> asks a question instead.{/if}
      </p>
      <p class="ql-state__meta">⏎ opens a moment · esc closes</p>
    </div>
  {:else if search.loading && !search.hasResults}
    <!-- First-search skeleton: one shimmer row of grid cells, so the settle
         into real cells doesn't jump. Refetches keep prior results dimmed
         (the --refetching class) instead. -->
    <div class="qgrid qgrid--skeleton" aria-hidden="true">
      {#each [0, 1, 2] as i (i)}
        <div class="ql-sk-cell">
          <div class="ql-sk ql-sk__f"></div>
          <div class="ql-sk ql-sk__l1"></div>
          <div class="ql-sk ql-sk__l2"></div>
        </div>
      {/each}
    </div>
  {:else if search.errorMessage}
    <!-- Error (frame 10): honest about the index, the field stays usable.
         The Retry rides inline until the toast system (slice 8) lands. -->
    <div class="ql-state">
      <p class="ql-state__title">Search didn't run</p>
      <p class="ql-state__meta">{search.errorMessage}</p>
      <div class="ql-state__actions">
        <button
          type="button"
          class="btn btn--primary"
          onclick={() => void search.runSearch(search.resultsQuery)}
        >
          Retry
        </button>
      </div>
    </div>
  {:else if search.resultsPaused}
    <!-- The backend suppressed results for a malformed filter: neither stale
         cells nor a bare "no matches". Points back at the inline error. -->
    <div class="ql-state">
      <p class="ql-state__title">Results paused</p>
      <p class="ql-state__meta">Fix the filter above to search.</p>
    </div>
  {:else if search.showEmpty}
    <!-- No match (frame 10): names the query and offers the one useful
         widening when a date scope is narrowing it. -->
    <div class="ql-state">
      <p class="ql-state__title">
        No matches for “{search.residualQuery.trim() || search.resultsQuery}”{search.dateScope === "today"
          ? " today"
          : search.dateScope === "week"
            ? " this week"
            : ""}
      </p>
      <p class="ql-state__meta">
        {#if dateScopeLabel !== null}
          Scope is set to {dateScopeLabel}.
        {:else}
          Nothing captured matches all terms{search.activeFilterChips.length > 0
            ? " and filters"
            : ""}.
        {/if}
      </p>
      <div class="ql-state__actions">
        {#if dateScopeLabel !== null}
          <button
            type="button"
            class="btn btn--primary"
            onclick={() => search.applyDateScope("any")}
          >
            Search All Time
          </button>
        {/if}
        {#if askAvailable}
          <button type="button" class="btn" onclick={onAskAi}>
            Ask AI instead <span class="kbd">⌃⏎</span>
          </button>
        {/if}
      </div>
      {@render semanticHint()}
    </div>
  {:else}
    {@render semanticHint()}
    {#each search.gridSections as section (section.label)}
      <div class="qsec">
        <span class="qsec__label">{section.label}</span>
        <span class="qsec__count"
          >{section.count} {section.count === 1 ? "result" : "results"}</span
        >
      </div>
      <div class="qgrid" role="presentation">
        {#each search.gridItems.slice(section.start, section.start + section.count) as item, i (item.kind === "frame" ? `f-${item.frame.groupKey}` : `a-${item.audio.groupKey}`)}
          {@const index = section.start + i}
          <ResultCell
            {item}
            thumbnailUrl={thumbnailFor(index)}
            id={`${OPTION_ID_PREFIX}${index}`}
            selected={search.selectedIndex === index}
            onopen={() => search.openResultAt(index)}
          />
        {/each}
      </div>
    {/each}
  {/if}
</div>

<style>
  /* The scrollable media surface. Inset arithmetic in the header comment. */
  .ql-body {
    flex: 1 1 auto;
    min-height: 0;
    min-width: 0;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
    padding: var(--s-4) 20px 20px;
  }

  /* Overlay-style scrollbar painting inside the right inset. */
  .ql-body::-webkit-scrollbar {
    width: 8px;
  }

  .ql-body::-webkit-scrollbar-thumb {
    background: color-mix(in srgb, var(--app-text-subtle) 45%, transparent);
    border-radius: 4px;
    border: 2px solid transparent;
    background-clip: content-box;
  }

  .ql-body::-webkit-scrollbar-track {
    background: transparent;
  }

  .ql-body--refetching {
    opacity: 0.55;
    transition: opacity 0.12s ease;
  }

  /* Temporal section line: a --t-label mono line with a right-aligned count,
     spacing only — no fills, no rules (the group idiom stays out of the media
     surface). */
  .qsec {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin: var(--gap-section) 0 var(--s-12);
  }

  .qsec:first-child {
    margin-top: var(--s-8);
  }

  .qsec__label {
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  .qsec__count {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }

  /* 3-up fixed-cell grid: 3×349 + 2×16 = 1079 wide, row-aligned (no masonry);
     row gap 22 leaves the caption air below each cell. */
  .qgrid {
    display: grid;
    grid-template-columns: repeat(3, 349px);
    gap: 22px 16px;
  }

  /* ── Full-width states (frame 10) ─────────────────────────────────────── */
  .ql-state {
    min-height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-12);
    text-align: center;
    padding: var(--s-16) 40px;
  }

  .ql-state__read {
    margin: 0;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text-muted);
  }

  .ql-state__read .is-mono {
    font-family: var(--app-font-mono);
  }

  .ql-state__title {
    margin: 0;
    font: var(--w-semi) var(--t-title) / var(--lh-title) var(--app-font-sans);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }

  .ql-state__meta {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .ql-state__actions {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-top: var(--s-4);
  }

  /* In-search discoverability hint (keyword-only → Settings). */
  .ql-hint {
    display: block;
    width: 100%;
    margin: var(--s-8) 0 0;
    padding: var(--s-8) var(--s-12);
    text-align: left;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
    background: var(--app-surface-raised);
    border: var(--hairline) solid var(--app-border);
    border-radius: var(--r-md);
    cursor: pointer;
  }

  .ql-state .ql-hint {
    max-width: 420px;
    text-align: center;
  }

  .ql-hint:hover {
    color: var(--app-text);
    border-color: var(--app-accent);
  }

  /* ── First-search skeleton cells ──────────────────────────────────────── */
  .qgrid--skeleton {
    margin-top: var(--s-8);
  }

  .ql-sk-cell {
    width: 349px;
  }

  .ql-sk {
    border-radius: var(--r-md);
    background: linear-gradient(
      90deg,
      var(--app-surface-hover) 25%,
      var(--app-surface-raised) 50%,
      var(--app-surface-hover) 75%
    );
    background-size: 400px 100%;
    animation: ql-shimmer 1.4s linear infinite;
  }

  .ql-sk__f {
    width: 349px;
    height: 196px;
  }

  .ql-sk__l1 {
    width: 70%;
    height: 10px;
    margin-top: var(--s-8);
    border-radius: 4px;
  }

  .ql-sk__l2 {
    width: 45%;
    height: 9px;
    margin-top: var(--s-6);
    border-radius: 4px;
  }

  @keyframes ql-shimmer {
    from {
      background-position: -200px 0;
    }
    to {
      background-position: 200px 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ql-sk {
      animation: none;
    }

    .ql-body--refetching {
      transition: none;
    }
  }
</style>
