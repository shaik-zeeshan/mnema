<script lang="ts">
  // Landing chip: a Quick Recall hit opened the Timeline, so its query rides
  // along. Shown with one `.tl-matchmark` at the landed frame (rendered in the
  // rail by the page); dismiss clears both. No dots for other matches (§8 #2).
  import { tip } from "$lib/components/tooltip";
  import IconDismiss from "~icons/lucide/x";

  let { query, ondismiss }: { query: string; ondismiss: () => void } = $props();
</script>

<span class="tl-searchchip">
  <span class="mx-chip mx-chip--accent tl-searchchip__label" use:tip={query}
    ><span>from Recall · “{query}”</span></span
  >
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
    aria-label="Dismiss"
    use:tip={"Dismiss"}
    onclick={ondismiss}><IconDismiss width="13" height="13" /></button
  >
</span>

<style>
  .tl-searchchip { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .tl-searchchip__label { max-width: 36ch; }
  .tl-searchchip__label span { min-width: 0; overflow: hidden; text-overflow: ellipsis; }

  /* ponytail: the mark lives in the page's rail track, but it only exists while
     this chip is mounted, so its styling rides here. */
  :global(.tl-matchmark) {
    position: absolute; z-index: 4; top: 6px; width: 4px; height: 4px; margin-right: 2px; border-radius: 50%;
    background: var(--app-accent); box-shadow: 0 0 0 2px var(--app-accent-glow); pointer-events: none;
    animation: tl-matchmark-rise var(--t-slow) var(--ease-expo) both;
  }
  @keyframes -global-tl-matchmark-rise { from { opacity: 0; transform: translateY(8px); } }
  @media (prefers-reduced-motion: reduce) { :global(.tl-matchmark) { animation: none; } }
</style>
