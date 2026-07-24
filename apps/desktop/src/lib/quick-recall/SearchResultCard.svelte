<!-- Quick Recall search result row, per the signed-off mockup
     docs/quick-recall/mockups/search-redesign.html (Raycast row anatomy):
     visual (150×94 thumb / source-colored waveform tile) | title + one snippet
     line | right-aligned accessory column (relative time, match count, pills).
     Accessories the detail pane duplicates (match count, meaning/redacted
     pills) are stripped from the SELECTED row — the Raycast rule — while the
     relative time stays. The old hover "open in browser" host chip is gone;
     the URL lives in the detail pane and ⌘O (search-keys.ts → store) remains
     the open-page path. -->
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
  // in-span match offset; the detail pane (slice 5) owns the real match marker.
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

{#if kind === "frame" && frame}
  <button
    class="search-card search-card--frame"
    class:search-card--selected={selected}
    {id}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
  >
    <span class="search-card__thumb">
      <svg
        class="search-card__thumb-glyph"
        width="20"
        height="20"
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
    </span>
    <span class="search-card__meta">
      <span class="search-card__line1">
        {#if appIcons.src(frame.appBundleId ?? frame.appName) !== null}
          <img
            class="search-card__appicon"
            src={appIcons.src(frame.appBundleId ?? frame.appName)}
            alt=""
            aria-hidden="true"
          />
        {/if}
        <span class="search-card__app">{frame.appName ?? "Unknown app"}</span>
        {#if frame.windowTitle}
          <span class="search-card__win" use:tip={frame.windowTitle}
            >{frame.windowTitle}</span
          >
        {/if}
      </span>
      <span class="search-card__snippet">
        {#each parseSearchSnippet(frame.snippet) as segment}{#if segment.marked}<mark
              >{segment.text}</mark
            >{:else}{segment.text}{/if}{/each}
      </span>
    </span>
    <span class="search-card__acc">
      <span class="search-card__time">{formatRelativeTime(frame.groupEndAt)}</span>
      {#if !selected}
        {#if frame.matchCount > 1}
          <span class="search-card__count">{frame.matchCount} matches</span>
        {/if}
        {#if frame.foundByMeaning}
          <span class="search-card__pill search-card__pill--meaning"
            >found by meaning</span
          >
        {/if}
        {#if frame.hasSecretRedactions}
          <span class="search-card__pill search-card__pill--redacted"
            >redacted</span
          >
        {/if}
      {/if}
    </span>
  </button>
{/if}

{#if kind === "audio" && audio}
  <button
    class="search-card search-card--audio"
    class:search-card--selected={selected}
    {id}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
  >
    <span
      class="search-card__wavetile"
      class:search-card__wavetile--mic={audio.sourceKind === "microphone"}
      class:search-card__wavetile--sys={audio.sourceKind !== "microphone"}
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
    </span>
    <span class="search-card__meta">
      <span class="search-card__quote"
        >“{#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark
              >{segment.text}</mark
            >{:else}{segment.text}{/if}{/each}”</span
      >
      <span class="search-card__srcline"
        >{audio.sourceKind === "microphone" ? "microphone" : "system audio"} · {formatDuration(
          Math.max(0, (audio.spanEndMs - audio.spanStartMs) / 1000),
        )}</span
      >
    </span>
    <span class="search-card__acc">
      <span class="search-card__time"
        >{formatRelativeTime(audio.absoluteStartAt)}</span
      >
      {#if !selected}
        {#if audio.matchCount > 1}
          <span class="search-card__count">{audio.matchCount} adjacent</span>
        {/if}
        {#if audio.foundByMeaning}
          <span class="search-card__pill search-card__pill--meaning"
            >found by meaning</span
          >
        {/if}
        {#if audio.hasSecretRedactions}
          <span class="search-card__pill search-card__pill--redacted"
            >redacted</span
          >
        {/if}
      {/if}
    </span>
  </button>
{/if}

<style>
  /* Raycast-style row: visual | title+snippet | right accessories. Values
     mirror the mockup's .row rules; vertical rhythm (8px between rows) comes
     from the results list's flex gap, not a margin here. */
  .search-card {
    width: 100%;
    min-width: 0;
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 8px 12px;
    overflow: hidden;
    text-align: left;
    border: 1px solid transparent;
    border-radius: 9px;
    background: transparent;
    color: var(--app-text);
    font: inherit;
    cursor: pointer;
  }

  @media (prefers-reduced-motion: no-preference) {
    .search-card {
      transition:
        background 0.12s ease,
        border-color 0.12s ease,
        box-shadow 0.12s ease;
    }
  }

  .search-card:hover {
    background: var(--app-surface-hover);
  }

  /* Selected is the roving highlight: green-tinted active surface plus the
     accent border and soft ring, so it reads clearly above a plain hover. */
  .search-card:focus-visible,
  .search-card--selected,
  .search-card--selected:hover {
    outline: none;
    background: var(--app-surface-active);
    border-color: var(--app-accent-border);
    box-shadow:
      0 0 0 1px var(--app-accent-border),
      0 0 0 4px color-mix(in srgb, var(--app-accent) 12%, transparent);
  }

  /* 150×94 thumbnail — large enough to recognize the app and content. The
     backing stays dark in both themes (it holds a screenshot); the glyph is a
     fixed mid-gray legible on that backing while the image loads or when no
     preview exists. */
  .search-card__thumb {
    position: relative;
    flex: none;
    width: 150px;
    height: 94px;
    display: inline-grid;
    place-items: center;
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    overflow: hidden;
    background: var(--app-thumb-stage);
    color: var(--app-thumb-stage-fg);
  }

  /* The image sits above the always-present glyph placeholder and fades in once
     decoded, so the reserved box never flashes from a void and never reflows. */
  .search-card__thumb-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
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

  /* Audio visual: a source-colored waveform glyph tile — deliberately NOT a
     screenshot (the words are the content; alignedFrame is not the hero).
     mic = green, sys = olive, matching the capture-source tokens. */
  .search-card__wavetile {
    position: relative;
    flex: none;
    width: 150px;
    height: 48px;
    border: 1px solid;
    border-radius: 6px;
    overflow: hidden;
  }

  .search-card__wavetile--mic {
    background: var(--app-source-mic-bg);
    border-color: var(--app-source-mic-border);
    color: var(--app-source-mic);
  }

  .search-card__wavetile--sys {
    background: var(--app-source-sysaudio-bg);
    border-color: var(--app-source-sysaudio-border);
    color: var(--app-source-sysaudio);
  }

  .search-card__wave {
    position: absolute;
    inset: 10px 8px;
    width: calc(100% - 16px);
    height: calc(100% - 20px);
  }

  .search-card__wave .wb {
    fill: color-mix(in srgb, currentColor 45%, transparent);
  }

  .search-card__wave .wb-on {
    fill: currentColor;
  }

  /* Middle column: title line + one snippet line. Screen rows top-align it
     beside the tall thumb; audio rows center it beside the short tile. */
  .search-card__meta {
    flex: 1;
    min-width: 0;
    display: block;
  }

  .search-card--frame .search-card__meta {
    align-self: flex-start;
    padding-top: 4px;
  }

  .search-card__line1 {
    display: flex;
    align-items: baseline;
    gap: 7px;
    min-width: 0;
    font-size: 12px;
  }

  /* Real app icon before the app name. line1 is baseline-aligned for the
     text; the icon centers itself against the row instead. */
  .search-card__appicon {
    flex: none;
    align-self: center;
    width: 14px;
    height: 14px;
    object-fit: contain;
  }

  .search-card__app {
    flex: none;
    color: var(--app-text-strong);
    font-weight: 600;
  }

  .search-card__win {
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  /* One marked snippet line (span needs block for ellipsis to apply). */
  .search-card__snippet {
    display: block;
    margin-top: 4px;
    font-size: 11px;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-card mark {
    border-radius: 2px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    color: var(--app-text-strong);
    padding: 0 1px;
  }

  /* Audio title line: the quoted transcript snippet IS the title. */
  .search-card__quote {
    color: var(--app-text-strong);
    font-size: 12px;
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .search-card__srcline {
    display: block;
    margin-top: 4px;
    font-size: 11px;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Right accessory column. Screen rows top-align it with the meta column;
     audio rows center it. Selected rows keep only the time (the detail pane
     duplicates the rest). */
  .search-card__acc {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    max-width: 120px;
  }

  .search-card--frame .search-card__acc {
    align-self: flex-start;
    padding-top: 4px;
  }

  .search-card__time,
  .search-card__count {
    font-size: 10px;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  .search-card__pill {
    display: inline-flex;
    align-items: center;
    font-size: 10px;
    line-height: 1;
    padding: 3px 6px;
    border-radius: 4px;
    border: 1px solid;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    white-space: nowrap;
  }

  .search-card__pill--meaning {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  .search-card__pill--redacted {
    color: var(--app-warn);
    background: var(--app-warn-bg);
    border-color: var(--app-warn-border);
  }
</style>
