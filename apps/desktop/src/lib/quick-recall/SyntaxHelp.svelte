<!-- Quick Recall body-operator syntax help affordance.

     A small `?` trigger in the search field row toggles a static popover that
     documents the Body Match Operators (`"phrase"`, `-term`, `OR`, `term*`).
     These operators stay TYPED TEXT — they are not chips and not in the Filter
     Picker — so this affordance is the place they're discoverable. The field
     operators (app:/source:/date:) are listed too since they pair with the
     picker. Restyled to the mockup's `#syn` panel: one uppercase title, a
     96px/1fr grid per row, accent operator + muted description, popover chrome
     on the shared `--app-shadow-popover`.

     The content is entirely static: the only state is the store's open/close
     boolean. Dismissal is threefold and must NOT clobber the surrounding
     Escape handlers: the trigger toggles it closed; an outside pointerdown
     closes it (the page-level $effect registers a document listener only while
     open, reading the wrapper element bound to the store here); and Escape
     closes it WITHOUT bubbling to the layout's window-close handler (handled
     at the top of the search keydown routing). The button is occasional-use
     and deliberately doesn't keep focus — DOM focus stays on the search input. -->
<script lang="ts">
  import { fade } from "svelte/transition";
  import { quickRecallSearch as search } from "$lib/quick-recall/searchStore.svelte";

  let { fadeMs }: { fadeMs: number } = $props();

  const SYNTAX_HELP_POPOVER_ID = "quick-recall-syntax-help";

  // Mockup `#syn` rows, verbatim (op → one-line description).
  const SYNTAX_ROWS: ReadonlyArray<{ op: string; desc: string }> = [
    { op: '"phrase"', desc: "exact phrase" },
    { op: "-term", desc: "exclude a term" },
    { op: "OR", desc: "either side matches" },
    { op: "term*", desc: "prefix match" },
    { op: "app:", desc: "only this app" },
    { op: "source:", desc: "screen · mic · system" },
    { op: "date:", desc: "on a day" },
    { op: "after:", desc: "since a day" },
    { op: "before:", desc: "up to a day" },
  ];
</script>

<div class="quick-recall__syntax" bind:this={search.syntaxHelpEl}>
  <button
    type="button"
    class="quick-recall__syntax-trigger"
    class:quick-recall__syntax-trigger--open={search.syntaxHelpOpen}
    onclick={() => search.toggleSyntaxHelp()}
    aria-label="Search syntax help"
    aria-expanded={search.syntaxHelpOpen}
    aria-controls={SYNTAX_HELP_POPOVER_ID}
  >
    ?
  </button>
  {#if search.syntaxHelpOpen}
    <div
      id={SYNTAX_HELP_POPOVER_ID}
      class="quick-recall__syntax-popover"
      role="tooltip"
      in:fade={{ duration: fadeMs }}
    >
      <p class="quick-recall__syntax-title">Search syntax</p>
      {#each SYNTAX_ROWS as row (row.op)}
        <div class="quick-recall__syntax-row">
          <span class="quick-recall__syntax-op">{row.op}</span>
          <span class="quick-recall__syntax-desc">{row.desc}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* The wrapper is the positioning context for the popover. Trigger matches
     the mockup `.synhelp > button`: a 22px round `?` on the raised surface
     with a strong hairline; hover/open lift to border-hover + strong text. */
  .quick-recall__syntax {
    position: relative;
    flex-shrink: 0;
    display: flex;
  }

  .quick-recall__syntax-trigger {
    width: 22px;
    height: 22px;
    font-family: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    color: var(--app-text-subtle);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 50%;
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      color 0.12s ease;
  }

  .quick-recall__syntax-trigger:hover,
  .quick-recall__syntax-trigger--open {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }

  .quick-recall__syntax-trigger:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__syntax-trigger:active {
    background: var(--app-surface-active);
  }

  /* Mockup `.syn-pop`: 252px card below-right of the trigger on the raised
     surface, strong hairline, shared popover shadow. Static documentation —
     no interactive content — so role="tooltip" suffices. */
  .quick-recall__syntax-popover {
    position: absolute;
    top: calc(100% + 10px);
    right: 0;
    z-index: 10;
    width: 252px;
    max-width: min(252px, calc(100vw - 30px));
    padding: 10px 12px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    box-shadow: var(--app-shadow-popover);
  }

  .quick-recall__syntax-title {
    margin: 0 0 7px;
    font-size: var(--text-xs);
    line-height: 1.3;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  .quick-recall__syntax-row {
    display: grid;
    grid-template-columns: 96px 1fr;
    gap: 8px;
    font-size: var(--text-sm);
    line-height: 1.5;
    padding: 2px 0;
  }

  .quick-recall__syntax-op {
    color: var(--app-accent);
    white-space: nowrap;
  }

  .quick-recall__syntax-desc {
    color: var(--app-text-muted);
  }

  @media (prefers-reduced-motion: reduce) {
    .quick-recall__syntax-trigger {
      transition: none;
    }
  }
</style>
