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
        <!-- The match wash, stated on the frame with exactly the ink the caption
             uses for <mark> — one accent, one fact, read twice. -->
        <span class="search-card__badge search-card__badge--match"
          >{matchCount} {matchWord}</span
        >
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
    <span class="search-card__caption">
      {#each parseSearchSnippet(frame.snippet) as segment}{#if segment.marked}<mark
            >{segment.text}</mark
          >{:else}{segment.text}{/if}{/each}
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
      {@render badges(
        frame.matchCount,
        "matches",
        frame.foundByMeaning,
        frame.hasSecretRedactions,
      )}
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
      “{#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark
            >{segment.text}</mark
          >{:else}{segment.text}{/if}{/each}”
    </span>
    <span class="search-card__caption"
      >{formatDuration(
        Math.max(0, (audio.spanEndMs - audio.spanStartMs) / 1000),
      )} of speech</span
    >
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
      {@render badges(
        audio.matchCount,
        "adjacent",
        audio.foundByMeaning,
        audio.hasSecretRedactions,
      )}
    </span>
  </button>
{/if}

<style>
  /* One grid cell — an OPAQUE FRAME PLATE. The material never touches a result:
     the picture and its caption land on --app-surface, and the plate's shadow IS
     the border it replaces (the direction's one exemption to "cards get no
     shadow"; do not add a border back). The frame runs first, full-bleed to the
     tile's top radius, and the caption sits beneath it. */
  .search-card {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    width: 100%;
    min-width: 0;
    padding: var(--s-8) var(--s-12) var(--s-8);
    overflow: hidden;
    text-align: left;
    border: 0;
    border-radius: var(--r-lg);
    background: var(--app-surface);
    box-shadow: var(--sh-tile);
    color: var(--app-text);
    font: inherit;
    cursor: pointer;
  }

  @media (prefers-reduced-motion: no-preference) {
    .search-card {
      transition:
        background var(--dur-quick) var(--ease),
        box-shadow var(--dur-quick) var(--ease);
    }
  }

  .search-card:hover {
    background: var(--app-surface-raised);
    box-shadow: var(--sh-float);
  }

  /* Selected is the roving highlight: the accent ring plus the lift that says
     this plate is the one the keyboard is on. */
  .search-card:focus-visible,
  .search-card--selected,
  .search-card--selected:hover {
    outline: none;
    background: var(--app-surface-raised);
    box-shadow: var(--sh-float), 0 0 0 2px var(--app-accent);
  }

  /* Header: app / source eyebrow left, time hard right. */
  .search-card__head {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    min-width: 0;
  }

  .search-card__appicon {
    flex: none;
    width: 14px;
    height: 14px;
    object-fit: contain;
  }

  .search-card__app {
    flex: 1;
    min-width: 0;
    font-size: var(--t-label);
    line-height: 1;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-card__time {
    flex: none;
    font-size: var(--t-label);
    line-height: 1;
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }

  /* One title line — the window title (screen) or the quoted transcript
     (audio). Never wraps: the tile's height is fixed by the media block. */
  .search-card__title {
    display: block;
    min-width: 0;
    font-size: var(--t-ui);
    line-height: 1.3;
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The matched text, so "why did this hit?" is answered on the tile itself —
     the detail pane that used to carry it is gone with the grid. */
  .search-card__caption {
    display: block;
    min-width: 0;
    font-size: var(--t-meta);
    line-height: 1.3;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ONE accent wash, spent twice: here in the caption and — same mix, same ink —
     on the frame's match badge below, so the eye reads them as one fact. */
  .search-card mark {
    border-radius: 2px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    color: var(--app-text-strong);
    padding: 0 1px;
  }

  /* 196px media, full-bleed to the tile's bottom radius. The backing stays dark
     in both themes (it holds a screenshot); the glyph is a fixed mid-gray
     legible on that backing while the image loads or when no preview exists. */
  .search-card__media {
    position: relative;
    /* The frame leads the plate: 196px of picture, then the caption. */
    order: -1;
    display: grid;
    place-items: center;
    height: 196px;
    margin: calc(var(--s-8) * -1) calc(var(--s-12) * -1) var(--s-4);
    overflow: hidden;
    background: #101014;
    color: #6a6a74;
  }

  .search-card__media-glyph {
    position: relative;
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
  .search-card__media--mic {
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }

  .search-card__media--sys {
    background: var(--app-source-sysaudio-bg);
    color: var(--app-source-sysaudio);
  }

  .search-card__wave {
    width: calc(100% - var(--s-24));
    height: 56px;
  }

  .search-card__wave .wb {
    fill: color-mix(in srgb, currentColor 45%, transparent);
  }

  .search-card__wave .wb-on {
    fill: currentColor;
  }

  /* Accessories float over the media's bottom edge rather than stealing a text
     row — the tile's vertical budget is spent on the picture. */
  .search-card__badges {
    position: absolute;
    left: var(--s-8);
    right: var(--s-8);
    bottom: var(--s-8);
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-4);
  }

  .search-card__badge {
    display: inline-flex;
    align-items: center;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    padding: 3px 6px;
    border-radius: var(--r-sm);
    border: 0;
    background: rgba(12, 12, 16, 0.62);
    -webkit-backdrop-filter: blur(8px);
    backdrop-filter: blur(8px);
    color: #fff;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    white-space: nowrap;
  }

  /* The caption's <mark> wash, restated over the frame. */
  .search-card__badge--match {
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    box-shadow: inset 0 0 0 var(--hairline)
      color-mix(in srgb, var(--app-accent) 55%, transparent);
    color: #fff;
  }

  .search-card__badge--meaning {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }

  .search-card__badge--redacted {
    color: var(--app-warn);
    background: var(--app-warn-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-warn-border);
  }
</style>
