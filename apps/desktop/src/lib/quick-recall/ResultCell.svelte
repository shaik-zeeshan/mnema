<!-- Quick Look grid cell (frame 08): a fixed 349×196 16:9 media frame — naked
     image, no card chrome — with its caption directly on the window below.
     Screen results show the frame thumbnail; audio results show a
     source-tinted waveform tile with a duration badge, the transcript quote
     carried in the caption. Selection is the accent ring; hover raises a
     hairline + the ⏎ affordance badge. Click = open (the Timeline handoff). -->
<script lang="ts">
  import type {
    FrameSearchResultDto,
    AudioSearchResultDto,
  } from "$lib/types/app-infra";
  import { parseSearchSnippet } from "$lib/search-snippet";
  import { parseToolDate } from "./query-tokens";
  import { cellTime, cellDuration, type GridItem } from "./grid";

  let {
    item,
    thumbnailUrl = null,
    selected = false,
    id = undefined,
    onopen,
  }: {
    item: GridItem;
    thumbnailUrl?: string | null;
    selected?: boolean;
    id?: string | undefined;
    onopen: () => void;
  } = $props();

  let frame = $derived<FrameSearchResultDto | null>(
    item.kind === "frame" ? item.frame : null,
  );
  let audio = $derived<AudioSearchResultDto | null>(
    item.kind === "audio" ? item.audio : null,
  );

  // Fade the thumbnail in once decoded (no layout shift; the dark frame box is
  // always reserved). Reset when the source changes.
  let imgLoaded = $state(false);
  $effect(() => {
    thumbnailUrl;
    imgLoaded = false;
  });

  function groupDurationMs(f: FrameSearchResultDto): number {
    const start = parseToolDate(f.groupStartAt)?.getTime();
    const end = parseToolDate(f.groupEndAt)?.getTime();
    return start !== undefined && end !== undefined ? end - start : NaN;
  }

  // Deterministic waveform bars for the audio tile (Lehmer PRNG seeded off the
  // group key, matching the list card's pattern), with a highlight cluster.
  const WAVE_BARS = 40;
  function waveBars(key: string): { h: number; on: boolean }[] {
    let s = 0;
    for (let i = 0; i < key.length; i++) s = (s * 31 + key.charCodeAt(i)) >>> 0;
    s = (s % 2147483646) + 1;
    const at = 4 + (s % (WAVE_BARS - 8));
    const bars = [];
    for (let i = 0; i < WAVE_BARS; i++) {
      s = (s * 16807) % 2147483647;
      bars.push({ h: 18 + ((s % 1000) / 1000) * 72, on: Math.abs(i - at) <= 2 });
    }
    return bars;
  }
  let audioWave = $derived(audio ? waveBars(audio.groupKey) : []);
</script>

{#snippet marked(snippet: string)}
  {#each parseSearchSnippet(snippet) as segment}{#if segment.marked}<mark
        >{segment.text}</mark
      >{:else}{segment.text}{/if}{/each}
{/snippet}

<button
  class="qcell"
  class:qcell--sel={selected}
  {id}
  role="option"
  aria-selected={selected}
  tabindex="-1"
  onclick={onopen}
>
  <span class="qcell__f" class:qcell__f--audio={audio !== null}>
    {#if frame}
      <svg
        class="qcell__glyph"
        width="24"
        height="24"
        viewBox="0 0 14 14"
        fill="none"
        stroke="currentColor"
        stroke-width="1.2"
        stroke-linecap="round"
        aria-hidden="true"
      >
        <rect x="1.5" y="2" width="11" height="8" rx="1.5" />
        <path d="M4 12h6" />
        <path d="M7 10v2" />
      </svg>
      {#if thumbnailUrl}
        <img
          class="qcell__img"
          class:qcell__img--loaded={imgLoaded}
          src={thumbnailUrl}
          alt=""
          loading="lazy"
          onload={() => (imgLoaded = true)}
        />
      {/if}
    {:else if audio}
      <span
        class="qcell__wave"
        class:qcell__wave--mic={audio.sourceKind === "microphone"}
        class:qcell__wave--sys={audio.sourceKind !== "microphone"}
        aria-hidden="true"
      >
        <svg viewBox={`0 0 ${audioWave.length * 7} 100`} preserveAspectRatio="none">
          {#each audioWave as bar, i (i)}
            <rect
              class={bar.on ? "wb-on" : "wb"}
              x={i * 7}
              y={(100 - bar.h) / 2}
              width="4"
              height={bar.h}
              rx="2"
            />
          {/each}
        </svg>
      </span>
      <span class="qcell__dur"
        >{cellDuration(Math.max(0, audio.spanEndMs - audio.spanStartMs))}</span
      >
    {/if}
    <span class="qcell__aff" aria-hidden="true">⏎ OPEN</span>
  </span>
  <span class="qcell__cap">
    {#if frame}
      <span class="qcell__l1">
        <span class="qcell__app">{frame.appName ?? "Screen"}</span>
        <span class="qcell__ttl">
          {#if frame.windowTitle}{frame.windowTitle}{:else}{@render marked(
              frame.snippet,
            )}{/if}
        </span>
      </span>
      <span class="qcell__l2">
        <span>{cellTime(frame.groupStartAt)}</span>
        <span>·</span>
        <span>{cellDuration(groupDurationMs(frame))}</span>
        {#if frame.matchCount > 1}
          <span>·</span>
          <span>{frame.matchCount} hits</span>
        {:else if frame.url}
          <span>·</span>
          <span class="qcell__trunc">{frame.url}</span>
        {/if}
        {#if frame.foundByMeaning}
          <span>·</span>
          <span class="qcell__tag">meaning</span>
        {/if}
        {#if frame.hasSecretRedactions}
          <span>·</span>
          <span class="qcell__tag qcell__tag--warn">redacted</span>
        {/if}
      </span>
    {:else if audio}
      <span class="qcell__l1">
        <span class="qcell__app"
          >{audio.sourceKind === "microphone" ? "Mic" : "Sys audio"}</span
        >
        <span class="qcell__ttl qcell__ttl--quote"
          >“{@render marked(audio.snippet)}”</span
        >
      </span>
      <span class="qcell__l2">
        <span>{cellTime(audio.absoluteStartAt)}</span>
        {#if audio.matchCount > 1}
          <span>·</span>
          <span>{audio.matchCount} adjacent</span>
        {/if}
        {#if audio.foundByMeaning}
          <span>·</span>
          <span class="qcell__tag">meaning</span>
        {/if}
        {#if audio.hasSecretRedactions}
          <span>·</span>
          <span class="qcell__tag qcell__tag--warn">redacted</span>
        {/if}
      </span>
    {/if}
  </span>
</button>

<style>
  /* Fixed cell: the frame is exactly 349×196 (16:9); the caption hangs below.
     The button is a naked grid cell — no card chrome (frame 08's rule). */
  .qcell {
    display: block;
    width: 349px;
    padding: 0;
    border: none;
    background: none;
    text-align: left;
    font: inherit;
    color: var(--app-text);
    cursor: pointer;
  }

  .qcell:focus-visible {
    outline: none;
  }

  .qcell__f {
    position: relative;
    display: block;
    width: 349px;
    height: 196px;
    border-radius: var(--r-md);
    overflow: hidden;
    /* Screenshot backing stays dark in both themes. */
    background: #0b0b10;
    color: #6a6a74;
  }

  .qcell__glyph {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
  }

  .qcell__img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
  }

  .qcell__img--loaded {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    .qcell__img {
      transition: opacity 0.18s ease;
    }
  }

  .qcell--sel .qcell__f {
    box-shadow: 0 0 0 3px var(--app-accent);
  }

  .qcell:hover:not(.qcell--sel) .qcell__f {
    box-shadow: 0 0 0 var(--hairline) var(--app-border-hover);
  }

  /* Audio tile: source-tinted waveform filling the frame. */
  .qcell__f--audio {
    color: inherit;
  }

  .qcell__wave {
    position: absolute;
    inset: 0;
    display: block;
  }

  .qcell__wave svg {
    position: absolute;
    inset: 16% 8%;
    width: 84%;
    height: 68%;
  }

  .qcell__wave--mic {
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }

  .qcell__wave--sys {
    background: var(--app-source-sysaudio-bg);
    color: var(--app-source-sysaudio);
  }

  .qcell__wave .wb {
    fill: color-mix(in srgb, currentColor 45%, transparent);
  }

  .qcell__wave .wb-on {
    fill: currentColor;
  }

  /* Duration badge (audio), bottom-left on the image — dark plate so it reads
     over any content (system.css gap 1: text over an image gets a plate). */
  .qcell__dur {
    position: absolute;
    z-index: 3;
    left: var(--s-6);
    bottom: var(--s-6);
    height: var(--o-badge);
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    display: inline-flex;
    align-items: center;
    background: rgba(10, 10, 14, 0.78);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: #f2f2f5;
  }

  /* Hover/selection affordance badge, bottom-right on the image. */
  .qcell__aff {
    position: absolute;
    z-index: 3;
    right: var(--s-6);
    bottom: var(--s-6);
    height: var(--o-badge);
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    display: none;
    align-items: center;
    background: rgba(10, 10, 14, 0.78);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    color: #f2f2f5;
  }

  .qcell:hover .qcell__aff,
  .qcell--sel .qcell__aff {
    display: inline-flex;
  }

  /* Caption directly on the window. */
  .qcell__cap {
    display: block;
    margin-top: var(--s-6);
  }

  .qcell__l1 {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    min-width: 0;
  }

  .qcell__app {
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  .qcell__ttl {
    font: var(--w-medium) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .qcell__l2 {
    margin-top: 2px;
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-mono);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
  }

  .qcell__trunc {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .qcell__tag {
    color: var(--app-accent);
  }

  .qcell__tag--warn {
    color: var(--app-warn);
  }

  .qcell :global(mark) {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-radius: 2px;
    padding: 0 2px;
  }
</style>
