<script lang="ts">
  // The scrub bar, told the truth about what is inside it.
  //
  // The interactive element is the SAME real `<input type="range">` the drawer
  // has always had, keeping its aria-valuetext and full keyboard operability;
  // the amplitude bars are decoration layered on top. Bar height is real
  // amplitude from `get_audio_segment_waveform_peaks`; bar hue is whoever held
  // the floor at that moment. An empty peaks array means no bars at all and the
  // plain progress track shows through — no error state, no empty box.
  // Hovering the bars names whoever held the floor there; a speaker picked in the
  // strip dims every other voice's bars.
  import SpeakerMarkGlyph from "./SpeakerMark.svelte";
  import {
    formatPlayerTime,
    type SpeakerMark,
    type StripSpeaker,
    type WaveBar,
  } from "./audio-drawer-view";

  interface Props {
    bars: WaveBar[];
    currentTime: number;
    duration: number;
    valueText: string;
    /** Half height, no playhead dot — the peek drawer's variant. */
    compact?: boolean;
    speakers?: StripSpeaker[];
    marks?: Map<number, SpeakerMark>;
    selectedClusterId?: number | null;
    oninput: (event: Event) => void;
    onchange: (event: Event) => void;
  }

  let {
    bars,
    currentTime,
    duration,
    valueText,
    compact = false,
    speakers = [],
    marks = new Map(),
    selectedClusterId = null,
    oninput,
    onchange,
  }: Props = $props();

  const playable = $derived(duration > 0);
  const progress = $derived(playable ? Math.min(100, (currentTime / duration) * 100) : 0);
  const currentMs = $derived(currentTime * 1000);

  let width = $state(0);
  let hoverX = $state<number | null>(null);
  const hover = $derived.by(() => {
    if (hoverX == null || !(width > 0) || bars.length === 0) return null;
    const x = Math.min(width, Math.max(0, hoverX));
    const bar = bars[Math.min(bars.length - 1, Math.floor((x / width) * bars.length))];
    const speaker = speakers.find((s) => s.clusterId === bar.clusterId) ?? null;
    return { x, speaker, time: formatPlayerTime((x / width) * duration) };
  });
</script>

<!-- The hover tooltip is decoration over the real control (the range input). -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="wave"
  class:wave--bars={bars.length > 0}
  class:wave--compact={compact}
  class:wave--sel={selectedClusterId != null}
  bind:clientWidth={width}
  onpointermove={(event) => (hoverX = event.clientX - event.currentTarget.getBoundingClientRect().left)}
  onpointerleave={() => (hoverX = null)}
>
  {#if bars.length > 0}
    <div class="wave__bars" aria-hidden="true">
      {#each bars as bar, i (i)}
        <span
          class="wave__bar"
          class:is-played={bar.atMs <= currentMs}
          class:is-silent={bar.colorVar == null}
          class:is-dim={selectedClusterId != null && bar.clusterId !== selectedClusterId}
          style="height: {bar.heightPct}%; {bar.colorVar
            ? `--bar: var(${bar.colorVar});`
            : ''}"
        ></span>
      {/each}
    </div>
  {/if}
  <input
    type="range"
    class="wave__range"
    min="0"
    max={playable ? duration : 0}
    step="0.05"
    value={currentTime}
    disabled={!playable}
    {oninput}
    {onchange}
    aria-label="Seek within segment"
    aria-valuemin={0}
    aria-valuemax={playable ? duration : 0}
    aria-valuenow={currentTime}
    aria-valuetext={valueText}
    style:--audio-progress={`${progress}%`}
  />
  {#if bars.length > 0}
    <div class="wave__head" style="left: {progress}%" aria-hidden="true"></div>
  {/if}
  {#if hover}
    <span class="wave__tip" style="left: {hover.x}px" aria-hidden="true">
      {#if hover.speaker}
        <SpeakerMarkGlyph mark={marks.get(hover.speaker.clusterId)} ghosted={hover.speaker.unnamed} />
        {hover.speaker.name}
      {:else}
        no speech
      {/if}
      <span class="wave__tip-time">{hover.time}</span>
    </span>
  {/if}
</div>

<style>
  .wave {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    align-items: center;
    height: 18px;
  }

  .wave--bars {
    height: 30px;
    border-radius: 4px;
    padding: 0 1px;
  }

  .wave--bars.wave--compact {
    height: 20px;
  }

  .wave__bars {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    gap: 1px;
    pointer-events: none;
    overflow: hidden;
    border-radius: 4px;
  }

  .wave__bar {
    flex: 1 1 0;
    min-width: 0;
    border-radius: 1px;
    background: var(--bar, var(--app-text-faint));
    opacity: 0.42;
    transition: opacity 90ms linear;
  }

  .wave__bar.is-played {
    opacity: 1;
  }

  .wave__bar.is-silent {
    opacity: 0.22;
  }

  .wave--sel .wave__bar:not(.is-dim):not(.is-played) {
    opacity: 0.6;
  }

  .wave__bar.is-dim {
    opacity: 0.1;
  }

  .wave__tip {
    position: absolute;
    bottom: calc(100% + 8px);
    z-index: 5;
    translate: -50% 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border-radius: var(--r-sm);
    border: 1px solid var(--app-overlay-border);
    background: var(--app-overlay-bg-strong);
    box-shadow: var(--app-shadow-popover);
    font: 500 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
    pointer-events: none;
  }

  .wave__tip-time {
    font-family: var(--font-mono);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }

  /* ── the real control ────────────────────────────────────────────────────── */
  .wave__range {
    flex: 1 1 auto;
    appearance: none;
    -webkit-appearance: none;
    width: 100%;
    height: 18px;
    margin: 0;
    background: transparent;
    cursor: pointer;
    color: var(--app-status-running-fg);
  }

  .wave__range:disabled {
    cursor: not-allowed;
    opacity: var(--app-disabled-opacity);
  }

  .wave__range::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--app-record-glyph-start) 0%,
      var(--app-record-glyph-start) var(--audio-progress, 0%),
      var(--app-surface-hover) var(--audio-progress, 0%),
      var(--app-surface-hover) 100%
    );
  }

  .wave__range::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: var(--app-surface-hover);
  }

  .wave__range::-moz-range-progress {
    height: 4px;
    border-radius: 2px;
    background: var(--app-record-glyph-start);
  }

  .wave__range::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-status-running-fg);
    border: 2px solid var(--app-surface-raised);
    margin-top: -3px;
    transition: transform 0.12s, box-shadow 0.12s;
  }

  .wave__range::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-status-running-fg);
    border: 2px solid var(--app-surface-raised);
    transition: transform 0.12s, box-shadow 0.12s;
  }

  .wave__range:hover::-webkit-slider-thumb,
  .wave__range:focus-visible::-webkit-slider-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--app-record-glyph-start) 18%, transparent);
  }

  .wave__range:hover::-moz-range-thumb,
  .wave__range:focus-visible::-moz-range-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--app-record-glyph-start) 18%, transparent);
  }

  .wave__range:focus-visible {
    outline: none;
  }

  /* With bars drawn, the native track would double-draw the progress, so the
     input goes transparent and only stays as the (still focusable) hit area. */
  .wave--bars .wave__range {
    position: absolute;
    inset: 0;
    height: 100%;
    opacity: 0;
  }

  .wave--bars .wave__range::-webkit-slider-thumb {
    width: 14px;
    height: 100%;
    margin-top: 0;
  }

  .wave--bars:has(.wave__range:focus-visible) {
    box-shadow: var(--app-ring);
  }

  .wave__head {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--app-accent-strong, var(--app-accent));
    pointer-events: none;
    box-shadow: 0 0 0 1px var(--app-accent-glow);
  }

  .wave--compact .wave__head::after {
    display: none;
  }

  .wave__head::after {
    content: "";
    position: absolute;
    top: -2px;
    left: -3px;
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--app-accent-strong, var(--app-accent));
  }

  @media (prefers-reduced-motion: reduce) {
    .wave__bar {
      transition: none;
    }
  }
</style>
