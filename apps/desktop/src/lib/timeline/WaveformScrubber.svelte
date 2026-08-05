<script lang="ts">
  // The scrub bar, told the truth about what is inside it.
  //
  // The interactive element is the SAME real `<input type="range">` the drawer
  // has always had, keeping its aria-valuetext and full keyboard operability;
  // the amplitude bars are decoration layered on top. Bar height is real
  // amplitude from `get_audio_segment_waveform_peaks`; bar hue is whoever held
  // the floor at that moment. An empty peaks array means no bars at all and the
  // plain progress track shows through — no error state, no empty box.
  import type { WaveBar } from "./audio-drawer-view";

  interface Props {
    bars: WaveBar[];
    currentTime: number;
    duration: number;
    valueText: string;
    /** Half height, no playhead dot — the peek drawer's variant. */
    compact?: boolean;
    oninput: (event: Event) => void;
    onchange: (event: Event) => void;
  }

  let { bars, currentTime, duration, valueText, compact = false, oninput, onchange }: Props =
    $props();

  const playable = $derived(duration > 0);
  const progress = $derived(playable ? Math.min(100, (currentTime / duration) * 100) : 0);
  const currentMs = $derived(currentTime * 1000);
</script>

<!-- TACTILE: with real peaks the scrubber is a WELL — the one recess treatment
     every instrument face in this direction shares. Empty peaks keep the plain
     track (no well around nothing). -->
<div
  class="wave"
  class:wave--bars={bars.length > 0}
  class:ti-well={bars.length > 0}
  class:wave--compact={compact}
>
  {#if bars.length > 0}
    <div class="wave__bars" aria-hidden="true">
      {#each bars as bar, i (i)}
        <span
          class="wave__bar"
          class:is-played={bar.atMs <= currentMs}
          class:is-silent={bar.colorVar == null}
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
    height: 34px;
    padding: 0 var(--s-6);
  }

  .wave--bars.wave--compact {
    height: 24px;
  }

  .wave__bars {
    position: absolute;
    inset: 0 var(--s-6);
    display: flex;
    align-items: center;
    gap: 1px;
    pointer-events: none;
    overflow: hidden;
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
    color: var(--app-accent);
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
      var(--app-accent) 0%,
      var(--app-accent) var(--audio-progress, 0%),
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
    background: var(--app-accent);
  }

  .wave__range::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-accent);
    border: 2px solid var(--app-surface-raised);
    margin-top: -3px;
    transition: transform 0.12s, box-shadow 0.12s;
  }

  .wave__range::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-accent);
    border: 2px solid var(--app-surface-raised);
    transition: transform 0.12s, box-shadow 0.12s;
  }

  .wave__range:hover::-webkit-slider-thumb,
  .wave__range:focus-visible::-webkit-slider-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px var(--app-accent-glow);
  }

  .wave__range:hover::-moz-range-thumb,
  .wave__range:focus-visible::-moz-range-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px var(--app-accent-glow);
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

  /* The drawer's playhead — the mockup draws it in recording red, flat. */
  .wave__head {
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: 2px;
    background: var(--app-record);
    pointer-events: none;
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
    background: var(--app-record);
  }

  @media (prefers-reduced-motion: reduce) {
    .wave__bar {
      transition: none;
    }
  }
</style>
