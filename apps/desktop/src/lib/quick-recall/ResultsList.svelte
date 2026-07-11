<!-- Quick Recall search-mode results region (extracted from the quick-recall
     +page.svelte in the search-mode extraction slice — no behavior/visual
     change). Renders every results-region state branch off the search store
     singleton: orientation (below-minimum), loading skeleton, error + Retry,
     results-paused parse error, no-matches recovery, the semantic hint, and
     the Screen/Audio result sections. DOM focus stays on the page's search
     input; this listbox is driven via aria-activedescendant. -->
<script lang="ts">
  import SearchResultCard from "$lib/quick-recall/SearchResultCard.svelte";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
  } from "$lib/quick-recall/searchStore.svelte";
  import {
    AUDIO_VISIBLE_CAP,
    FRAME_VISIBLE_CAP,
    moreRowLabel,
  } from "$lib/quick-recall/result-sections";

  let {
    askAvailable,
    onAskAi,
    split = false,
  }: {
    askAvailable: boolean;
    onAskAi: () => void;
    // True when the page shows the list/detail two-pane split (skeleton or
    // results branches): the list becomes the fixed-width left column with
    // the mockup's divider + subtle background. False for the full-width
    // states (orientation, error, paused, no matches).
    split?: boolean;
  } = $props();
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
  class:quick-recall__results--split={split}
  role="listbox"
  aria-label="Search results"
  aria-busy={search.loading}
>
  {#if search.belowMinimum}
    <!-- Feature-teaching orientation view for the pristine / short-query
         state (mockup state A): centered glyph / lead / sub / faint stack.
         No clickable canned queries — calm cues only. -->
    <div class="quick-recall__state-center">
      <span class="quick-recall__state-glyph" aria-hidden="true">⌕</span>
      <p class="quick-recall__state-lead">Search everything you've captured</p>
      <p class="quick-recall__state-sub">Screen · Audio · Ask AI</p>
      <p class="quick-recall__state-faint">
        Type to find a moment{askAvailable ? ", or press " : "."}{#if askAvailable}<kbd
            >⌃↵</kbd
          > to ask AI.{/if}
      </p>
    </div>
  {:else if search.loading && !search.hasResults}
    <!-- Skeleton rows mirroring the redesigned row anatomy (mockup state B):
         a section-label bar, then rows with the 150×94 thumb block and two
         shimmer lines. Only the FIRST search (no prior results) shows the
         full skeleton; a refetch on a subsequent keystroke keeps the prior
         results visible-but-dimmed (the `--refetching` class on the list) so
         the surface doesn't flash empty between keystrokes. The page dims the
         detail pane alongside (DetailPane's `dim` prop). -->
    <div class="quick-recall__skeletons" aria-hidden="true">
      <div class="quick-recall__sk quick-recall__sk-label"></div>
      {#each [
        [70, 92],
        [58, 84],
        [76, 64],
      ] as widths, i (i)}
        <div class="quick-recall__skeleton-row">
          <div class="quick-recall__sk quick-recall__skeleton-thumb"></div>
          <div class="quick-recall__skeleton-body">
            <span
              class="quick-recall__sk quick-recall__skeleton-line"
              style={`width:${widths[0]}%`}
            ></span>
            <span
              class="quick-recall__sk quick-recall__skeleton-line"
              style={`width:${widths[1]}%`}
            ></span>
          </div>
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
    <!-- Rows render the VISIBLE slices only; the flattened selection index
         space is visible frames first, then visible audio. Clicking a row
         SELECTS it (previews in the detail pane) — Enter is the open action.
         The show-more row (mockup `.more-row`) reveals the already-fetched
         remainder client-side; per the mockup it is click-only and NOT part
         of the arrow-key roving order. -->
    {#if search.frames.length > 0}
      <div class="quick-recall__section" role="presentation">
        <span class="quick-recall__section-label"
          >Screen<span class="quick-recall__section-count"
            >{search.frames.length}</span
          ></span
        >
        <div class="quick-recall__list" role="presentation">
          {#each search.visibleFrames as result, i (result.groupKey)}
            <SearchResultCard
              kind="frame"
              frame={result}
              thumbnailUrl={search.thumbnailCache.get(result.thumbnailFrameId) ??
                null}
              id={`${OPTION_ID_PREFIX}${i}`}
              selected={search.selectedIndex === i}
              onselect={() => search.selectResultAt(i)}
            />
          {/each}
        </div>
        {#if moreRowLabel(search.frames.length, FRAME_VISIBLE_CAP, search.framesExpanded, "screen") !== null}
          <button
            type="button"
            class="quick-recall__more-row"
            tabindex="-1"
            onclick={() => search.toggleFramesExpanded()}
          >
            {moreRowLabel(
              search.frames.length,
              FRAME_VISIBLE_CAP,
              search.framesExpanded,
              "screen",
            )}
          </button>
        {/if}
      </div>
    {/if}

    {#if search.audio.length > 0}
      <div class="quick-recall__section" role="presentation">
        <span class="quick-recall__section-label"
          >Audio<span class="quick-recall__section-count"
            >{search.audio.length}</span
          ></span
        >
        <div class="quick-recall__list" role="presentation">
          {#each search.visibleAudio as result, i (result.groupKey)}
            <SearchResultCard
              kind="audio"
              audio={result}
              id={`${OPTION_ID_PREFIX}${search.visibleFrames.length + i}`}
              selected={search.selectedIndex === search.visibleFrames.length + i}
              onselect={() => search.selectResultAt(search.visibleFrames.length + i)}
            />
          {/each}
        </div>
        {#if moreRowLabel(search.audio.length, AUDIO_VISIBLE_CAP, search.audioExpanded, "audio") !== null}
          <button
            type="button"
            class="quick-recall__more-row"
            tabindex="-1"
            onclick={() => search.toggleAudioExpanded()}
          >
            {moreRowLabel(
              search.audio.length,
              AUDIO_VISIBLE_CAP,
              search.audioExpanded,
              "audio",
            )}
          </button>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .quick-recall__results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  /* Left column of the list/detail split (mockup `.result-list`): fixed
     ~700px at the 1120 design width, hairline divider against the detail
     pane, subtle background so the pane split reads. */
  .quick-recall__results--split {
    flex: 0 0 700px;
    min-width: 0;
    border-right: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }

  /* Refetch-in-flight: prior results stay on screen but dim slightly so the
     keystroke-driven refresh reads as "updating" without the surface flashing
     empty between every keystroke. */
  .quick-recall__results--refetching {
    opacity: 0.55;
    transition: opacity 0.12s ease;
  }

  .quick-recall__section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* Section header (mockup `.section-label`): uppercase modality label left,
     plain result count right on the same baseline. */
  .quick-recall__section-label {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: var(--text-sm);
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    padding: 0 2px;
  }

  .quick-recall__section-count {
    text-transform: none;
    letter-spacing: 0;
    color: var(--app-text-subtle);
  }

  .quick-recall__list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* Section show-more/show-less toggle (mockup `.more-row`): a quiet full-
     width centered text row that reveals the already-fetched overflow. */
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
     faint stack with an optional actions row, used by orientation, error,
     results-paused, and no-matches. */
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

  /* Loading skeleton (mockup state B / `.sk`): a section-label bar plus rows
     mirroring the redesigned row anatomy (150×94 thumb + two lines) so the
     transition from skeleton to real cards doesn't jump. The shimmer is the
     mockup's sweeping background-position gradient. */
  .quick-recall__skeletons {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .quick-recall__sk {
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      var(--app-surface-hover) 25%,
      var(--app-surface-raised) 50%,
      var(--app-surface-hover) 75%
    );
    background-size: 400px 100%;
    animation: quick-recall-shimmer 1.4s linear infinite;
  }

  .quick-recall__sk-label {
    height: 9px;
    width: 56px;
    margin: 10px 8px 6px;
  }

  .quick-recall__skeleton-row {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 8px 12px;
    border-radius: 9px;
  }

  .quick-recall__skeleton-thumb {
    flex-shrink: 0;
    width: 150px;
    height: 94px;
    border-radius: 6px;
  }

  .quick-recall__skeleton-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .quick-recall__skeleton-line {
    display: block;
    height: 9px;
  }

  @keyframes quick-recall-shimmer {
    from {
      background-position: -200px 0;
    }
    to {
      background-position: 200px 0;
    }
  }

  /* Reduced-motion gating for this region's animations/transitions (the rest
     of the surface is gated in the page / sibling components). */
  @media (prefers-reduced-motion: reduce) {
    .quick-recall__sk {
      animation: none;
    }

    .quick-recall__state-btn,
    .quick-recall__more-row,
    .quick-recall__results--refetching {
      transition: none;
    }
  }
</style>
