<script lang="ts">
  import { formatTimestampCompact } from "$lib/format-time";
  import AudioWaveform from "$lib/components/AudioWaveform.svelte";

  let {
    kind,
    appName = null,
    windowTitle = null,
    startedAt,
    endedAt,
    sourceKind = null,
    thumbnailUrl = null,
    onselect,
  }: {
    kind: "frame" | "audio";
    appName?: string | null;
    windowTitle?: string | null;
    startedAt: string;
    endedAt: string;
    sourceKind?: "microphone" | "system" | null;
    thumbnailUrl?: string | null;
    onselect: () => void;
  } = $props();

  // Fade the thumbnail image in once it decodes so it eases over the reserved
  // placeholder box instead of hard-popping (and so no layout shift occurs).
  // Reset whenever the source changes so a recycled card re-fades its new image.
  let imgLoaded = $state(false);
  $effect(() => {
    thumbnailUrl;
    imgLoaded = false;
  });

  // Anything that is not an explicit microphone source reads as system audio,
  // matching the mic/sysaudio split used by SearchResultCard and the timeline.
  let isMic = $derived(sourceKind === "microphone");
</script>

<button
  class="source-card"
  class:source-card--frame={kind === "frame"}
  class:source-card--audio={kind === "audio"}
  type="button"
  tabindex="-1"
  aria-label={kind === "frame"
    ? `Screen capture from ${appName ?? "Unknown app"}`
    : `${isMic ? "Microphone" : "System audio"} capture`}
  onclick={onselect}
>
  {#if kind === "frame"}
    <div class="source-card__thumb">
      <svg class="source-card__thumb-glyph" width="20" height="20" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" aria-hidden="true">
        <rect x="1.5" y="2" width="11" height="8" rx="1.5" />
        <path d="M4 12h6" />
        <path d="M7 10v2" />
      </svg>
      {#if thumbnailUrl}
        <img
          class="source-card__thumb-img"
          class:source-card__thumb-img--loaded={imgLoaded}
          src={thumbnailUrl}
          alt=""
          loading="lazy"
          onload={() => (imgLoaded = true)}
        />
      {/if}
    </div>
  {:else}
    <div
      class="source-card__thumb source-card__thumb--audio"
      class:source-card__thumb--mic={isMic}
      class:source-card__thumb--sysaudio={!isMic}
    >
      <AudioWaveform class="source-card__wave" widthPercent={48} />
    </div>
  {/if}

  <div class="source-card__body">
    <div class="source-card__line">
      {#if kind === "frame"}
        <span class="source-card__app">{appName ?? "Unknown app"}</span>
      {:else}
        <span
          class="source-card__source"
          class:source-card__source--mic={isMic}
          class:source-card__source--sysaudio={!isMic}
        >{isMic ? "microphone" : "system audio"}</span>
      {/if}
    </div>
    {#if windowTitle}
      <span class="source-card__sub" title={windowTitle}>{windowTitle}</span>
    {/if}
    <span class="source-card__time">{formatTimestampCompact(startedAt)}</span>
  </div>
</button>

<style>
  /* Horizontal answer-source card: a compact, fixed-width tile meant to sit in a
     horizontally-scrolling strip. Thumbnail on top, metadata stacked below. */
  .source-card {
    flex: 0 0 auto;
    width: 208px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    overflow: hidden;
    text-align: left;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface-raised);
    color: var(--app-text);
    font: inherit;
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }

  .source-card:hover {
    border-color: var(--app-accent-border);
  }

  .source-card:focus-visible {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow:
      0 0 0 1px var(--app-accent-border),
      0 0 0 4px color-mix(in srgb, var(--app-accent) 12%, transparent);
  }

  .source-card__thumb {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 10;
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    overflow: hidden;
    background: var(--app-bg);
    color: var(--app-text-faint);
  }

  /* The image sits above the always-present glyph placeholder and fades in once
     decoded, so the reserved box never flashes from a void and never reflows. */
  .source-card__thumb-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
  }

  .source-card__thumb-img--loaded {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    .source-card__thumb-img {
      transition: opacity 0.18s ease;
    }
  }

  .source-card__thumb-glyph {
    color: var(--app-text-faint);
  }

  /* Audio rail: a source-colored waveform tile. mic = green, system = olive,
     matching the capture-source tokens used across the timeline. */
  .source-card__thumb--audio {
    background: var(--app-surface-raised);
    color: var(--app-text-subtle);
  }

  .source-card__thumb--mic {
    border-color: var(--app-source-mic-border);
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }

  .source-card__thumb--sysaudio {
    border-color: var(--app-source-sysaudio-border);
    background: var(--app-source-sysaudio-bg);
    color: var(--app-source-sysaudio);
  }

  .source-card__body {
    min-width: 0;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 3px;
    overflow: hidden;
  }

  .source-card__line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .source-card__app {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--app-text-strong);
    font-size: 12px;
    font-weight: 600;
  }

  .source-card__source {
    flex: 0 0 auto;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--app-text-muted);
  }

  .source-card__source--mic {
    color: var(--app-source-mic);
  }

  .source-card__source--sysaudio {
    color: var(--app-source-sysaudio);
  }

  .source-card__sub {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--app-text-subtle);
    font-size: 11px;
  }

  .source-card__time {
    color: var(--app-text-subtle);
    font-size: 10px;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .source-card {
      transition: none;
    }
  }
</style>
