<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // RailFooter — the pinned engine-status line at the bottom of the Insights
  // rail (Insights-rail refactor, Slices 2/3). Mirrors the mockup's `.rail-foot`
  // / `.rail-engine` minimal idiom: a single faint, lowercase line carrying the
  // Reasoning Engine state. Two variants:
  //   ON  — accent dot + "engine · <model>" (muted).
  //   OFF — grey dot + "engine off ·" + a quiet dotted "Enable" link.
  // While the status calls are still in flight (`!statusLoaded`) a tiny skeleton
  // placeholder stands in so the line doesn't flash "engine off" before the
  // first load lands. The owning shell (`+page.svelte`) keeps the status state
  // and passes it down; this component only renders + reports the Enable click.
  import Skeleton from "$lib/insights/Skeleton.svelte";

  interface Props {
    engineOn: boolean;
    modelLabel: string;
    statusLoaded: boolean;
    onEnable: () => void;
  }

  let { engineOn, modelLabel, statusLoaded, onEnable }: Props = $props();
</script>

<div class="rail-foot">
  {#if !statusLoaded}
    <span class="rail-foot-skeleton" aria-label="Loading engine status">
      <Skeleton width="92px" height="9px" radius="5px" muted />
    </span>
  {:else if engineOn}
    <span class="rail-engine" use:tip={"Reasoning Engine is on"}>
      <span class="dot" aria-hidden="true"></span>
      engine
      <span class="sep">·</span>
      <span class="model">{modelLabel || "on"}</span>
    </span>
  {:else}
    <span class="rail-engine rail-engine--off" use:tip={"Reasoning Engine is off"}>
      <span class="dot" aria-hidden="true"></span>
      engine off
      <span class="sep">·</span>
      <button type="button" class="rail-enable" onclick={onEnable}>
        Enable
      </button>
    </span>
  {/if}
</div>

<style>
  /* Pinned single faint line carrying the engine status — mirrors the mockup's
     `.rail-foot`. Token-driven; lowercase, minimal. */
  .rail-foot {
    flex: 0 0 auto;
    border-top: 1px solid var(--app-border);
    padding: 12px 16px;
    display: flex;
    align-items: center;
    gap: 6px;
    /* 9.5px was below the legible floor for this persistent status line; lift it
       to 11px (the rest of the line/Enable link inherit, so it reads clearly). */
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .rail-foot-skeleton {
    display: inline-flex;
    align-items: center;
  }
  /* Engine status — ON variant: accent dot, muted label. */
  .rail-engine {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--app-text-muted);
  }
  .rail-engine .dot {
    width: 5px;
    height: 5px;
    border-radius: 999px;
    background: var(--app-accent);
    flex: 0 0 auto;
  }
  .rail-engine .sep {
    color: var(--app-text-faint);
  }
  .rail-engine .model {
    color: var(--app-text-muted);
  }
  /* OFF variant: grey dot + a tiny dotted "Enable" link. */
  .rail-engine--off .dot {
    background: var(--app-status-dot);
  }
  .rail-enable {
    color: var(--app-accent-strong);
    cursor: pointer;
    border-bottom: 1px dotted var(--app-accent-border);
    border-top: 0;
    border-left: 0;
    border-right: 0;
    background: transparent;
    padding: 0;
    font: inherit;
    font-size: 11px;
  }
  .rail-enable:hover {
    color: var(--app-accent);
  }
  .rail-enable:focus-visible {
    outline: none;
    color: var(--app-accent);
    border-bottom-color: var(--app-accent);
  }
</style>
