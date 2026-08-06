<script lang="ts">
  // Context destination (page 10) — what you told Mnema, beside what it
  // concluded. Opened from the Overview "Context" tile's door; the way back is
  // the ‹ Overview button the shared title bar draws on this route.
  import Context from "$lib/insights/Context.svelte";
</script>

<div class="dest">
  <div class="dest__scroll">
    <Context />
  </div>
</div>

<style>
  .dest {
    flex: 1 1 auto;
    min-height: 0;
    margin-top: calc(var(--h-titlebar) * -1);
    position: relative;
  }
  .dest__scroll {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: calc(var(--h-titlebar) + 14px) 20px 28px;
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-hover) transparent;
  }
</style>
