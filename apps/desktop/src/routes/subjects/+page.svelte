<script lang="ts">
  // Subjects — a destination, not a surface (direction 01, mockup 09). Opened
  // from Overview's Subjects tile; `?s=<subject>` is the drill-in (the titlebar
  // then reads `‹ Subjects`). Implementation lands in the page-09 slice.
</script>

<div class="subjects-dest scroll">
  <p class="t-meta">Subjects</p>
</div>

<style>
  .subjects-dest {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-16) var(--pad-window) var(--s-48);
  }
</style>
