<script lang="ts">
  // The one bento tile surface (redesign frame 04): a single-level borderless
  // fill at the System-Settings group radius — a tile and a settings group are
  // the same surface. Grid placement (span per width) belongs to the parent
  // grid's CSS via the passthrough `class`; this component owns only the
  // surface (fill, radius, padding, optional label header).
  import type { Snippet } from "svelte";

  let {
    label,
    more,
    media = false,
    class: klass = "",
    children,
  }: {
    /** Machine label in the tile header ("TODAY", "CAPTURE"). */
    label?: string;
    /** Right-aligned quiet meta in the header ("updated 14:40", "3 today"). */
    more?: string;
    /** Media tile: no padding, content clipped by the radius (the strip). */
    media?: boolean;
    class?: string;
    children?: Snippet;
  } = $props();
</script>

<section class="tile {klass}" class:tile--media={media}>
  {#if label !== undefined}
    <div class="tile-h">
      <span class="tile-h__label t-label">{label}</span>
      {#if more}<span class="tile-h__more">{more}</span>{/if}
    </div>
  {/if}
  {@render children?.()}
</section>

<style>
  .tile {
    background: var(--app-surface);
    border-radius: 10px; /* the System Settings group radius (between --r-lg and --r-xl) */
    padding: var(--pad-panel);
    min-width: 0;
  }

  .tile--media {
    padding: 0;
    overflow: hidden;
  }

  .tile-h {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
    margin-bottom: var(--s-8);
  }

  .tile-h__label {
    color: var(--app-text-subtle);
  }

  .tile-h__more {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
