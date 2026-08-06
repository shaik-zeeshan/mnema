<script lang="ts">
  // The receipt's transport row (mockup 08): the scrub track carrying one tick
  // per cited frame with the headline tick at full strength, the play button and
  // the 1×/2×/8×/16× speeds, the frame counter, "Open in Timeline", and the
  // footer counts line. Split out of Receipt.svelte to keep both files under the
  // 800-line ceiling; purely presentational — the parent owns every clock.
  import IconChevRight from "~icons/lucide/chevron-right";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
  import { clock } from "$lib/insights/receipt-clock";
  import { SPEEDS, type Speed } from "$lib/insights/receipt-playback";

  let {
    startMs,
    endMs,
    frameCount,
    index,
    ticks,
    currentPos,
    headPos,
    headClock,
    isAudioOnly,
    canPlay,
    isPlaying,
    speed,
    counter,
    footer,
    trackEl = $bindable(),
    onTogglePlay,
    onSpeedChange,
    onOpenInTimeline,
    onTrackPointerDown,
    onTrackPointerMove,
    onTrackPointerUp,
    onTrackPointerCancel,
  }: {
    startMs: number;
    endMs: number;
    frameCount: number;
    index: number;
    ticks: { pos: number; headline: boolean }[];
    currentPos: number;
    headPos: number;
    headClock: string;
    isAudioOnly: boolean;
    canPlay: boolean;
    isPlaying: boolean;
    speed: Speed;
    counter: string;
    footer: string[];
    trackEl: HTMLDivElement | null;
    onTogglePlay: () => void;
    onSpeedChange: (speed: Speed) => void;
    onOpenInTimeline: () => void;
    onTrackPointerDown: (e: PointerEvent) => void;
    onTrackPointerMove: (e: PointerEvent) => void;
    onTrackPointerUp: (e: PointerEvent) => void;
    onTrackPointerCancel: (e: PointerEvent) => void;
  } = $props();
</script>

<div>
  <div
    class="trk"
    bind:this={trackEl}
    role="slider"
    aria-label="Scrub"
    aria-valuemin={1}
    aria-valuemax={Math.max(1, frameCount)}
    aria-valuenow={index + 1}
    tabindex="-1"
    onpointerdown={onTrackPointerDown}
    onpointermove={onTrackPointerMove}
    onpointerup={onTrackPointerUp}
    onpointercancel={onTrackPointerCancel}
  >
    {#if !isAudioOnly}
      {#each ticks as t, i (i)}
        <span class="trk__ev" class:hd={t.headline} style="left:{t.pos * 100}%"></span>
      {/each}
      <span class="trk__f" style="width:{currentPos * 100}%"></span>
    {/if}
    <span class="trk__head" style="left:{headPos * 100}%">{headClock}</span>
  </div>
  <div class="trkcaps"><span>{clock(startMs)}</span><span>{clock(endMs)}</span></div>
</div>

<div class="ctl">
  <button
    type="button"
    class="btn btn--icon"
    aria-label={isPlaying ? "Pause" : "Play"}
    disabled={!canPlay}
    onclick={onTogglePlay}
  >{#if isPlaying}<IconPause />{:else}<IconPlay />{/if}</button>
  {#if !isAudioOnly}
    <span class="seg seg--sm" role="radiogroup" aria-label="Playback speed">
      {#each SPEEDS as s (s)}
        <button
          type="button"
          class="seg__i"
          class:on={speed === s}
          role="radio"
          aria-checked={speed === s}
          onclick={() => onSpeedChange(s)}>{s}×</button
        >
      {/each}
    </span>
  {/if}
  <span class="ctl__c">{counter}</span>
  <button type="button" class="btn btn--sm open" onclick={onOpenInTimeline}>
    Open in Timeline <IconChevRight />
  </button>
</div>

<div class="rfoot">
  {#each footer as part, i (i)}
    {#if i > 0}<span>·</span>{/if}<span>{part}</span>
  {/each}
</div>

<style>
  /* Scrub track — one tick per cited frame, the headline tick full strength. */
  .trk {
    position: relative;
    height: 6px;
    border-radius: 3px;
    background: var(--app-border-strong);
    cursor: pointer;
    touch-action: none;
  }
  .trk__f {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    border-radius: 3px;
    background: var(--app-accent);
    pointer-events: none;
  }
  .trk__ev {
    position: absolute;
    top: -4px;
    width: 2px;
    height: 14px;
    border-radius: 1px;
    background: var(--app-accent);
    opacity: 0.5;
    pointer-events: none;
  }
  .trk__ev.hd {
    opacity: 1;
    box-shadow: 0 0 6px var(--app-accent-glow);
  }
  .trk__head {
    position: absolute;
    top: -9px;
    height: 20px;
    padding: 0 var(--s-6);
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    border-radius: var(--r-pill);
    background: var(--app-accent);
    color: var(--app-accent-contrast);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    pointer-events: none;
  }
  .trkcaps {
    display: flex;
    justify-content: space-between;
    margin-top: var(--s-8);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }

  .ctl {
    display: flex;
    align-items: center;
    gap: var(--s-12);
  }
  .ctl button {
    cursor: pointer;
  }
  .ctl button:disabled {
    opacity: var(--opacity-disabled);
    cursor: default;
  }
  .ctl :global(svg) {
    width: 12px;
    height: 12px;
  }
  .ctl__c {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }
  .open {
    margin-left: auto;
  }

  .rfoot {
    display: flex;
    gap: var(--s-8);
    padding-top: var(--s-8);
    border-top: var(--hairline) dashed var(--tile-sep);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }
</style>
