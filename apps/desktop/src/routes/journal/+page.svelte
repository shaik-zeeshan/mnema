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
    background: var(--app-bg);
  }
  /* The destination pane's own tint — two soft washes so the plates read as
     floating on a lit floor rather than sitting on a flat fill. */
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
        color-mix(in srgb, var(--cat-communication) 12%, transparent),
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
