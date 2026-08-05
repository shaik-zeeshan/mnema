<!-- Quick Recall search result TILE — direction 04's `.qcell`: the frame IS the
     cell. A 349×196 media block (screenshot for screen results, source-coloured
     waveform for audio) carries the affordance keycaps bottom-left and the
     duration chip bottom-right; the two-line caption sits UNDERNEATH it —
     uppercase mono app name + title, then mono metadata and the matched snippet
     with its <mark> highlights. Selection is a 2px accent ring on the frame, not
     a box around the text. -->
<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import type {
    FrameSearchResultDto,
    AudioSearchResultDto,
  } from "$lib/types/app-infra";
  import { parseSearchSnippet } from "$lib/search-snippet";
  import { formatRelativeTime, parseCapturedAt } from "$lib/format-time";
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

  // The caption's mono clock (`14:32:08`). The mockup leads line 2 with the
  // wall-clock instant, not a relative age — a grid of frames is scanned by
  // "when in the day", which "2d ago" can't answer.
  function clockOf(ts: string): string {
    const at = parseCapturedAt(ts);
    if (Number.isNaN(at.getTime())) return "";
    return at.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
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
      <!-- Affordance keycaps bottom-left: the selected cell says what ⏎ does
           right on the picture, the direction's one rule applied to the grid. -->
      {#if selected}
        <span class="search-card__aff" aria-hidden="true">
          <span class="kbd">↵</span><span class="kbd kbd--wide">open</span>
        </span>
      {/if}
      {@render badges(
        frame.matchCount,
        "matches",
        frame.foundByMeaning,
        frame.hasSecretRedactions,
      )}
    </span>
    <span class="search-card__cap">
      <span class="search-card__l1">
        {#if appIcons.src(frame.appBundleId ?? frame.appName) !== null}
          <img
            class="search-card__appicon"
            src={appIcons.src(frame.appBundleId ?? frame.appName)}
            alt=""
            aria-hidden="true"
          />
        {/if}
        <span class="search-card__app">{frame.appName ?? "Unknown app"}</span>
        <span class="search-card__title" use:tip={frame.windowTitle ?? undefined}>
          {frame.windowTitle ?? frame.appName ?? "Screen"}
        </span>
      </span>
      <span class="search-card__l2">
        <span>{clockOf(frame.groupEndAt)}</span><span
          class="search-card__l2-dot">·</span
        ><span>{formatRelativeTime(frame.groupEndAt)}</span><span
          class="search-card__l2-dot">·</span
        ><span class="search-card__q"
          >{#each parseSearchSnippet(frame.snippet) as segment}{#if segment.marked}<mark
                >{segment.text}</mark
              >{:else}{segment.text}{/if}{/each}</span
        >
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
      {#if selected}
        <span class="search-card__aff" aria-hidden="true">
          <span class="kbd">↵</span><span class="kbd kbd--wide">open</span>
        </span>
      {/if}
      <!-- Duration chip, bottom-right of the frame (mockup `.qcell__dur`). -->
      <span class="search-card__dur"
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
    <span class="search-card__cap">
      <span class="search-card__l1">
        <span class="search-card__app"
          >{audio.sourceKind === "microphone" ? "Mic" : "System audio"}</span
        >
        <span class="search-card__title"
          >“{#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark
                >{segment.text}</mark
              >{:else}{segment.text}{/if}{/each}”</span
        >
      </span>
      <span class="search-card__l2">
        <span>{clockOf(audio.absoluteStartAt)}</span><span
          class="search-card__l2-dot">·</span
        ><span>{formatRelativeTime(audio.absoluteStartAt)}</span><span
          class="search-card__l2-dot">·</span
        ><span class="search-card__q">speech</span>
      </span>
    </span>
  </button>
{/if}

<style>
  /* One grid cell (`.qcell`): the 196px frame, then the caption under it. The
     cell itself is bare — no card chrome — so the picture is the object and the
     text is its label, which is what makes the grid read as media. */
  .search-card {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    width: 100%;
    min-width: 0;
    padding: 0;
    text-align: left;
    border: 0;
    background: none;
    color: var(--app-text);
    font: inherit;
    cursor: pointer;
  }

  .search-card:focus-visible {
    outline: none;
  }

  /* Selection is a 2px accent ring on the FRAME (`.qcell.is-sel .qcell__f`),
     never a box drawn around the caption text. */
  .search-card:hover .search-card__media {
    box-shadow: 0 0 0 var(--hairline) var(--app-border-hover);
  }

  .search-card:focus-visible .search-card__media,
  .search-card--selected .search-card__media,
  .search-card--selected:hover .search-card__media {
    box-shadow: 0 0 0 2px var(--app-accent);
  }

  /* Caption, underneath: uppercase mono app name + title, then the mono
     metadata line carrying the matched snippet. */
  .search-card__cap {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .search-card__l1 {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
    min-width: 0;
  }

  .search-card__appicon {
    flex: none;
    width: 12px;
    height: 12px;
    object-fit: contain;
    align-self: center;
  }

  .search-card__app {
    flex: none;
    max-width: 40%;
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-weight: var(--w-medium);
    line-height: 1.4;
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* One title line — the window title (screen) or the quoted transcript
     (audio). Never wraps: the cell's height is fixed by the frame. */
  .search-card__title {
    min-width: 0;
    font-family: var(--app-font-sans);
    font-size: var(--t-ui);
    line-height: 1.25;
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Line 2: mono clock + age, then the matched text — so "why did this hit?"
     is answered on the cell itself. */
  .search-card__l2 {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font-family: var(--app-font-mono);
    font-size: var(--t-meta);
    line-height: 1.35;
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
  }

  .search-card__l2-dot {
    color: var(--app-text-faint);
  }

  .search-card__q {
    min-width: 0;
    font-family: var(--app-font-sans);
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-card mark {
    border-radius: 2px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
    padding: 0 2px;
  }

  /* The 196px frame. The backing stays dark in both themes (it holds a
     screenshot); the glyph is a fixed mid-gray legible on that backing while
     the image loads or when no preview exists. */
  .search-card__media {
    position: relative;
    display: grid;
    place-items: center;
    height: 196px;
    border-radius: var(--r-lg);
    overflow: hidden;
    background: #101014;
    color: #6a6a74;
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
  }

  @media (prefers-reduced-motion: no-preference) {
    .search-card__media {
      transition: box-shadow var(--dur-quick) var(--ease);
    }
  }

  /* Affordance keycaps, bottom-left of the frame (mockup `.qcell__aff`). */
  .search-card__aff {
    position: absolute;
    left: var(--s-8);
    bottom: var(--s-8);
    display: inline-flex;
    gap: var(--s-4);
  }

  /* Duration chip, bottom-right (mockup `.qcell__dur`). */
  .search-card__dur {
    position: absolute;
    right: var(--s-8);
    bottom: var(--s-8);
    padding: 3px 5px;
    border-radius: var(--r-sm);
    background: rgba(10, 12, 16, 0.62);
    backdrop-filter: blur(6px);
    color: #fff;
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-weight: var(--w-medium);
    line-height: 1;
    font-variant-numeric: tabular-nums;
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

  /* Accessories float over the frame's TOP edge — the bottom belongs to the
     affordance keycaps (left) and the duration chip (right). */
  .search-card__badges {
    position: absolute;
    left: var(--s-8);
    right: var(--s-8);
    top: var(--s-8);
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-4);
  }

  .search-card__badge {
    display: inline-flex;
    align-items: center;
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-weight: var(--w-medium);
    line-height: 1;
    padding: 3px 5px;
    border-radius: var(--r-sm);
    background: rgba(10, 12, 16, 0.62);
    backdrop-filter: blur(6px);
    color: rgba(255, 255, 255, 0.86);
    text-transform: uppercase;
    letter-spacing: var(--ls-label);
    font-variant-numeric: tabular-nums;
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
