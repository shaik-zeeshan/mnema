<!-- Quick Recall search result CELL — direction 02's `.ss-qcell`: the 16:9 frame
     first, then one identity line (app + title) and one mono meta line. There is
     no card: no border, no fill, no padding — the picture is the object and the
     2px accent ring on the frame is the selection. Everything the old tile's
     third line carried (the matched snippet, the URL, the score) is the
     inspector's job now, which is why the inspector is always on screen. -->
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
    class="search-card ss-qcell"
    class:is-sel={selected}
    {id}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
  >
    <span class="search-card__media ss-qcell__f">
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
    <span class="ss-qcell__l1">
      {#if appIcons.src(frame.appBundleId ?? frame.appName) !== null}
        <img
          class="search-card__appicon"
          src={appIcons.src(frame.appBundleId ?? frame.appName)}
          alt=""
          aria-hidden="true"
        />
      {/if}
      <span class="ss-qcell__app">{frame.appName ?? "Unknown app"}</span>
      <span class="ss-qcell__ttl" use:tip={frame.windowTitle ?? undefined}
        >{frame.windowTitle ?? frame.appName ?? "Screen"}</span
      >
    </span>
    <span class="ss-qcell__l2">
      <span>{formatRelativeTime(frame.groupEndAt)}</span>
      {#if frame.matchCount > 0}
        <span aria-hidden="true">·</span>
        <span>{frame.matchCount} {frame.matchCount === 1 ? "hit" : "hits"}</span>
      {/if}
      {#if frame.url}
        <span aria-hidden="true">·</span>
        <span class="search-card__host">{frame.url}</span>
      {/if}
    </span>
  </button>
{/if}

{#if kind === "audio" && audio}
  <button
    class="search-card ss-qcell"
    class:is-sel={selected}
    {id}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
  >
    <span
      class="search-card__media search-card__media--wave ss-qcell__f"
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
      <span class="ss-qcell__dur"
        >{formatDuration(
          Math.max(0, (audio.spanEndMs - audio.spanStartMs) / 1000),
        )}</span
      >
      {@render badges(
        audio.matchCount,
        "adjacent",
        audio.foundByMeaning,
        audio.hasSecretRedactions,
      )}
    </span>
    <span class="ss-qcell__l1">
      <span class="ss-qcell__app"
        >{audio.sourceKind === "microphone" ? "Mic" : "System"}</span
      >
      <span class="ss-qcell__ttl">
        “{#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark
              >{segment.text}</mark
            >{:else}{segment.text}{/if}{/each}”
      </span>
    </span>
    <span class="ss-qcell__l2">
      <span>{formatRelativeTime(audio.absoluteStartAt)}</span>
      <span aria-hidden="true">·</span>
      <span>speech</span>
    </span>
  </button>
{/if}

<style>
  /* The cell is `.ss-qcell` (kit): frame / identity line / meta line. This
     block only strips the <button> chrome and states the parts the kit leaves
     to the surface — the kit owns the geometry and the selection ring. */
  .search-card {
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
    color: var(--app-text);
    font: inherit;
    cursor: pointer;
  }

  .search-card:focus-visible {
    outline: none;
  }

  /* Hover and keyboard focus warm the frame's hairline; the 2px accent ring on
     `.is-sel` (kit) stays the one selection signal. */
  .search-card:hover .search-card__media,
  .search-card:focus-visible .search-card__media {
    box-shadow: 0 0 0 var(--hairline) var(--app-border-hover);
  }

  .search-card:focus-visible .ss-qcell__ttl {
    color: var(--app-accent-strong);
  }

  .search-card__appicon {
    flex: none;
    width: 13px;
    height: 13px;
    object-fit: contain;
    align-self: center;
  }

  /* The guarded host, mono like the rest of the meta line but allowed to
     ellipsize before the timestamp does. */
  .search-card__host {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-card mark {
    border-radius: 2px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    color: var(--app-text-strong);
    padding: 0 1px;
  }

  /* The 16:9 frame is `.ss-qcell__f`; this only adds the placeholder backing.
     It stays dark in both themes (it holds a screenshot) and the glyph is a
     fixed mid-gray legible on it while the image loads or when none exists. */
  .search-card__media {
    display: grid;
    place-items: center;
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

  /* The mockup's audio cell spends 54% of the frame's height on the waveform. */
  .search-card__wave {
    width: calc(100% - var(--s-24));
    height: 54%;
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
    left: var(--s-6);
    bottom: var(--s-6);
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-4);
    /* Anchored left only, so the duration chip keeps the bottom-right corner
       the kit gives it. */
    max-width: calc(100% - 64px);
  }

  .search-card__badge {
    display: inline-flex;
    align-items: center;
    font-size: var(--t-label);
    line-height: 1;
    padding: 3px 6px;
    border-radius: var(--r-sm);
    border: var(--hairline) solid var(--app-border-strong);
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    white-space: nowrap;
  }

  .search-card__badge--meaning {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  .search-card__badge--redacted {
    color: var(--app-warn);
    background: var(--app-warn-bg);
    border-color: var(--app-warn-border);
  }
</style>
