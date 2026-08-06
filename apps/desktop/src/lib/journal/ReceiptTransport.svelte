<script lang="ts">
  // The receipt's transport: scrub bar (one tick per cited frame, the headline
  // frame emphasized), filmstrip (cited cells marked), and the control row —
  // ␣ play/pause, the 1×/2×/8×/16× speed picker, the frame counter, ←→ step,
  // and the Open in Timeline handoff. Presentational: every bit of playback
  // state and every handler comes from Receipt.svelte, which owns the clock.
  import Segmented from "$lib/components/Segmented.svelte";
  import type { Speed } from "$lib/insights/receipt-playback";

  interface Props {
    strip: { id: number; ms: number }[];
    index: number;
    ticks: { pos: number; headline: boolean }[];
    citedIds: Set<number>;
    thumbUrls: Record<number, string>;
    currentPos: number;
    headPos: number;
    headClock: string;
    startLabel: string;
    endLabel: string;
    isAudioOnly: boolean;
    isPlaying: boolean;
    playDisabled: boolean;
    speed: Speed;
    speedOptions: { value: string; label: string }[];
    counter: string;
    /** Wall clock for a filmstrip cell's aria-label. */
    clockOf: (ms: number) => string;
    /** Svelte action that queues a cell's thumbnail when it scrolls into view. */
    thumbCell: (node: HTMLElement, fid: number) => { destroy: () => void };
    trackEl: HTMLDivElement | null;
    onTrackPointerDown: (e: PointerEvent) => void;
    onTrackPointerMove: (e: PointerEvent) => void;
    onTrackPointerUp: (e: PointerEvent) => void;
    onTrackPointerCancel: (e: PointerEvent) => void;
    onSeek: (i: number) => void;
    onTogglePlay: () => void;
    onSpeedChange: (v: string) => void;
    onOpenTimeline: () => void;
  }

  let {
    strip,
    index,
    ticks,
    citedIds,
    thumbUrls,
    currentPos,
    headPos,
    headClock,
    startLabel,
    endLabel,
    isAudioOnly,
    isPlaying,
    playDisabled,
    speed,
    speedOptions,
    counter,
    clockOf,
    thumbCell,
    trackEl = $bindable(),
    onTrackPointerDown,
    onTrackPointerMove,
    onTrackPointerUp,
    onTrackPointerCancel,
    onSeek,
    onTogglePlay,
    onSpeedChange,
    onOpenTimeline,
  }: Props = $props();

  let filmEl = $state<HTMLDivElement | null>(null);

  // Keep the current cell in view as playback / scrubbing advances.
  $effect(() => {
    const cell = filmEl?.children[index] as HTMLElement | undefined;
    cell?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
</script>

<div class="rcpt__scrub">
  <div
    class="scrub"
    bind:this={trackEl}
    role="slider"
    aria-label="Scrub"
    aria-valuemin={1}
    aria-valuemax={Math.max(1, strip.length)}
    aria-valuenow={index + 1}
    tabindex="-1"
    onpointerdown={onTrackPointerDown}
    onpointermove={onTrackPointerMove}
    onpointerup={onTrackPointerUp}
    onpointercancel={onTrackPointerCancel}
  >
    {#if !isAudioOnly}
      <span class="scrub__f" style="width:{currentPos * 100}%"></span>
      {#each ticks as t, i (i)}
        <span class="ev" class:ev--hl={t.headline} style="left:{t.pos * 100}%"></span>
      {/each}
    {/if}
    {#if headClock}
      <span class="scrub__head" style="left:{headPos * 100}%">{headClock}</span>
    {/if}
  </div>
  <div class="scrub__caps">
    <span>{startLabel}</span>
    <span class="scrub__caps-end">{endLabel}</span>
  </div>
</div>

{#if !isAudioOnly}
  <div class="film" bind:this={filmEl}>
    {#each strip as f, ti (f.id)}
      <button
        type="button"
        class="film__c"
        class:on={ti === index}
        class:cited={citedIds.has(f.id)}
        aria-label={`Seek to ${clockOf(f.ms)}`}
        use:thumbCell={f.id}
        onclick={() => onSeek(ti)}
      >
        {#if thumbUrls[f.id]}<img src={thumbUrls[f.id]} alt="" />{/if}
      </button>
    {/each}
  </div>
{/if}

<div class="rcpt__ctrl">
  <button
    type="button"
    class="btn btn--icon btn--sm"
    aria-label={isPlaying ? "Pause" : "Play"}
    disabled={playDisabled}
    onclick={onTogglePlay}
  >
    {#if isPlaying}
      <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><rect x="4.4" y="3.4" width="2.6" height="9.2" rx=".8" /><rect x="9" y="3.4" width="2.6" height="9.2" rx=".8" /></svg>
    {:else}
      <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M4.6 3.2 12.4 8l-7.8 4.8z" /></svg>
    {/if}
  </button>
  <span class="hint"><span class="kbd">␣</span></span>
  {#if !isAudioOnly}
    <Segmented
      options={speedOptions}
      value={String(speed)}
      onValueChange={onSpeedChange}
      ariaLabel="Playback speed"
      compact
    />
  {/if}
  <span class="t-meta is-mono is-num rcpt__count">{counter}</span>
  <span class="hint rcpt__step">
    <span class="kbd">←</span><span class="kbd">→</span><span>step frame</span>
  </span>
  <button type="button" class="btn btn--sm" onclick={onOpenTimeline}>
    Open in Timeline
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" /></svg>
  </button>
</div>

<style>
  .rcpt__scrub {
    flex: 0 0 auto;
    padding: var(--s-20) var(--s-12) 0;
  }
  .scrub {
    position: relative;
    height: 6px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
    touch-action: none;
  }
  .scrub__f {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    border-radius: 3px;
    background: var(--app-accent);
    pointer-events: none;
  }
  .ev {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 2px;
    border-radius: 1px;
    background: var(--app-text-subtle);
    pointer-events: none;
  }
  .ev--hl {
    width: 3px;
    background: var(--app-text-strong);
    box-shadow:
      0 0 0 2px var(--app-accent),
      0 0 0 4px var(--app-accent-glow);
  }
  .scrub__head {
    position: absolute;
    top: -22px;
    transform: translateX(-50%);
    padding: 2px 6px;
    border-radius: var(--r-sm);
    background: var(--app-surface-raised);
    box-shadow: 0 0 0 var(--hairline) var(--app-border-strong);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
    white-space: nowrap;
    pointer-events: none;
  }
  .scrub__caps {
    display: flex;
    margin-top: 5px;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .scrub__caps-end {
    margin-left: auto;
  }

  .film {
    flex: 0 0 auto;
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: calc((100% - 11 * var(--s-4)) / 12);
    gap: var(--s-4);
    padding: var(--s-8) var(--s-12) 0;
    overflow-x: auto;
    overflow-y: hidden;
  }
  .film__c {
    position: relative;
    aspect-ratio: 16 / 10;
    padding: 0;
    border: 0;
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
    cursor: default;
  }
  .film__c img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .film__c.on {
    box-shadow: 0 0 0 1.5px var(--app-accent);
  }
  .film__c.cited::before {
    content: "";
    position: absolute;
    left: 4px;
    top: 4px;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--app-accent);
    z-index: 3;
  }

  .rcpt__ctrl {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: var(--s-8) var(--s-12);
  }
  .rcpt__ctrl svg {
    width: 12px;
    height: 12px;
  }
  .rcpt__count {
    color: var(--app-text-muted);
  }
  .rcpt__step {
    margin-left: auto;
  }
</style>
