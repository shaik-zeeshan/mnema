<script lang="ts">
  // Context's 30px tool strip. The way back is its FIRST control — this surface
  // is a destination opened from the Overview's Context tile, not a peer of
  // Timeline, so "‹ Overview" leads and the label states where you are.
  //
  // The counts and the derivation chip are real reads or they are absent (G8).
  import IconBack from "~icons/lucide/chevron-left";
  import IconPanel from "~icons/lucide/panel-right";
  import IconSliders from "~icons/lucide/sliders-horizontal";
  import { openSettings } from "$lib/surface-windows";
  import type { ContextData } from "./context-data.svelte";

  interface Props {
    data: ContextData;
    inspectorOpen: boolean;
    inspectorAvailable: boolean;
    onback: () => void;
    ontoggleinspector: () => void;
  }

  let { data, inspectorOpen, inspectorAvailable, onback, ontoggleinspector }: Props = $props();

  const counts = $derived.by<string | null>(() => {
    const parts: string[] = [];
    if (data.standingCount !== null) parts.push(`${data.standingCount} standing`);
    if (data.dismissedCount !== null) parts.push(`${data.dismissedCount} dismissed`);
    return parts.length > 0 ? parts.join(" · ") : null;
  });

  // `budgetTier` is the derivation mode; with no engine there is no derivation
  // to have a mode, and the chip says that rather than naming a tier that is
  // not running.
  const derivation = $derived.by<string | null>(() => {
    const status = data.status;
    if (!status) return null;
    if (!status.engineAvailable) return "Derivation · unavailable";
    const tier = status.budgetTier;
    return `Derivation · ${tier.charAt(0).toUpperCase()}${tier.slice(1)}`;
  });
</script>

<div class="ss-tstrip">
  <div class="ss-tstrip__g">
    <button type="button" class="btn btn--sm btn--ghost" onclick={onback}>
      <IconBack /> Overview
    </button>
    <div class="ss-tstrip__sep"></div>
    <span class="t-label name">Context</span>
  </div>

  {#if counts}
    <div class="ss-tstrip__sep"></div>
    <span class="t-meta is-mono">{counts}</span>
  {/if}

  <span class="ss-tstrip__spacer"></span>

  {#if derivation}
    <span class="ss-chip">{derivation}</span>
  {/if}
  <button type="button" class="btn btn--sm btn--ghost" onclick={() => void openSettings("userContext")}>
    <IconSliders /> User Context settings
  </button>

  {#if inspectorAvailable}
    <div class="ss-tstrip__sep"></div>
    <button
      type="button"
      class="btn btn--sm btn--icon"
      class:is-on={inspectorOpen}
      aria-pressed={inspectorOpen}
      aria-label="Toggle inspector"
      onclick={ontoggleinspector}><IconPanel /></button
    >
  {/if}
</div>

<style>
  .name {
    color: var(--app-text-strong);
  }

  .btn.is-on {
    background: var(--app-surface-active);
  }
</style>
