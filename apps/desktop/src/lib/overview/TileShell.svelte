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
    /** Extra tile classes: `ss-tile--2`, `ss-tile--media`, … */
    span?: string;
    /** Ringed when one of this tile's rows owns the inspector. */
    selected?: boolean;
    /** When set, the body is this sentence instead of `children`. */
    quiet?: string | null;
    children?: Snippet;
  }

  let { label, more, span = "", selected = false, quiet = null, children }: Props = $props();
</script>

<section class="ss-tile {span}" class:is-sel={selected}>
  <div class="ss-tile__h">
    <span class="t-label">{label}</span>
    {#if more}<span class="ss-more">{more}</span>{/if}
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
  .quiet {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
</style>
