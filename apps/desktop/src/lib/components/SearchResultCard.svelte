<script lang="ts">
  import type { FrameSearchResultDto, AudioSearchResultDto } from "$lib/types/app-infra";
  import { parseSearchSnippet } from "$lib/search-snippet";

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

  function formatTimestamp(ts: string): string {
    const d = new Date(ts.includes("T") ? ts : ts.replace(" ", "T"));
    if (isNaN(d.getTime())) return ts;
    return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }
  function formatDuration(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return "—";
    const total = Math.round(seconds);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }
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
    <div class="search-card__thumb">
      {#if thumbnailUrl}
        <img src={thumbnailUrl} alt="" loading="lazy" />
      {:else}
        <svg class="search-card__thumb-glyph" width="20" height="20" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" aria-hidden="true">
          <rect x="1.5" y="2" width="11" height="8" rx="1.5" />
          <path d="M4 12h6" />
          <path d="M7 10v2" />
        </svg>
      {/if}
    </div>
    <div class="search-card__body">
      <div class="search-card__line">
        <span class="search-card__app">{frame.appName ?? "Unknown app"}</span>
        {#if frame.windowTitle}
          <span class="search-card__sub" title={frame.windowTitle}>{frame.windowTitle}</span>
        {/if}
      </div>
      <p class="search-card__snippet">
        {#each parseSearchSnippet(frame.snippet) as segment}{#if segment.marked}<mark>{segment.text}</mark>{:else}{segment.text}{/if}{/each}
      </p>
      <div class="search-card__foot">
        <span class="search-card__time">{formatTimestamp(frame.groupEndAt)}</span>
        {#if frame.matchCount > 1}
          <span class="search-card__badge">{frame.matchCount} matches</span>
        {/if}
        {#if frame.hasSecretRedactions}
          <span class="search-card__badge search-card__badge--warn">redacted</span>
        {/if}
      </div>
    </div>
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
    <div
      class="search-card__thumb search-card__thumb--audio"
      class:search-card__thumb--mic={audio.sourceKind === "microphone"}
      class:search-card__thumb--sysaudio={audio.sourceKind !== "microphone"}
    >
      <svg class="search-card__wave" viewBox="0 0 44 24" aria-hidden="true">
        {#each [7, 13, 20, 10, 23, 9, 16, 12, 8] as barHeight, barIndex (barIndex)}
          <rect x={2 + barIndex * 4.8} y={(24 - barHeight) / 2} width="2.4" height={barHeight} rx="1.2" />
        {/each}
      </svg>
    </div>
    <div class="search-card__body">
      <div class="search-card__line">
        <span
          class="search-card__source"
          class:search-card__source--mic={audio.sourceKind === "microphone"}
          class:search-card__source--sysaudio={audio.sourceKind !== "microphone"}
        >{audio.sourceKind === "microphone" ? "microphone" : "system audio"}</span>
        <span class="search-card__sub">{formatDuration(Math.max(0, (audio.spanEndMs - audio.spanStartMs) / 1000))}</span>
      </div>
      <p class="search-card__snippet">
        {#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark>{segment.text}</mark>{:else}{segment.text}{/if}{/each}
      </p>
      <div class="search-card__foot">
        <span class="search-card__time">{formatTimestamp(audio.absoluteStartAt)}</span>
        {#if audio.matchCount > 1}
          <span class="search-card__badge">{audio.matchCount} adjacent</span>
        {/if}
        {#if audio.hasSecretRedactions}
          <span class="search-card__badge search-card__badge--warn">redacted</span>
        {/if}
      </div>
    </div>
  </button>
{/if}

<style>
  .search-card {
    width: 100%;
    min-width: 0;
    display: grid;
    grid-template-columns: 116px 1fr;
    gap: 13px;
    align-items: center;
    padding: 9px 10px;
    overflow: hidden;
    text-align: left;
    border: 1px solid transparent;
    border-radius: 9px;
    background: transparent;
    color: var(--app-text);
    font: inherit;
    cursor: pointer;
    transition:
      background 0.1s,
      border-color 0.1s;
  }

  .search-card:hover {
    border-color: var(--app-border);
    background: var(--app-surface-raised);
  }

  .search-card:focus-visible,
  .search-card--selected {
    outline: none;
    border-color: var(--app-accent-border);
    background: var(--app-surface-raised);
    box-shadow: 0 0 0 1px var(--app-accent-border);
  }

  .search-card__thumb {
    width: 116px;
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

  .search-card__thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .search-card__thumb-glyph {
    color: var(--app-text-faint);
  }

  /* Audio rail: a source-colored waveform tile. mic = green, system = olive,
     matching the capture-source tokens used across the timeline. */
  .search-card__thumb--audio {
    background: var(--app-surface-raised);
    color: var(--app-text-subtle);
  }

  .search-card__thumb--mic {
    border-color: var(--app-source-mic-border);
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }

  .search-card__thumb--sysaudio {
    border-color: var(--app-source-sysaudio-border);
    background: var(--app-source-sysaudio-bg);
    color: var(--app-source-sysaudio);
  }

  .search-card__wave {
    width: 62%;
    height: auto;
    fill: currentColor;
  }

  .search-card__body {
    min-width: 0;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow: hidden;
  }

  .search-card__line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .search-card__app {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--app-text-strong);
    font-size: 12.5px;
    font-weight: 600;
  }

  .search-card__source {
    flex: 0 0 auto;
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text-muted);
  }

  .search-card__source--mic {
    color: var(--app-source-mic);
  }

  .search-card__source--sysaudio {
    color: var(--app-source-sysaudio);
  }

  .search-card__sub {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--app-text-subtle);
    font-size: 11.5px;
  }

  .search-card__snippet {
    margin: 0;
    color: var(--app-text);
    font-size: 12px;
    line-height: 1.5;
    min-width: 0;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .search-card mark {
    border-radius: 2px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    color: var(--app-text-strong);
    padding: 0 1px;
  }

  .search-card__foot {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }

  .search-card__time {
    color: var(--app-text-subtle);
    font-size: 10.5px;
    white-space: nowrap;
  }

  .search-card__badge {
    flex: 0 0 auto;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--app-surface-hover);
    color: var(--app-text-subtle);
    font-size: 10px;
    line-height: 1.7;
  }

  .search-card__badge--warn {
    background: var(--app-warn-bg);
    color: var(--app-warn);
  }

  @media (max-width: 760px) {
    .search-card {
      grid-template-columns: 88px 1fr;
      gap: 10px;
    }

    .search-card__thumb {
      width: 88px;
    }
  }
</style>
