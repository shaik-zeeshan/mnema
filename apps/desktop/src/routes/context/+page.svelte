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
    background: var(--app-bg);
  }
  /* The destination pane's own tint, same two washes every destination wears —
     accent for the authored layer, info for the inferred one it steers. */
  .dest::before {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    background:
      radial-gradient(
        520px 320px at 4% 0%,
        color-mix(in srgb, var(--app-accent) 10%, transparent),
        transparent 70%
      ),
      radial-gradient(
        420px 380px at 98% 96%,
        color-mix(in srgb, var(--app-info) 12%, transparent),
        transparent 72%
      );
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
