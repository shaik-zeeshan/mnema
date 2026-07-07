<!-- Quick Recall Filter Picker + Filter Value List (extracted from the
     quick-recall +page.svelte in the search-mode extraction slice — no
     behavior/visual change). The page renders this component whenever
     `filters.pickerOpen || filters.valueListActive`, preserving the original
     branch order (the picker wins while open; the two are mutually exclusive
     by construction — activeOperatorContext is null while pickerOpen). DOM
     focus stays on the page's search input; both listboxes are driven via
     aria-activedescendant, and the input's keydown / the window capture
     listener own Arrow/Enter/Escape while either surface is up. -->
<script lang="ts">
  import { quickRecallSearch as search } from "$lib/quick-recall/searchStore.svelte";
  import {
    PICKER_CATEGORIES,
    PICKER_OPT_PREFIX,
    VALUE_LIST_OPT_PREFIX,
  } from "$lib/quick-recall/filterSurfaces.svelte";
  import { appIcons } from "$lib/quick-recall/app-icons.svelte";

  const filters = search.filters;
</script>

{#if filters.pickerOpen}
  <!-- Filter Picker overlay. Replaces the results region while open. The
       input's keydown routes Arrow/Enter/Escape/Tab here through
       handlePickerKeydown. -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    bind:this={filters.pickerEl}
    id="quick-recall-picker"
    class="quick-recall__results quick-recall__picker"
    role="listbox"
    tabindex="-1"
    aria-label="Filter picker"
    onkeydown={(event) => {
      // The category door owns Arrow/Enter/Escape/Tab while open; route them
      // through the shared handler (also called from the input's keydown and
      // the window capture listener for focus resilience).
      if (filters.handlePickerKeydown(event)) {
        return;
      }
    }}
  >
    <!-- Category door header: the picker only lists categories; selecting one
         writes its operator stub and hands off to the value list. -->
    <div class="quick-recall__picker-header">
      <span class="quick-recall__picker-title">Filters</span>
      <span class="quick-recall__orient-cue-dot" aria-hidden="true">·</span>
      <span class="quick-recall__picker-crumb-hint">pick a category</span>
    </div>

    <div class="quick-recall__picker-list" role="presentation">
      {#each PICKER_CATEGORIES as category, i (category.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          id={`${PICKER_OPT_PREFIX}${i}`}
          class="quick-recall__picker-item"
          class:quick-recall__picker-item--selected={filters.pickerIndex === i}
          role="option"
          tabindex="-1"
          aria-selected={filters.pickerIndex === i}
          onmousemove={() => (filters.pickerIndex = i)}
          onclick={() => {
            filters.pickerIndex = i;
            filters.pickerSelectHighlighted();
          }}
        >
          <span class="quick-recall__picker-item-label">{category.label}</span>
          <span class="quick-recall__picker-item-hint">{category.hint}</span>
          <span class="quick-recall__picker-item-chevron" aria-hidden="true">›</span>
        </div>
      {/each}
    </div>

    <p class="quick-recall__picker-cue" aria-hidden="true">
      <kbd>↑</kbd><kbd>↓</kbd> move · <kbd>↵</kbd> select · <kbd>esc</kbd> close
    </p>
  </div>
{:else}
  <!-- The Filter Value List (typed path). Replaces the results region while
       the caret sits in an un-committed field-operator value. Reuses the
       picker's CSS so it reads as a drilled picker value list: a header naming
       the operator, the value rows (or an app-only empty line), an optional
       conflict note, a typed-date hint for date operators, and the shared
       move/select/back cue. -->
  <div
    id="quick-recall-value-list"
    class="quick-recall__results quick-recall__picker"
    role="listbox"
    aria-label="Filter values"
  >
    <div class="quick-recall__picker-header">
      <span class="quick-recall__picker-title">
        {filters.activeOperatorContext?.operator === "app:"
          ? "App"
          : filters.activeOperatorContext?.operator === "source:"
            ? "Source"
            : "Date range"}
      </span>
    </div>

    {#if filters.valueListEmptyMessage !== null}
      <p class="quick-recall__state">{filters.valueListEmptyMessage}</p>
    {:else}
      <div class="quick-recall__picker-list" role="presentation">
        {#each filters.valueListRows as row, i (row.id)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            id={`${VALUE_LIST_OPT_PREFIX}${i}`}
            class="quick-recall__picker-item"
            class:quick-recall__picker-item--selected={filters.valueListIndex ===
              i && !row.disabled}
            class:quick-recall__picker-item--disabled={row.disabled}
            role="option"
            tabindex="-1"
            aria-selected={filters.valueListIndex === i && !row.disabled}
            aria-disabled={row.disabled}
            onmousemove={() => {
              if (!row.disabled) filters.valueListIndex = i;
            }}
            onclick={() => {
              if (!row.disabled) void filters.commitValueListRow(row.token);
            }}
          >
            {#if filters.activeOperatorContext?.operator === "app:" && appIcons.src(row.label) !== null}
              <img
                class="quick-recall__picker-item-icon"
                src={appIcons.src(row.label)}
                alt=""
                aria-hidden="true"
              />
            {/if}
            <span class="quick-recall__picker-item-label">{row.label}</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if filters.valueListConflictReason !== null}
      <p class="quick-recall__picker-conflict">{filters.valueListConflictReason}</p>
    {/if}

    {#if filters.activeOperatorContext?.operator === "date:" || filters.activeOperatorContext?.operator === "after:" || filters.activeOperatorContext?.operator === "before:"}
      <p class="quick-recall__picker-hint">
        Type a date like <code>after:2026-05-01</code> for a custom range.
      </p>
    {/if}

    <p class="quick-recall__picker-cue" aria-hidden="true">
      <kbd>↑</kbd><kbd>↓</kbd> move · <kbd>↵</kbd> select · <kbd>esc</kbd> back
    </p>
  </div>
{/if}

<style>
  /* Mockup `#picker` (`.app-menu`): the picker/value list renders as a compact
     dropdown-style card anchored top-right of the body — under the field-row
     Filter button — on the raised surface with a strong hairline and the
     shared popover shadow. It still occupies the results slot (behavior
     unchanged: results unmount while a filter surface owns the region); only
     the chrome is the mockup's anchored-popover look. */
  .quick-recall__results {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .quick-recall__picker {
    flex: 0 1 auto;
    align-self: flex-start;
    margin: 8px 16px auto auto;
    width: min(320px, calc(100% - 32px));
    max-height: calc(100% - 24px);
    overflow-y: auto;
    gap: 2px;
    padding: 6px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    box-shadow: var(--app-shadow-popover);
  }

  /* Mockup `.app-menu .cat`: uppercase category header inside the card. */
  .quick-recall__picker-header {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-shrink: 0;
    padding: 6px 9px 3px;
  }

  .quick-recall__picker-title {
    font-size: var(--text-xs);
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  .quick-recall__picker-crumb-hint {
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-subtle);
  }

  .quick-recall__orient-cue-dot {
    color: var(--app-text-subtle);
    font-size: var(--text-xs);
    line-height: 1;
  }

  .quick-recall__picker-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  /* Mockup `.app-menu button`: quiet muted rows; hover lifts to the hover
     surface + strong text, the roving selection carries the accent-on-
     accent-bg `active` treatment. */
  .quick-recall__picker-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 9px;
    border-radius: 5px;
    color: var(--app-text-muted);
    cursor: pointer;
  }

  /* Real app icon on `app:` value rows (resolved by display name). */
  .quick-recall__picker-item-icon {
    flex: none;
    width: 15px;
    height: 15px;
    object-fit: contain;
  }

  .quick-recall__picker-item:not(.quick-recall__picker-item--disabled):not(
      .quick-recall__picker-item--selected
    ):hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  /* Pressed feedback for a clickable row (excludes disabled/selected, which carry
     their own treatment), so a pointer click reads as responsive. */
  .quick-recall__picker-item:not(.quick-recall__picker-item--disabled):not(
      .quick-recall__picker-item--selected
    ):active {
    background: var(--app-surface-active);
  }

  .quick-recall__picker-item--selected {
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }

  /* A value-list row that would conflict with an active chip (app + audio
     source are mutually exclusive). Dimmed and non-selectable — it never
     highlights on hover and Enter skips it. */
  .quick-recall__picker-item--disabled {
    opacity: var(--app-disabled-opacity);
    cursor: default;
  }

  /* The one-line conflict note below the value list, and the typed-date hint
     for the date operators. The conflict note is a correction prompt ("these
     filters can't combine"), so it shares the danger ramp with the inline
     parse-error line rather than reading as success-green. */
  .quick-recall__picker-conflict {
    margin: 0;
    padding: 4px 9px 2px;
    flex-shrink: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-danger-text);
  }

  .quick-recall__picker-hint {
    margin: 0;
    padding: 4px 9px 2px;
    flex-shrink: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-text-muted);
  }

  .quick-recall__picker-hint code {
    font-family: inherit;
    color: var(--app-text-muted);
  }

  .quick-recall__picker-item-label {
    font-size: var(--text-base);
    line-height: 1.3;
    color: inherit;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__picker-item-hint {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    line-height: 1.3;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__picker-item-chevron {
    flex-shrink: 0;
    font-size: var(--text-base);
    color: var(--app-text-subtle);
  }

  .quick-recall__picker-cue {
    margin: 2px 0 0;
    padding: 5px 9px 3px;
    flex-shrink: 0;
    border-top: 1px solid var(--app-border);
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .quick-recall__picker-cue kbd {
    font-family: inherit;
    font-size: var(--text-xs);
    line-height: 1;
    text-transform: lowercase;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 2px 5px;
    margin: 0 1px;
  }

  .quick-recall__state {
    margin: 0;
    padding: 6px 9px;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
  }
</style>
