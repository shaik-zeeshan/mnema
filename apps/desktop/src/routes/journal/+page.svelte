<script lang="ts">
  // Journal — a destination, not a surface (direction 01, mockup 08). Opened
  // from Overview's Moments tile; the titlebar's `‹ Overview` chevron (owned by
  // +layout.svelte) is the way back. Implementation lands in the page-08 slice.
</script>

<div class="journal-dest scroll">
  <p class="t-meta">Journal</p>
</div>

<style>
  .journal-dest {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-16) var(--pad-window) var(--s-48);
  }
</style>
