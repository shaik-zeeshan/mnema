<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // RailFooter — the pinned engine-status line at the bottom of the Insights
  // rail. A single faint, lowercase line carrying the Reasoning Engine state,
  // read from the shared `EngineState` (Direction A, SB-03/MT-02):
  //   on        — accent dot + "engine · <model>" (or "· chat off · Turn on"
  //               when only the Ask AI switch is off).
  //   pitch/off — grey dot + "engine off · Enable".
  //   unreachable — amber dot + "can't reach <x> · Retry".
  //   fix       — amber dot + the provider + "Sign in again" / "Fix".
  // While the status calls are in flight a skeleton stands in so the line
  // never flashes a wrong state. The shell owns the state; this only renders.
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import type { EngineState } from "$lib/insights/engine-state";

  interface Props {
    engine: EngineState;
    chatOff: boolean;
    modelLabel: string;
    onEnable: () => void;
    onRetry: () => void;
  }

  let { engine, chatOff, modelLabel, onEnable, onRetry }: Props = $props();
</script>

<div class="rail-foot">
  {#if engine.kind === "loading"}
    <span class="rail-foot-skeleton" aria-label="Loading engine status">
      <Skeleton width="92px" height="9px" radius="5px" muted />
    </span>
  {:else if engine.kind === "on"}
    <span class="rail-engine" use:tip={"Reasoning Engine is on"}>
      <span class="dot" aria-hidden="true"></span>
      engine
      <span class="sep">·</span>
      {#if chatOff}
        chat off
        <span class="sep">·</span>
        <button type="button" class="rail-enable" onclick={onEnable}>Turn on</button>
      {:else}
        <span class="model">{modelLabel || "on"}</span>
      {/if}
    </span>
  {:else if engine.kind === "pitch" || engine.kind === "off"}
    <span class="rail-engine rail-engine--off" use:tip={"Reasoning Engine is off"}>
      <span class="dot" aria-hidden="true"></span>
      engine off
      <span class="sep">·</span>
      <button type="button" class="rail-enable" onclick={onEnable}>
        Enable
      </button>
    </span>
  {:else}
    <span class="rail-engine rail-engine--warn" use:tip={engine.text}>
      <span class="dot" aria-hidden="true"></span>
      {engine.short}
      <span class="sep">·</span>
      {#if engine.kind === "unreachable"}
        <button type="button" class="rail-enable" onclick={onRetry}>Retry</button>
      {:else}
        <button type="button" class="rail-enable" onclick={onEnable}>
          {engine.reconnectProviderId ? "Sign in again" : "Fix"}
        </button>
      {/if}
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
  .rail-engine--warn .dot {
    background: var(--app-warn);
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
