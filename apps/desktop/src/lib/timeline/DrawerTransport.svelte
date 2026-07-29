<script lang="ts">
  // Play/pause, the two time readouts, and the waveform scrubber between them.
  import WaveformScrubber from "./WaveformScrubber.svelte";
  import { formatPlayerTime, type WaveBar } from "./audio-drawer-view";

  interface Props {
    isPlaying: boolean;
    currentTime: number;
    duration: number;
    playable: boolean;
    mediaLoading: boolean;
    bars: WaveBar[];
    compact: boolean;
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
    compact,
    onToggle,
    onScrubInput,
    onScrubChange,
  }: Props = $props();
</script>

<footer class="transport">
  <button
    type="button"
    class="playbtn"
    onclick={onToggle}
    disabled={!playable}
    aria-label={isPlaying ? "Pause" : "Play"}
    aria-pressed={isPlaying}
  >
    {#if isPlaying}
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
        <rect x="3.5" y="2.5" width="3" height="11" rx="0.5" fill="currentColor" />
        <rect x="9.5" y="2.5" width="3" height="11" rx="0.5" fill="currentColor" />
      </svg>
    {:else}
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
        <path d="M4.5 2.5 L13 8 L4.5 13.5 Z" fill="currentColor" />
      </svg>
    {/if}
  </button>
  <span class="time">{formatPlayerTime(currentTime)}</span>
  {#if mediaLoading}
    <span class="transport__loading" role="status" aria-live="polite" aria-busy="true">
      <span class="spinner" aria-hidden="true"></span> loading audio segment…
    </span>
  {:else}
    <WaveformScrubber
      {bars}
      {currentTime}
      {duration}
      {compact}
      valueText={`${formatPlayerTime(currentTime)} of ${formatPlayerTime(duration)}`}
      oninput={onScrubInput}
      onchange={onScrubChange}
    />
  {/if}
  <span class="time">{formatPlayerTime(duration)}</span>
</footer>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-subtle, var(--app-surface));
  }

  .playbtn {
    width: 26px;
    height: 26px;
    flex: none;
    display: grid;
    place-items: center;
    border: 1px solid var(--app-status-running-border);
    border-radius: 999px;
    background: color-mix(in srgb, var(--app-record-glyph-start) 10%, transparent);
    color: var(--app-status-running-fg);
    cursor: pointer;
    transition:
      background 0.12s,
      transform 0.08s;
  }

  .playbtn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--app-record-glyph-start) 18%, transparent);
  }

  .playbtn:active:not(:disabled) {
    transform: scale(0.96);
  }

  .playbtn:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .playbtn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring-danger, var(--app-ring));
  }

  .time {
    min-width: 5ch;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }

  .transport__loading {
    flex: 1 1 auto;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-muted);
  }

  .spinner {
    width: 10px;
    height: 10px;
    flex: none;
    display: inline-block;
    border-radius: 999px;
    border: 1.5px solid color-mix(in srgb, var(--app-text-muted) 30%, transparent);
    border-top-color: currentColor;
    animation: drawer-transport-spin 700ms linear infinite;
  }

  @keyframes drawer-transport-spin {
    to {
      transform: rotate(1turn);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .playbtn {
      transition: none;
    }

    .spinner {
      animation: none;
    }
  }
</style>
