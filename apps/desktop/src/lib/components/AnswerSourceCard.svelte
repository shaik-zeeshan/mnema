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
    url = null,
    onselect,
    onopenurl,
  }: {
    kind: "frame" | "audio";
    appName?: string | null;
    windowTitle?: string | null;
    startedAt: string;
    endedAt: string;
    sourceKind?: "microphone" | "system" | null;
    thumbnailUrl?: string | null;
    /** Guarded host+path of the captured page (frame sources only), or null. The
     *  raw URL never reaches the UI; only the host is shown as the control label. */
    url?: string | null;
    onselect: () => void;
    /** Open the captured page in the browser (frame sources with a `url`). */
    onopenurl?: () => void;
  } = $props();

  // Label the open control with the host only (the substring before the first
  // "/" of the guarded host+path); the full guarded form goes in the tooltip.
  let openHost = $derived(kind === "frame" && url ? url.split("/")[0] : "");

  function handleOpen(event: MouseEvent): void {
    // Stop the surrounding card-select button from also firing.
    event.stopPropagation();
    onopenurl?.();
  }

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

<!-- The card root is itself the <button> select target, so the "open in browser"
     control cannot nest inside it (invalid HTML). This positioned wrapper makes
     the card-button and the open-button DOM siblings. -->
<div class="source-card-wrap">
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
{#if openHost}
  <!-- Keyboard-reachable as its own tab stop: this card lives in a plain
       horizontal source strip (role="presentation", no roving-focus model), so a
       tab stop here doesn't conflict with anything — it just makes the open
       action reachable without a mouse. Tabbing onto it reveals it (the
       :focus-within reveal + :focus-visible ring below are now genuinely live). -->
  <button
    type="button"
    class="source-card__open"
    title={`Open ${url} in browser`}
    aria-label={`Open ${openHost} in browser`}
    onclick={handleOpen}
  >
    <svg class="source-card__open-glyph" width="11" height="11" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M5.5 2.5H2.5v9h9v-3" />
      <path d="M8 2.5h3.5V6" />
      <path d="M7 7l4.5-4.5" />
    </svg>
    <span class="source-card__open-host">{openHost}</span>
  </button>
{/if}
</div>

<style>
  /* The card root is the select <button>, so the open control can't nest inside
     it. This positioned wrapper is the fixed-width flex tile in the scrolling
     strip; the card-button and the open-button are its DOM siblings. */
  .source-card-wrap {
    position: relative;
    flex: 0 0 auto;
    width: 208px;
  }

  /* Horizontal answer-source card: a compact, fixed-width tile meant to sit in a
     horizontally-scrolling strip. Thumbnail on top, metadata stacked below. */
  .source-card {
    width: 100%;
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

  /* A quiet secondary affordance: muted host chip with an external-link glyph,
     overlaid in the tile's top-right corner. It only saturates to the accent on
     hover/focus so it never competes with the card's own hover/focus state. */
  .source-card__open {
    position: absolute;
    top: 13px;
    right: 13px;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: calc(100% - 26px);
    padding: 1px 6px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    line-height: 1.6;
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease,
      background 0.12s ease;
  }

  /* Reveal the control on tile hover, or whenever focus is within the tile — the
     open button is a real tab stop (see markup), so tabbing onto it brings it into
     view here without cluttering resting cards. */
  .source-card-wrap:hover .source-card__open,
  .source-card-wrap:focus-within .source-card__open {
    opacity: 1;
  }

  .source-card__open:hover,
  .source-card__open:focus-visible {
    outline: none;
    opacity: 1;
    border-color: var(--app-accent-border);
    background: var(--app-surface-raised);
    color: var(--app-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-accent) 12%, transparent);
  }

  .source-card__open-glyph {
    flex: 0 0 auto;
  }

  .source-card__open-host {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .source-card {
      transition: none;
    }

    .source-card__open {
      transition: none;
    }
  }
</style>
