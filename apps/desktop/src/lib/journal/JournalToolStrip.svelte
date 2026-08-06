<script lang="ts">
  // Journal's 30px tool strip — the day's navigation, and the way back.
  //
  // Journal is a destination, not a surface (direction 02 README, bullet 6), so
  // the FIRST control is "‹ Overview": the tool strip carries the return, and the
  // title bar's two-surface toggle is left alone. After it: the day stepper, the
  // three band jumps, "↻ re-read" and the inspector toggle.
  import IconBack from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";
  import IconUpDown from "~icons/lucide/chevrons-up-down";
  import IconPanel from "~icons/lucide/panel-right";
  import IconRefresh from "~icons/lucide/refresh-cw";
  import { dayKeyOf, formatDayShort } from "$lib/overview/overview-format";
  import type { BandLabel } from "$lib/insights/journal-view";

  interface Props {
    dayKey: string;
    /** Bands that actually have rows today — the others' chips stay disabled. */
    availableBands: BandLabel[];
    /** True while a re-read is in flight; false hides nothing, it only disables. */
    regenerating: boolean;
    /** The re-read control only exists when the engine can write one. */
    canReRead: boolean;
    inspectorOpen: boolean;
    inspectorAvailable: boolean;
    onday: (dayKey: string) => void;
    onstep: (days: number) => void;
    onjump: (band: BandLabel) => void;
    onreread: () => void;
    ontoggleinspector: () => void;
    onback: () => void;
  }

  let {
    dayKey,
    availableBands,
    regenerating,
    canReRead,
    inspectorOpen,
    inspectorAvailable,
    onday,
    onstep,
    onjump,
    onreread,
    ontoggleinspector,
    onback,
  }: Props = $props();

  const BANDS: BandLabel[] = ["Morning", "Afternoon", "Evening"];
  const todayKey = dayKeyOf(new Date());
  const isToday = $derived(dayKey === todayKey);
  const canStepForward = $derived(dayKey < todayKey);
</script>

<div class="ss-tstrip">
  <div class="ss-tstrip__g">
    <button type="button" class="btn btn--sm btn--ghost" onclick={onback}>
      <IconBack />Overview
    </button>
    <div class="ss-tstrip__sep"></div>
    <span class="label">Journal</span>
  </div>

  <div class="ss-tstrip__sep"></div>

  <div class="ss-tstrip__g">
    <button
      type="button"
      class="btn btn--sm btn--icon btn--ghost"
      aria-label="Previous day"
      onclick={() => onstep(-1)}
    ><IconBack /></button>
    <!-- The OS ships a calendar, a keyboard path and a locale-correct format;
         a bespoke popover would be the same control with bugs. -->
    <label class="ss-pop pop" aria-label="Day shown">
      <span class="pop__t">{formatDayShort(dayKey)}</span>
      {#if isToday}<span class="live" aria-label="today"></span>{/if}
      <span class="ss-pop__b" aria-hidden="true"><IconUpDown /></span>
      <input type="date" value={dayKey} max={todayKey} onchange={(e) => onday(e.currentTarget.value)} />
    </label>
    <button
      type="button"
      class="btn btn--sm btn--icon btn--ghost"
      aria-label="Next day"
      disabled={!canStepForward}
      onclick={() => onstep(1)}
    ><IconNext /></button>
  </div>

  <div class="ss-tstrip__sep"></div>

  <div class="ss-seg" role="group" aria-label="Jump to a part of the day">
    {#each BANDS as band (band)}
      <button
        type="button"
        class="ss-seg__i"
        disabled={!availableBands.includes(band)}
        onclick={() => onjump(band)}
      >{band}</button>
    {/each}
  </div>

  <span class="ss-tstrip__spacer"></span>

  {#if canReRead}
    <button
      type="button"
      class="btn btn--sm btn--ghost"
      class:is-busy={regenerating}
      disabled={regenerating}
      onclick={onreread}
    >
      <span class="spin" class:is-busy={regenerating}><IconRefresh /></span>
      {regenerating ? "reading…" : "re-read"}
    </button>
    <div class="ss-tstrip__sep"></div>
  {/if}

  {#if inspectorAvailable}
    <button
      type="button"
      class="btn btn--sm btn--icon"
      class:is-on={inspectorOpen}
      aria-pressed={inspectorOpen}
      aria-label="Toggle inspector"
      title="Inspector ⌥⌘I"
      onclick={ontoggleinspector}
    ><IconPanel /></button>
  {/if}
</div>

<style>
  .label {
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-strong);
  }

  .pop {
    position: relative;
    gap: var(--s-6);
    padding: 0 5px 0 var(--s-8);
    cursor: default;
  }
  .pop__t {
    font-variant-numeric: tabular-nums;
  }
  .pop :global(.ss-pop__b svg) {
    width: 9px;
    height: 9px;
    stroke-width: 2.4;
  }
  /* The native picker overlays the pill; its own indicator is the click target. */
  .pop input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    border: 0;
    padding: 0;
    cursor: default;
  }
  /* Today reads as live — the same green the record state uses elsewhere. */
  .live {
    width: 6px;
    height: 6px;
    border-radius: 2px;
    background: var(--app-accent);
    flex: 0 0 auto;
  }

  .ss-seg__i {
    cursor: pointer;
  }
  .ss-seg__i:hover:not(:disabled) {
    color: var(--app-text-strong);
  }
  .ss-seg__i:disabled {
    opacity: var(--opacity-disabled, 0.4);
    cursor: default;
  }

  .btn.is-on {
    background: var(--app-surface-active);
  }
  .spin {
    display: flex;
  }
  .spin.is-busy {
    animation: journal-spin 0.8s linear infinite;
  }
  @keyframes journal-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin.is-busy {
      animation: none;
    }
  }
</style>
