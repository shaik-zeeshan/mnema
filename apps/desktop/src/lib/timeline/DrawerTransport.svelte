<script lang="ts">
  // Play/pause, the two time readouts, and the waveform scrubber between them —
  // plus the peek drawer's floating "open reader" door, which sits just above.
  import IconExpand from "~icons/lucide/maximize-2";
  import WaveformScrubber from "./WaveformScrubber.svelte";
  import {
    formatPlayerTime,
    type SpeakerMark,
    type StripSpeaker,
    type WaveBar,
  } from "./audio-drawer-view";

  interface Props {
    isPlaying: boolean;
    currentTime: number;
    duration: number;
    playable: boolean;
    mediaLoading: boolean;
    bars: WaveBar[];
    speakers: StripSpeaker[];
    marks: Map<number, SpeakerMark>;
    selectedClusterId: number | null;
    compact: boolean;
    /** Show the floating "open reader" button (peek drawer, reader showing). */
    showExpand: boolean;
    onExpand: () => void;
    onToggle: () => void;
    onScrubInput: (event: Event) => void;
    onScrubChange: (event: Event) => void;
  }

  let {
    isPlaying,
    currentTime,
    duration,
    playable,
    mediaLoading,
    bars,
    speakers,
    marks,
    selectedClusterId,
    compact,
    showExpand,
    onExpand,
    onToggle,
    onScrubInput,
    onScrubChange,
  }: Props = $props();
</script>

<footer class="transport">
  <button
    type="button"
    class="mx-btn mx-btn--icon"
    onclick={onToggle}
    disabled={!playable}
    aria-label={isPlaying ? "Pause" : "Play"}
    aria-pressed={isPlaying}
  >
    {#if isPlaying}
      <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
        <rect x="3.5" y="2.5" width="3" height="11" rx="0.5" fill="currentColor" />
        <rect x="9.5" y="2.5" width="3" height="11" rx="0.5" fill="currentColor" />
      </svg>
    {:else}
      <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
        <path d="M4.5 2.5 L13 8 L4.5 13.5 Z" fill="currentColor" />
      </svg>
    {/if}
  </button>
  <span>{formatPlayerTime(currentTime)}</span>
  {#if mediaLoading}
    <span class="transport__loading" role="status" aria-live="polite" aria-busy="true">
      <span class="mx-spin mx-spin--sm" aria-hidden="true"></span> loading audio segment…
    </span>
  {:else}
    <WaveformScrubber
      {bars}
      {speakers}
      {marks}
      {selectedClusterId}
      {currentTime}
      {duration}
      {compact}
      valueText={`${formatPlayerTime(currentTime)} of ${formatPlayerTime(duration)}`}
      oninput={onScrubInput}
      onchange={onScrubChange}
    />
  {/if}
  <span>{formatPlayerTime(duration)}</span>
  {#if showExpand}
    <button type="button" class="mx-btn mx-btn--sm expand-cta" onclick={onExpand}>
      <IconExpand width="13" height="13" aria-hidden="true" />open reader
    </button>
  {/if}
</footer>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: 10px 14px;
    border-top: 1px solid var(--mx-hairline);
    font: 500 var(--text-sm) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }

  .transport__loading {
    flex: 1 1 auto;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  /* Anchored to the drawer (the nearest positioned box), just above this bar. */
  .expand-cta {
    position: absolute;
    right: 18px;
    bottom: 60px;
  }
</style>
