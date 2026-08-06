<script lang="ts">
  // One bento cell. The Studio Shell de-boxing rule lives here: a hairline under
  // the header instead of a card edge (`.ss-tile__h`), the optional `more` note
  // right-aligned, and the body a flex column that never scrolls — a tile is a
  // headline, and the detail belongs in the inspector.
  //
  // `quiet` is the honest-empty path (round-4 decision **G8**): when a read
  // failed or returned nothing, the body is one muted sentence, never a zero.
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    /** Right-aligned note in the header — a count, a timestamp, an affordance. */
    more?: string;
    /** When set, the tile is a door: the header-right note becomes the opener
     *  button ("Open Journal ›") that navigates to the tile's destination. */
    onopen?: () => void;
    /** Extra tile classes: `ss-tile--2`, `ss-tile--media`, … */
    span?: string;
    /** Ringed when one of this tile's rows owns the inspector. */
    selected?: boolean;
    /** When set, the body is this sentence instead of `children`. */
    quiet?: string | null;
    children?: Snippet;
  }

  let { label, more, onopen, span = "", selected = false, quiet = null, children }: Props = $props();
</script>

<section class="ss-tile {span}" class:is-sel={selected}>
  <div class="ss-tile__h">
    <span class="t-label">{label}</span>
    {#if onopen}
      <button type="button" class="ss-more open" onclick={onopen}>
        {more}<span aria-hidden="true"> ›</span>
      </button>
    {:else if more}<span class="ss-more">{more}</span>{/if}
  </div>
  <div class="ss-tile__b">
    {#if quiet}
      <p class="quiet">{quiet}</p>
    {:else if children}
      {@render children()}
    {/if}
  </div>
</section>

<style>
  /* The opener inherits `.ss-more`'s seat in the header and adds the accent +
     button reset. Keyboard reachable — a destination must not be mouse-only. */
  .open {
    border: none;
    background: transparent;
    padding: 0;
    font: inherit;
    cursor: pointer;
    color: var(--app-accent-strong);
    transition: color 0.12s ease;
  }
  .open:hover {
    color: var(--app-accent);
  }
  .open:focus-visible {
    outline: none;
    border-radius: 4px;
    box-shadow: var(--app-ring);
  }

  .quiet {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
</style>
