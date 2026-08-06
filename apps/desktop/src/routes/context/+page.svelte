<script lang="ts">
  // Context — a destination, not a surface (direction 01, mockup 10). Opened
  // from Overview's Context tile header. Implementation lands in the page-10
  // slice.
</script>

<div class="context-dest scroll">
  <p class="t-meta">Context</p>
</div>

<style>
  .context-dest {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-16) var(--pad-window) var(--s-48);
  }
</style>
