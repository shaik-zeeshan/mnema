<!-- Quick Recall search-mode results region. Renders every results-region state
     branch off the search store singleton: the "Ask AI about ⟨query⟩" ranked row
     (G4 — rank 0 of this same list), orientation (below-minimum), loading
     skeleton, error + Retry, results-paused parse error, no-matches, the
     semantic hint, and the Screen/Audio result sections as the round-4 3-up
     349×196 tile grid. DOM focus stays on the page's search input; this listbox
     is driven via aria-activedescendant. -->
<script lang="ts">
  import SearchResultCard from "$lib/quick-recall/SearchResultCard.svelte";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
    ASK_ROW_OPTION_ID,
  } from "$lib/quick-recall/searchStore.svelte";
  import {
    AUDIO_VISIBLE_CAP,
    FRAME_VISIBLE_CAP,
    moreRowLabel,
  } from "$lib/quick-recall/result-sections";

  let {
    askAvailable,
    onAskAi,
  }: {
    askAvailable: boolean;
    onAskAi: () => void;
  } = $props();

  // The ask row's subordinate line: what taking it would actually cost/do. It
  // is the one place the search↔ask escalation is priced, so it names the real
  // match count rather than a generic invitation.
  const askRowHint = $derived.by(() => {
    if (search.loading && !search.hasResults) return "searches everything you have captured";
    const matches = search.frames.length + search.audio.length;
    if (matches > 0) {
      return `reads the ${matches} ${matches === 1 ? "match" : "matches"} and answers in one paragraph`;
    }
    return "nothing matched — ask across everything you have captured";
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
  <!-- G4: ask is a RANKED ROW in this list, never a competing mode control.
       It sits at rank 0 (selection index -1) above every state branch, so it is
       reachable while results load, while a filter parse error pauses results,
       and after a backend error. A zero-result search leaves the selection at
       -1, which IS this row — that is the promotion, no special case. -->
  {#if search.askRowVisible}
    <button
      type="button"
      id={ASK_ROW_OPTION_ID}
      class="quick-recall__ask-row"
      class:quick-recall__ask-row--selected={search.askRowSelected}
      role="option"
      aria-selected={search.askRowSelected}
      tabindex="-1"
      onclick={onAskAi}
    >
      <span class="quick-recall__ask-row-glyph" aria-hidden="true">✦</span>
      <span class="quick-recall__ask-row-label"
        >Ask AI about “{search.trimmedQuery}”</span
      >
      <span class="quick-recall__ask-row-hint">{askRowHint}</span>
      <span class="kbd quick-recall__ask-row-key" aria-hidden="true">⌃↵</span>
    </button>
  {/if}

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
    <!-- Skeleton tiles mirroring the 3-up grid anatomy so the swap to real
         tiles doesn't jump. Only the FIRST search (no prior results) shows the
         skeleton; a refetch on a subsequent keystroke keeps the prior results
         visible-but-dimmed (the `--refetching` class on the list) so the
         surface doesn't flash empty between keystrokes. -->
    <div class="quick-recall__section" aria-hidden="true">
      <div class="quick-recall__sk quick-recall__sk-label"></div>
      <div class="quick-recall__grid">
        {#each [70, 58, 76] as width, i (i)}
          <div class="quick-recall__skeleton-tile">
            <span
              class="quick-recall__sk quick-recall__skeleton-line"
              style={`width:${width}%`}
            ></span>
            <div class="quick-recall__sk quick-recall__skeleton-media"></div>
          </div>
        {/each}
      </div>
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
    <!-- No-matches recovery: centered ⌀ + lead + sub. The Ask AI pivot that
         used to live here as a button is GONE — the ranked ask row above is
         already selected in this state (selection index -1), so duplicating it
         as a second control would be the competing mode affordance G4 kills. -->
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
      {:else if !askAvailable}
        <p class="quick-recall__state-faint">try fewer or broader words</p>
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
        <div class="quick-recall__grid" role="presentation">
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
        <div class="quick-recall__grid" role="presentation">
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
    padding: var(--s-12) var(--grid-inset);
    display: flex;
    flex-direction: column;
    gap: var(--gap-group);
  }

  /* G4's ranked ask row: rank 0 of this list, a full-width row so it reads as
     the list's first entry rather than a second control. Accent-tinted because
     it is the one escalation on the surface that costs a model call; the ring
     only appears when the roving selection is actually on it. */
  .quick-recall__ask-row {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    width: 100%;
    min-width: 0;
    padding: var(--s-8) var(--s-12);
    text-align: left;
    font: inherit;
    color: var(--app-accent-strong);
    background: var(--app-accent-bg);
    border: var(--hairline) solid var(--app-accent-border);
    border-radius: var(--r-md);
    cursor: pointer;
  }

  .quick-recall__ask-row:hover {
    border-color: var(--app-accent);
  }

  .quick-recall__ask-row--selected,
  .quick-recall__ask-row:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .quick-recall__ask-row-glyph {
    flex: none;
    font-size: var(--t-ui);
    line-height: 1;
    color: var(--app-accent);
  }

  .quick-recall__ask-row-label {
    flex: none;
    max-width: 55%;
    font-size: var(--t-ui);
    line-height: 1.3;
    font-weight: var(--w-medium);
    color: var(--app-accent-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__ask-row-hint {
    flex: 1;
    min-width: 0;
    font-size: var(--t-meta);
    line-height: 1.3;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__ask-row-key {
    flex: none;
    color: var(--app-accent);
    background: transparent;
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
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
    font-size: var(--t-meta);
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

  /* Round-4 result grid: 3-up tiles on the Overview's cell unit (349 wide,
     16px gutter). The tracks cap at 349px and shrink evenly below it, so the
     window's 960px minimum width still shows three columns instead of
     overflowing or dropping to two. */
  .quick-recall__grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 349px));
    gap: var(--grid-gutter);
  }

  /* Section show-more/show-less toggle (mockup `.more-row`): a quiet full-
     width centered text row that reveals the already-fetched overflow. */
  .quick-recall__more-row {
    display: block;
    width: 100%;
    text-align: center;
    font: inherit;
    font-size: var(--t-meta);
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
    font-size: var(--t-display);
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
    font-size: var(--t-ui);
    line-height: 1.4;
    color: var(--app-text-strong);
  }

  .quick-recall__state-sub {
    margin: 0;
    font-size: var(--t-meta);
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .quick-recall__state-sub--danger {
    color: var(--app-danger-text);
  }

  .quick-recall__state-faint {
    margin: 0;
    font-size: var(--t-label);
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  .quick-recall__state-center kbd {
    font-family: inherit;
    font-size: var(--t-label);
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
    font-size: var(--t-meta);
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

  /* In-search discoverability hint (issue #125): keyword-only search → Settings. */
  .quick-recall__semantic-hint {
    display: block;
    width: 100%;
    margin: 4px 0 8px;
    padding: 8px 10px;
    text-align: left;
    font-size: var(--t-ui);
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
  }

  /* One skeleton tile per grid cell: a caption line over the 196px media block,
     the same shape a real tile settles into. */
  .quick-recall__skeleton-tile {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    padding: var(--s-12);
    border: var(--hairline) solid var(--app-border);
    border-radius: var(--r-lg);
    background: var(--app-surface-subtle);
  }

  .quick-recall__skeleton-media {
    height: 196px;
    border-radius: var(--r-md);
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
