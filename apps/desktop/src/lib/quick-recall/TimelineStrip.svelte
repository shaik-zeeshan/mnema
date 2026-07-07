<!-- Quick Recall timeline strip (Slice 6), per the signed-off mockup
     docs/quick-recall/mockups/search-redesign.html: a thin 8-day axis under
     the two-pane body with one dot per FETCHED result at its true time
     (groupStartAt / absoluteStartAt), the mockup's min-gap pass keeping
     clustered dots hoverable, a hover preview bubble (200px thumbnail cache
     for screen results, source-colored waveform tile for audio), and
     click = select (auto-expanding a collapsed show-more section — the store's
     selectFetchedResult). Filtered-view legend per the mockup; its dimmed
     non-matching dots deliberately DON'T exist here — the backend applies
     chips to the fetched set, so there are no fetched-but-filtered results to
     dim (see timelineLegend in timeline-dots.ts). -->
<script lang="ts">
  import { quickRecallSearch } from "./searchStore.svelte";
  import {
    computeTimelineDots,
    timelineAxisStartMs,
    timelineDayIndex,
    timelineDayLabels,
    timelineLegend,
    TIMELINE_DAY_SPAN,
  } from "./timeline-dots";
  import { parseCapturedAt, formatTimestampCompact } from "$lib/format-time";
  import type {
    FrameSearchResultDto,
    AudioSearchResultDto,
  } from "$lib/types/app-infra";

  const search = quickRecallSearch;

  type DotMeta =
    | { kind: "frame"; index: number; frame: FrameSearchResultDto; timeMs: number }
    | { kind: "audio"; index: number; audio: AudioSearchResultDto; timeMs: number };

  // The axis is anchored to "now" at each new result set (not per render), so
  // dots don't creep while the user reads a stable list.
  const axisNow = $derived.by(() => {
    void search.frames;
    void search.audio;
    return new Date();
  });
  const axisStartMs = $derived(timelineAxisStartMs(axisNow));
  const dayLabels = $derived(timelineDayLabels(axisNow));

  // One dot source per fetched result (unparseable timestamps are skipped —
  // a dot with no honest time would lie about "when").
  const metaByKey = $derived.by(() => {
    const map = new Map<string, DotMeta>();
    for (const [index, frame] of search.frames.entries()) {
      const timeMs = parseCapturedAt(frame.groupStartAt).getTime();
      if (!isNaN(timeMs)) {
        map.set(`frame:${frame.groupKey}`, { kind: "frame", index, frame, timeMs });
      }
    }
    for (const [index, audio] of search.audio.entries()) {
      const timeMs = parseCapturedAt(audio.absoluteStartAt).getTime();
      if (!isNaN(timeMs)) {
        map.set(`audio:${audio.groupKey}`, { kind: "audio", index, audio, timeMs });
      }
    }
    return map;
  });

  const dots = $derived(
    computeTimelineDots(
      Array.from(metaByKey.entries(), ([key, m]) => ({ key, timeMs: m.timeMs })),
      axisStartMs,
    ),
  );

  // Days with no matches keep their label, just dimmer (mockup `.empty`).
  const daysWithDots = $derived.by(() => {
    const days = new Set<number>();
    for (const meta of metaByKey.values()) {
      days.add(timelineDayIndex(meta.timeMs, axisStartMs));
    }
    return days;
  });

  const selectedKey = $derived.by(() => {
    const sel = search.selectedResult;
    if (sel === null) return null;
    return sel.kind === "frame"
      ? `frame:${sel.frame.groupKey}`
      : `audio:${sel.audio.groupKey}`;
  });

  const legend = $derived(
    timelineLegend(
      search.totalResultCount,
      search.activeFilterChips.map((chip) => `${chip.kind}: ${chip.label}`),
    ),
  );

  // ── Hover preview ──────────────────────────────────────────────────────────
  // Anchored to the strip wrapper (the mockup anchors to the window; the
  // wrapper spans the same full width, so the clamp math is identical).
  let stripEl = $state<HTMLDivElement | null>(null);
  let hovered = $state<{ key: string; left: number } | null>(null);

  const PREVIEW_W = 152;

  function showPreview(key: string, dotEl: HTMLElement): void {
    if (stripEl === null) return;
    const stripRect = stripEl.getBoundingClientRect();
    const dotRect = dotEl.getBoundingClientRect();
    const x = dotRect.left - stripRect.left + dotRect.width / 2;
    // Mockup clamp: keep the 152px bubble fully inside the window.
    const left = Math.max(
      8,
      Math.min(x - PREVIEW_W / 2, stripRect.width - PREVIEW_W - 8),
    );
    hovered = { key, left };
  }

  function hidePreview(): void {
    hovered = null;
  }

  function selectDot(key: string): void {
    const meta = metaByKey.get(key);
    if (meta === undefined) return;
    hidePreview();
    search.selectFetchedResult(meta.kind, meta.index);
  }

  const hoveredMeta = $derived(
    hovered !== null ? (metaByKey.get(hovered.key) ?? null) : null,
  );

  function dotTitle(meta: DotMeta): string {
    // Mockup dot title: "App · Today 15:02" (time answers "when was this?").
    if (meta.kind === "frame") {
      return `${meta.frame.appName ?? "Unknown app"} · ${formatTimestampCompact(meta.frame.groupStartAt)}`;
    }
    return `${sourceLabel(meta.audio)} · ${formatTimestampCompact(meta.audio.absoluteStartAt)}`;
  }

  function sourceLabel(audio: AudioSearchResultDto): string {
    return audio.sourceKind === "microphone" ? "microphone" : "system audio";
  }

  function formatDuration(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return "—";
    const total = Math.round(seconds);
    return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, "0")}`;
  }

  // Deterministic waveform bars for the audio preview tile — same renderer the
  // audio rows use (SearchResultCard.waveBars: 64 bars, 16807 Lehmer PRNG,
  // ±2-bar highlight cluster), duplicated here because it lives inside that
  // component. Same groupKey seed, so the preview tile matches its row.
  const WAVE_BARS = 64;

  function waveBars(
    key: string,
  ): { x: number; y: number; h: number; on: boolean }[] {
    let s = 0;
    for (let i = 0; i < key.length; i++) s = (s * 31 + key.charCodeAt(i)) >>> 0;
    s = (s % 2147483646) + 1;
    const at = 4 + (s % (WAVE_BARS - 8));
    const bars = [];
    for (let i = 0; i < WAVE_BARS; i++) {
      s = (s * 16807) % 2147483647;
      const h = 4 + ((s % 1000) / 1000) * 14;
      bars.push({ x: i * 7, y: (20 - h) / 2, h, on: Math.abs(i - at) <= 2 });
    }
    return bars;
  }
</script>

<div class="timeline-strip" bind:this={stripEl}>
  <div
    class="timeline-strip__bar"
    aria-label="Match timeline, last {TIMELINE_DAY_SPAN} days"
  >
    <div class="timeline-strip__axis"></div>
    {#if legend !== null}
      <div class="timeline-strip__legend">{legend}</div>
    {/if}
    {#each dayLabels as label, day (day)}
      <div
        class="timeline-strip__tick"
        style:left="calc(16px + (100% - 32px) * {day / TIMELINE_DAY_SPAN})"
      ></div>
      <div
        class="timeline-strip__tick-label"
        class:timeline-strip__tick-label--empty={!daysWithDots.has(day)}
        style:left="calc(16px + (100% - 32px) * {(day + 0.5) / TIMELINE_DAY_SPAN})"
      >
        {label}
      </div>
    {/each}
    {#each dots as dot (dot.key)}
      {@const meta = metaByKey.get(dot.key)}
      {#if meta !== undefined}
        <button
          type="button"
          class="timeline-strip__dot"
          class:timeline-strip__dot--sel={dot.key === selectedKey}
          style:left="calc(16px + (100% - 32px) * {dot.pc / 100})"
          tabindex="-1"
          aria-label={dotTitle(meta)}
          onmouseenter={(e) => showPreview(dot.key, e.currentTarget)}
          onmouseleave={hidePreview}
          onclick={() => selectDot(dot.key)}
        ></button>
      {/if}
    {/each}
  </div>

  {#if hovered !== null && hoveredMeta !== null}
    <div class="timeline-strip__preview" style:left="{hovered.left}px">
      {#if hoveredMeta.kind === "frame"}
        {@const thumbnailUrl = search.thumbnailCache.get(
          hoveredMeta.frame.thumbnailFrameId,
        )}
        <div class="timeline-strip__pthumb">
          {#if thumbnailUrl}
            <img class="timeline-strip__pimg" src={thumbnailUrl} alt="" />
          {:else}
            <!-- Glyph fallback while the thumbnail batch is in flight / missing
                 (same screen glyph the result rows fall back to). -->
            <svg
              class="timeline-strip__pglyph"
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
          {/if}
        </div>
        <div class="timeline-strip__ptime">
          {formatTimestampCompact(hoveredMeta.frame.groupStartAt)}
        </div>
        <div class="timeline-strip__plabel">
          {hoveredMeta.frame.appName ?? "Unknown app"}{hoveredMeta.frame
            .windowTitle
            ? ` · ${hoveredMeta.frame.windowTitle}`
            : ""}
        </div>
      {:else}
        <div
          class="timeline-strip__pthumb"
          class:timeline-strip__pthumb--mic={hoveredMeta.audio.sourceKind ===
            "microphone"}
          class:timeline-strip__pthumb--sys={hoveredMeta.audio.sourceKind !==
            "microphone"}
        >
          <svg
            class="timeline-strip__pwave"
            viewBox="0 0 448 20"
            preserveAspectRatio="none"
            aria-hidden="true"
          >
            {#each waveBars(hoveredMeta.audio.groupKey) as bar (bar.x)}
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
        </div>
        <div class="timeline-strip__ptime">
          {formatTimestampCompact(hoveredMeta.audio.absoluteStartAt)}
        </div>
        <div class="timeline-strip__plabel">
          {sourceLabel(hoveredMeta.audio)} · {formatDuration(
            Math.max(
              0,
              (hoveredMeta.audio.spanEndMs - hoveredMeta.audio.spanStartMs) /
                1000,
            ),
          )}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Positioning context for the hover preview: full window width, sits between
     the body and the footer, so the bubble math matches the mockup's
     window-anchored `.tl-preview` (bottom: 6px above the strip top). */
  .timeline-strip {
    flex: none;
    position: relative;
  }

  /* Mockup `.timeline`: thin 36px strip on the subtle chrome surface. */
  .timeline-strip__bar {
    height: 36px;
    position: relative;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }

  .timeline-strip__axis {
    position: absolute;
    left: 16px;
    right: 16px;
    top: 16px;
    height: 1px;
    background: var(--app-border-strong);
  }

  .timeline-strip__tick {
    position: absolute;
    top: 13px;
    width: 1px;
    height: 7px;
    background: var(--app-border-strong);
  }

  .timeline-strip__tick-label {
    position: absolute;
    top: 22px;
    transform: translateX(-50%);
    font-size: 10px;
    letter-spacing: 0.05em;
    color: var(--app-text-subtle);
    text-transform: uppercase;
    white-space: nowrap;
  }

  .timeline-strip__tick-label--empty {
    color: var(--app-text-faint);
  }

  :global([data-theme="light"]) .timeline-strip__tick-label--empty {
    color: var(--app-border-strong);
  }

  .timeline-strip__legend {
    position: absolute;
    top: 1px;
    right: 16px;
    font-size: 10px;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  /* Mockup `.dot`: 7px accent dot on the axis, scale on hover, an invisible
     inset -5px hit-area extension, and a soft ring on the selected dot. */
  .timeline-strip__dot {
    position: absolute;
    top: 13px;
    width: 7px;
    height: 7px;
    margin-left: -3.5px;
    padding: 0;
    border-radius: 50%;
    background: var(--app-accent);
    border: 1px solid var(--app-accent-strong);
    cursor: pointer;
  }

  @media (prefers-reduced-motion: no-preference) {
    .timeline-strip__dot {
      transition:
        transform 0.1s,
        opacity 0.12s;
    }
  }

  .timeline-strip__dot:hover {
    transform: scale(1.5);
  }

  .timeline-strip__dot::after {
    content: "";
    position: absolute;
    inset: -5px;
    border-radius: 50%;
  }

  .timeline-strip__dot--sel {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-accent) 25%, transparent);
  }

  /* Mockup `.tl-preview`: floating bubble 6px above the strip, clamped inside
     the window by the inline `left`. Pointer-events off so it never steals the
     dot's mouseleave. */
  .timeline-strip__preview {
    position: absolute;
    bottom: calc(100% + 6px);
    z-index: 20;
    width: 152px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    box-shadow: var(--app-shadow-popover);
    padding: 5px;
    pointer-events: none;
  }

  .timeline-strip__pthumb {
    position: relative;
    width: 140px;
    height: 88px;
    border-radius: 5px;
    overflow: hidden;
    border: 1px solid var(--app-border);
    background: #101014;
    display: grid;
    place-items: center;
    color: #6a6a74;
  }

  .timeline-strip__pimg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .timeline-strip__pthumb--mic {
    background: var(--app-source-mic-bg);
    border-color: var(--app-source-mic-border);
    color: var(--app-source-mic);
  }

  .timeline-strip__pthumb--sys {
    background: var(--app-source-sysaudio-bg);
    border-color: var(--app-source-sysaudio-border);
    color: var(--app-source-sysaudio);
  }

  .timeline-strip__pwave {
    position: absolute;
    inset: 30px 10px;
    width: calc(100% - 20px);
    height: calc(100% - 60px);
  }

  .timeline-strip__pwave .wb {
    fill: color-mix(in srgb, currentColor 45%, transparent);
  }

  .timeline-strip__pwave .wb-on {
    fill: currentColor;
  }

  .timeline-strip__ptime {
    font-size: 11px;
    color: var(--app-text-strong);
    padding: 5px 3px 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .timeline-strip__plabel {
    font-size: 10px;
    color: var(--app-text-muted);
    padding: 2px 3px 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
