<script lang="ts">
  // Journal destination (page 08) — the day as a river of plates. Opened from
  // the Overview "Today" tile's door; the way back is the ‹ Overview button the
  // shared title bar draws on this route. The surface itself is <DayTimeline/>.
  import DayTimeline from "$lib/insights/DayTimeline.svelte";
</script>

<div class="dest">
  <div class="dest__scroll">
    <DayTimeline />
  </div>
</div>

<style>
  /* Same scroll-under-the-chrome idiom as Overview: pull the pane up under the
     glass title bar, pad the content back down past it. */
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
