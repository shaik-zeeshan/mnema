<script lang="ts">
  // ConclusionStrip — horizontally-scrollable strip of conclusion cards for the
  // Subject-detail redesign (Slice 2). A sort control (Segmented) reorders the
  // strip; each card selects a conclusion (drives the sibling timeline) and
  // exposes a pin toggle. Sorting is delegated to the shared, tested
  // `sortConclusions`; the trend glyph mirrors SubjectDetail's first-vs-last
  // deadband derivation. Card anatomy + confidence tiers ported from
  // the unified-timeline mockup (.cs-card, lines 153-185).
  import type { Conclusion, SubjectTrajectory } from "$lib/types/recording";
  import Segmented from "$lib/components/Segmented.svelte";
  import { tip } from "$lib/components/tooltip";
  import { relativeTime } from "$lib/insights/conversationStore.svelte";
  import {
    sortConclusions,
    type ConclusionSort,
  } from "$lib/insights/subjectTimeline";

  interface Props {
    conclusions: Conclusion[];
    trajectories: Map<number, SubjectTrajectory>;
    selectedId: number | null;
    onSelect: (id: number) => void;
    onTogglePin: (id: number, pinned: boolean) => void;
    actionId?: number | null;
  }

  let {
    conclusions,
    trajectories,
    selectedId,
    onSelect,
    onTogglePin,
    actionId = null,
  }: Props = $props();

  let sort = $state<ConclusionSort>("confidence");

  const sortOptions = [
    { value: "confidence", label: "Confidence" },
    { value: "recent", label: "Recent" },
    { value: "warming", label: "Warming" },
  ];

  const ordered = $derived(sortConclusions(conclusions, trajectories, sort));

  type Trend = "up" | "steady" | "down";
  type Tier = "t-strong" | "t-moderate" | "t-weak" | "t-faded";

  // Confidence tier drives dot/bar/pct/stmt intensity (mockup lines 163-183).
  function tierFor(c: Conclusion): Tier {
    if (c.status === "faded") return "t-faded";
    if (c.confidence >= 0.68) return "t-strong";
    if (c.confidence >= 0.45) return "t-moderate";
    return "t-weak";
  }

  // Trend glyph: derived from the real trajectory (last vs first) with a ±0.04
  // deadband, matching SubjectDetail.
  function trendFor(c: Conclusion): Trend {
    const hist = trajectories.get(c.id)?.history ?? [];
    if (hist.length >= 2) {
      const delta = hist[hist.length - 1].confidence - hist[0].confidence;
      if (delta > 0.04) return "up";
      if (delta < -0.04) return "down";
    }
    return "steady";
  }

  const TREND_GLYPH: Record<Trend, string> = {
    up: "↑",
    down: "↓",
    steady: "–",
  };

  function pct(confidence: number): number {
    return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
  }

  function handleKeydown(event: KeyboardEvent, id: number): void {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(id);
    }
  }

  function togglePin(event: MouseEvent, c: Conclusion): void {
    // Don't let the star's click bubble up and also select the card.
    event.stopPropagation();
    onTogglePin(c.id, !c.pinned);
  }
</script>

<div class="strip-head">
  <span class="strip-eyebrow">Conclusions</span>
  <span class="strip-count">{ordered.length}</span>
  <div class="strip-sort">
    <span class="sort-lbl">Sort</span>
    <Segmented
      options={sortOptions}
      bind:value={sort}
      compact
      ariaLabel="Sort conclusions"
    />
  </div>
</div>

<div class="cstrip-wrap">
  <div class="cstrip" role="list">
    {#each ordered as c (c.id)}
      {@const hist = trajectories.get(c.id)?.history ?? []}
      {@const n = hist.length}
      {@const t = trendFor(c)}
      {@const first = n ? pct(hist[0].confidence) : pct(c.confidence)}
      {@const last = n ? pct(hist[n - 1].confidence) : pct(c.confidence)}
      {@const rel = relativeTime(c.lastSupportedAtMs)}
      <div
        class="cs-card {tierFor(c)}"
        class:is-selected={c.id === selectedId}
        role="listitem"
      >
        <div
          class="cs-hit"
          role="button"
          tabindex="0"
          aria-pressed={c.id === selectedId}
          use:tip={c.statement}
          onclick={() => onSelect(c.id)}
          onkeydown={(e) => handleKeydown(e, c.id)}
        >
          <div class="cs-top">
            <span class="cs-dot" aria-hidden="true"></span>
            {#if c.pinned}
              <button
                type="button"
                class="cs-pin"
                disabled={actionId === c.id}
                aria-label="Unpin conclusion"
                aria-pressed="true"
                onclick={(e) => togglePin(e, c)}>★</button
              >
            {/if}
            <span class="cs-snap"
              >{#if c.status === "faded"}faded · {/if}{n} snap</span
            >
          </div>
          <div class="cs-stmt">{c.statement}</div>
          <div class="cs-metrics">
            <span class="cs-bar"><i style="width:{pct(c.confidence)}%"></i></span
            >
            <span class="cs-pct">{pct(c.confidence)}%</span>
            <span class="cs-trend {t}" aria-hidden="true">{TREND_GLYPH[t]}</span>
          </div>
          <div class="cs-delta">
            {#if n < 2}
              {last} <span class="sep">·</span> {rel}
            {:else if c.status === "faded"}
              {first} <span class="sep">→</span> {last}
              <span class="sep">·</span> below floor
            {:else if t === "steady"}
              steady near {last} <span class="sep">·</span> {rel}
            {:else}
              {first} <span class="sep">→</span> {last}
              <span class="sep">·</span> {rel}
            {/if}
          </div>
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .strip-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .strip-eyebrow {
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.13em;
    color: var(--app-text-subtle);
  }
  .strip-count {
    font-size: var(--text-sm);
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }
  .strip-sort {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .sort-lbl {
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.13em;
    color: var(--app-text-subtle);
  }

  /* Horizontal scroll region + right-edge fade affordance (mockup lines 144-151). */
  .cstrip-wrap {
    position: relative;
    margin-bottom: 18px;
  }
  .cstrip {
    display: flex;
    gap: 10px;
    overflow-x: auto;
    padding: 1px 2px 9px;
    scroll-snap-type: x proximity;
  }
  .cstrip::-webkit-scrollbar {
    height: 8px;
  }
  .cstrip::-webkit-scrollbar-track {
    background: transparent;
  }
  .cstrip::-webkit-scrollbar-thumb {
    background: var(--app-border-strong);
    border: 2px solid var(--app-bg);
    border-radius: 999px;
  }
  .cstrip::-webkit-scrollbar-thumb:hover {
    background: var(--app-border-hover);
  }
  .cstrip-wrap::after {
    content: "";
    position: absolute;
    top: 0;
    right: 0;
    bottom: 9px;
    width: 48px;
    pointer-events: none;
    background: linear-gradient(to right, transparent, var(--app-bg));
  }

  .cs-card {
    flex: 0 0 auto;
    width: 254px;
    scroll-snap-align: start;
    text-align: left;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface);
    transition:
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .cs-card:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-subtle);
  }
  .cs-card.is-selected {
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
  }

  .cs-hit {
    display: flex;
    flex-direction: column;
    gap: 9px;
    padding: 12px;
    cursor: pointer;
    border-radius: 7px;
  }
  .cs-hit:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .cs-hit:active {
    transform: translateY(1px);
  }

  .cs-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 13px;
  }
  .cs-dot {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    flex: 0 0 auto;
    background: var(--app-accent);
  }
  .cs-card.t-moderate .cs-dot {
    opacity: 0.6;
  }
  .cs-card.t-weak .cs-dot {
    background: var(--app-text-subtle);
  }
  .cs-card.t-faded .cs-dot {
    background: var(--app-text-faint);
  }
  .cs-pin {
    padding: 0;
    font: inherit;
    font-size: var(--text-xs);
    line-height: 1;
    border: none;
    background: transparent;
    color: var(--app-warn);
    cursor: pointer;
  }
  .cs-card.is-selected .cs-pin {
    color: var(--app-accent-strong);
  }
  .cs-pin:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-radius: 3px;
  }
  .cs-pin:disabled {
    opacity: 0.5;
    cursor: progress;
  }
  .cs-snap {
    margin-left: auto;
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  .cs-stmt {
    font-size: var(--text-sm);
    line-height: 1.45;
    color: var(--app-text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: 32px;
  }
  .cs-card.is-selected .cs-stmt {
    color: var(--app-text-strong);
  }
  .cs-card.t-faded .cs-stmt {
    color: var(--app-text-faint);
  }

  .cs-metrics {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .cs-bar {
    flex: 1 1 auto;
    height: 4px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
  }
  .cs-bar i {
    display: block;
    height: 100%;
    background: var(--app-accent);
  }
  .cs-card.t-moderate .cs-bar i {
    opacity: 0.72;
  }
  .cs-card.t-weak .cs-bar i {
    opacity: 0.5;
  }
  .cs-card.t-faded .cs-bar i {
    opacity: 0.4;
    background: var(--app-text-faint);
  }
  .cs-pct {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    min-width: 31px;
    text-align: right;
  }
  .cs-card.t-faded .cs-pct {
    color: var(--app-text-subtle);
  }
  .cs-trend {
    font-size: var(--text-sm);
    width: 11px;
    text-align: center;
    flex: 0 0 auto;
    line-height: 1;
  }
  .cs-trend.up {
    color: var(--app-accent-strong);
  }
  .cs-trend.down {
    color: var(--app-danger);
  }
  .cs-trend.steady {
    color: var(--app-text-muted);
  }

  .cs-delta {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
  }
  .cs-delta .sep {
    color: var(--app-text-faint);
  }

  @media (prefers-reduced-motion: reduce) {
    .cs-card {
      transition: none;
    }
    .cs-hit:active {
      transform: none;
    }
  }
</style>
