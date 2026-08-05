<!-- Quick Recall search result TILE — the round-4 3-up grid cell (349 wide,
     196-tall media). One anatomy for both modalities: a header row (app / source
     eyebrow left, time right), one title line, one marked caption line, then the
     media bleeding to the tile's bottom radius (screenshot for screen results,
     source-coloured waveform for audio) with the match/meaning/redacted badges
     floating over its bottom edge. The list/detail split is gone with the grid,
     so the tile carries the accessories the detail pane used to duplicate. -->
<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import type {
    FrameSearchResultDto,
    AudioSearchResultDto,
  } from "$lib/types/app-infra";
  import { parseSearchSnippet } from "$lib/search-snippet";
  import { formatRelativeTime } from "$lib/format-time";
  import { appIcons } from "./app-icons.svelte";

  let {
    kind,
    frame = null,
    audio = null,
    thumbnailUrl = null,
    selected = false,
    id = undefined,
    onselect,
  }: {
    kind: "frame" | "audio";
    frame?: FrameSearchResultDto | null;
    audio?: AudioSearchResultDto | null;
    thumbnailUrl?: string | null;
    selected?: boolean;
    id?: string | undefined;
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

  function formatDuration(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return "—";
    const total = Math.round(seconds);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  // Deterministic waveform bars for the audio tile, ported from the mockup's
  // renderWave (64 bars, 16807 Lehmer PRNG, a ±2-bar highlight cluster).
  // ponytail: the highlight position is decorative — search results carry no
  // in-span match offset.
  const WAVE_BARS = 64;

  function waveBars(
    key: string,
  ): { x: number; y: number; h: number; on: boolean }[] {
    let s = 0;
    for (let i = 0; i < key.length; i++) s = (s * 31 + key.charCodeAt(i)) >>> 0;
    s = (s % 2147483646) + 1; // Lehmer seed must be in [1, 2147483646]
    const at = 4 + (s % (WAVE_BARS - 8)); // keep the cluster off the edges
    const bars = [];
    for (let i = 0; i < WAVE_BARS; i++) {
      s = (s * 16807) % 2147483647;
      const h = 4 + ((s % 1000) / 1000) * 14;
      bars.push({ x: i * 7, y: (20 - h) / 2, h, on: Math.abs(i - at) <= 2 });
    }
    return bars;
  }

  let audioWave = $derived(
    kind === "audio" && audio ? waveBars(audio.groupKey) : [],
  );
</script>

<!-- The badge strip is identical for both modalities, so it lives in one
     snippet taking the three flags the two DTOs share. -->
{#snippet badges(
  matchCount: number,
  matchWord: string,
  foundByMeaning: boolean,
  redacted: boolean,
)}
  {#if matchCount > 1 || foundByMeaning || redacted}
    <span class="search-card__badges">
      {#if matchCount > 1}
        <span class="search-card__badge">{matchCount} {matchWord}</span>
      {/if}
      {#if foundByMeaning}
        <span class="search-card__badge search-card__badge--meaning">meaning</span>
      {/if}
      {#if redacted}
        <span class="search-card__badge search-card__badge--redacted">redacted</span>
      {/if}
    </span>
  {/if}
{/snippet}

{#if kind === "frame" && frame}
  <button
    class="search-card"
    class:search-card--selected={selected}
    {id}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
  >
    <span class="search-card__head">
      {#if appIcons.src(frame.appBundleId ?? frame.appName) !== null}
        <img
          class="search-card__appicon"
          src={appIcons.src(frame.appBundleId ?? frame.appName)}
          alt=""
          aria-hidden="true"
        />
      {/if}
      <span class="search-card__app">{frame.appName ?? "Unknown app"}</span>
      <span class="search-card__time">{formatRelativeTime(frame.groupEndAt)}</span>
    </span>
    <span class="search-card__title" use:tip={frame.windowTitle ?? undefined}>
      {frame.windowTitle ?? frame.appName ?? "Screen"}
    </span>
    <span class="search-card__media">
      <svg
        class="search-card__media-glyph"
        width="22"
        height="22"
        viewBox="0 0 14 14"
        fill="none"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
        aria-hidden="true"
      >
        <rect x="1.5" y="2" width="11" height="8" rx="1.5" />
        <path d="M4 12h6" />
        <path d="M7 10v2" />
      </svg>
      {#if thumbnailUrl}
        <img
          class="search-card__thumb-img"
          class:search-card__thumb-img--loaded={imgLoaded}
          src={thumbnailUrl}
          alt=""
          loading="lazy"
          onload={() => (imgLoaded = true)}
        />
      {/if}
      <span class="search-card__over">
        {@render badges(
          frame.matchCount,
          "matches",
          frame.foundByMeaning,
          frame.hasSecretRedactions,
        )}
        <span class="search-card__aff" aria-hidden="true">⏎ open</span>
      </span>
      <!-- Text over an image gets an OPAQUE plate, never a soft scrim: a scrim
           only dims what is under it, so the contrast is whatever the pixels
           happen to be. -->
      <span class="search-card__plate">
        {#each parseSearchSnippet(frame.snippet) as segment}{#if segment.marked}<mark
              >{segment.text}</mark
            >{:else}{segment.text}{/if}{/each}
      </span>
    </span>
  </button>
{/if}

{#if kind === "audio" && audio}
  <button
    class="search-card"
    class:search-card--selected={selected}
    {id}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
  >
    <span class="search-card__head">
      <span class="search-card__app"
        >{audio.sourceKind === "microphone" ? "Microphone" : "System audio"}</span
      >
      <span class="search-card__time"
        >{formatRelativeTime(audio.absoluteStartAt)}</span
      >
    </span>
    <span class="search-card__title">
      {audio.sourceKind === "microphone" ? "Microphone" : "System audio"} · {formatDuration(
        Math.max(0, (audio.spanEndMs - audio.spanStartMs) / 1000),
      )} of speech
    </span>
    <span
      class="search-card__media search-card__media--wave"
      class:search-card__media--mic={audio.sourceKind === "microphone"}
      class:search-card__media--sys={audio.sourceKind !== "microphone"}
    >
      <svg
        class="search-card__wave"
        viewBox="0 0 448 20"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        {#each audioWave as bar (bar.x)}
          <rect
            class={bar.on ? "wb-on" : "wb"}
            x={bar.x}
            y={bar.y}
            width="4"
            height={bar.h}
            rx="1"
          />
        {/each}
      </svg>
      <span class="search-card__over">
        {@render badges(
          audio.matchCount,
          "adjacent",
          audio.foundByMeaning,
          audio.hasSecretRedactions,
        )}
        <span class="search-card__aff" aria-hidden="true">⏎ open</span>
      </span>
      <!-- The spoken words ARE the audio result, so they take the plate. -->
      <span class="search-card__plate">
        “{#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark
              >{segment.text}</mark
            >{:else}{segment.text}{/if}{/each}”
      </span>
    </span>
  </button>
{/if}

<style>
  /* One grid cell — and a TILE like every other tile in the app: a grouped
     borderless fill, a constant 18px header row, then a payload that bleeds to
     the tile edge and clips on the radius. Never a bordered card. */
  .search-card {
    display: flex;
    flex-direction: column;
    width: 100%;
    min-width: 0;
    padding: var(--tile-pad) var(--tile-pad) 0;
    overflow: hidden;
    text-align: left;
    border: 0;
    border-radius: var(--tile-r);
    background: var(--tile-fill);
    color: var(--app-text);
    font: inherit;
    cursor: default;
  }

  @media (prefers-reduced-motion: no-preference) {
    .search-card {
      transition:
        background-color var(--dur-quick) var(--ease),
        box-shadow var(--dur-quick) var(--ease);
    }
  }

  .search-card:hover {
    background-color: var(--tile-fill-hover);
  }

  /* Selected is the roving highlight: a 2px accent ring on the tile, the one
     selection idiom on the surface. */
  .search-card:focus-visible,
  .search-card--selected,
  .search-card--selected:hover {
    outline: none;
    box-shadow: 0 0 0 2px var(--app-accent);
  }

  /* The constant tile header row: mono app eyebrow left, mono time right, on
     the one baseline every tile in the app shares. */
  .search-card__head {
    flex: 0 0 var(--tile-hd);
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    min-width: 0;
  }

  .search-card__appicon {
    flex: none;
    width: 13px;
    height: 13px;
    object-fit: contain;
  }

  .search-card__app {
    flex: 1;
    min-width: 0;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-card__time {
    flex: none;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }

  /* One title line — the window title (screen) or the source + duration
     (audio). Never wraps: the tile's height is fixed by the media block. */
  .search-card__title {
    display: block;
    min-width: 0;
    margin-top: var(--s-4);
    font: var(--w-medium) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Match highlight: accent at 26% plus a hairline, so it reads as a mark on
     both a light plate and a dark screenshot. */
  .search-card mark {
    border-radius: 2px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
    color: inherit;
    padding: 0 1px;
  }

  /* 196px of media, bleeding to the tile's left/right/bottom edges so it clips
     on the tile radius. `--media-void` is what a frame that has not decoded
     looks like: a hole in the tile, not a flash. */
  .search-card__media {
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    height: 196px;
    margin: 10px calc(var(--tile-pad) * -1) 0;
    overflow: hidden;
    background: var(--media-void);
    color: var(--app-text-faint);
  }

  .search-card__media-glyph {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
  }

  .search-card__thumb-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top center;
    opacity: 0;
  }

  .search-card__thumb-img--loaded {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    .search-card__thumb-img {
      transition: opacity 0.18s ease;
    }
  }

  /* Audio media: a source-coloured waveform field — deliberately NOT a
     screenshot (the words are the content). mic = green, sys = olive. */
  .search-card__media--wave {
    justify-content: flex-end;
  }

  .search-card__media--mic {
    background: var(--app-src-mic-bg);
    color: var(--app-src-mic);
  }

  .search-card__media--sys {
    background: var(--app-src-sys-bg);
    color: var(--app-src-sys);
  }

  .search-card__wave {
    position: absolute;
    top: 50%;
    left: var(--s-12);
    right: var(--s-12);
    transform: translateY(-50%);
    width: auto;
    height: 56px;
  }

  .search-card__wave .wb {
    fill: color-mix(in srgb, currentColor 45%, transparent);
  }

  .search-card__wave .wb-on {
    fill: currentColor;
  }

  /* The accessory strip floats over the media: badges left, the affordance
     right. It names what ⏎ does on the tile the pointer is over. */
  .search-card__over {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: flex-end;
    gap: var(--s-8);
    padding: var(--s-8);
  }

  .search-card__badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-4);
    min-width: 0;
  }

  .search-card__badge {
    display: inline-flex;
    align-items: center;
    height: var(--o-badge);
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    background: rgba(10, 10, 14, 0.78);
    color: #f2f2f5;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    white-space: nowrap;
  }

  .search-card__badge--meaning {
    color: var(--app-accent);
  }

  .search-card__badge--redacted {
    color: var(--app-warn);
  }

  .search-card__aff {
    margin-left: auto;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    height: var(--o-badge);
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    background: rgba(10, 10, 14, 0.78);
    color: #f2f2f5;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    opacity: 0;
  }

  .search-card:hover .search-card__aff,
  .search-card--selected .search-card__aff {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    .search-card__aff {
      transition: opacity var(--dur-quick) var(--ease);
    }
  }

  /* The matched text, so "why did this hit?" is answered on the tile itself.
     An OPAQUE plate — a soft scrim leaves the contrast up to the screenshot. */
  .search-card__plate {
    position: relative;
    z-index: 2;
    padding: var(--s-6) var(--s-12);
    background: rgba(8, 8, 12, 0.84);
    backdrop-filter: blur(6px);
    font: var(--w-regular) var(--t-meta) / 1.35 var(--app-font-sans);
    color: #f2f2f5;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-card__plate mark {
    color: #f2f2f5;
  }
</style>
