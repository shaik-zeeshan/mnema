<script lang="ts">
  // ConclusionStrip — the Subject detail's MASTER LIST (page 09): a grid of
  // opaque conclusion plates under a floating "CONCLUSIONS n" eyebrow, with a
  // Confidence / Recent / Warming segmented on the right. Selecting a card
  // drives the sibling hero + trajectory track; the star toggles pin.
  //
  // Each card states only what the backend has: a confidence bar, the percent,
  // a trend glyph, and a delta reading ("42% → 72% · 4h ago"). Sorting is
  // delegated to the shared, tested `sortConclusions`; the trend glyph mirrors
  // ConclusionHero's first-vs-last deadband derivation.
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

  /** "9 snap" / "faded · 4 snap" — built in JS so the separator keeps its
   *  spacing (template whitespace around a block gets trimmed). */
  function snapLabel(c: Conclusion, n: number): string {
    return c.status === "faded" ? `faded · ${n} snap` : `${n} snap`;
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

<div class="ceyebrow">
  <span class="ceyebrow__t">Conclusions</span>
  <span class="ceyebrow__n is-num">{ordered.length}</span>
  <div class="ceyebrow__sort">
    <Segmented
      options={sortOptions}
      bind:value={sort}
      ariaLabel="Sort conclusions"
    />
  </div>
</div>

<div class="cstrip" role="list">
  {#each ordered as c (c.id)}
    {@const hist = trajectories.get(c.id)?.history ?? []}
    {@const n = hist.length}
    {@const t = trendFor(c)}
    {@const first = n ? pct(hist[0].confidence) : pct(c.confidence)}
    {@const last = n ? pct(hist[n - 1].confidence) : pct(c.confidence)}
    {@const rel = relativeTime(c.lastSupportedAtMs)}
    <div
      class="plate ccard {tierFor(c)}"
      class:is-selected={c.id === selectedId}
      role="listitem"
    >
      <div
        class="ccard__hit"
        role="button"
        tabindex="0"
        aria-pressed={c.id === selectedId}
        use:tip={c.statement}
        onclick={() => onSelect(c.id)}
        onkeydown={(e) => handleKeydown(e, c.id)}
      >
        <div class="ccard__h">
          <span class="ccard__dot" aria-hidden="true"></span>
          {#if c.pinned}
            <button
              type="button"
              class="ccard__pin"
              disabled={actionId === c.id}
              aria-label="Unpin conclusion"
              aria-pressed="true"
              onclick={(e) => togglePin(e, c)}>★</button
            >
          {/if}
          <span class="ccard__snap is-num">{snapLabel(c, n)}</span>
        </div>
        <p class="ccard__stmt">{c.statement}</p>
        <span class="cbar"><i style="width:{pct(c.confidence)}%"></i></span>
        <div class="ccard__f">
          <span class="ccard__pct is-num">{pct(c.confidence)}%</span>
          <span class="ccard__trend {t}" aria-hidden="true">{TREND_GLYPH[t]}</span>
          <span class="ccard__delta is-num">
            {#if n < 2}
              {last}% <span class="sep">·</span> {rel}
            {:else if c.status === "faded"}
              {first}% <span class="sep">→</span> {last}%
              <span class="sep">·</span> below floor
            {:else if t === "steady"}
              steady near {last}% <span class="sep">·</span> {rel}
            {:else}
              {first}% <span class="sep">→</span> {last}%
              <span class="sep">·</span> {rel}
            {/if}
          </span>
        </div>
      </div>
    </div>
  {/each}
</div>

<style>
  .is-num {
    font-variant-numeric: tabular-nums;
  }

  /* Eyebrow — a floating label over the pane, not on a plate. */
  .ceyebrow {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 var(--s-8);
    padding: 0 var(--s-4);
  }
  .ceyebrow__t {
    font: var(--w-medium) var(--t-label) / var(--lh-label) var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .ceyebrow__n {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
  }
  .ceyebrow__sort {
    margin-left: auto;
  }

  /* The master list — a grid of plates, three up at 09's width, reflowing down
     to one on a narrow window. Never a horizontal scroller: a conclusion the
     user has to scroll to find is a conclusion they never read. */
  .cstrip {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
    gap: 10px;
    margin-bottom: var(--s-16);
  }

  .ccard {
    position: relative;
    transition: box-shadow var(--dur-quick) var(--ease);
  }
  .ccard:hover {
    box-shadow: var(--sh-tile), inset 0 0 0 var(--hairline) var(--app-border-hover);
  }
  /* Selection is a rim, not a fill — the plate stays opaque and readable. */
  .ccard.is-selected {
    box-shadow: var(--sh-tile), inset 0 0 0 1.5px var(--app-accent);
  }
  .ccard__hit {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: 10px 12px 11px;
    border-radius: var(--r-lg);
    cursor: pointer;
  }
  .ccard__hit:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  .ccard__h {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .ccard__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--app-accent);
  }
  .ccard.t-moderate .ccard__dot,
  .ccard.t-weak .ccard__dot {
    background: var(--chart-grey-3);
  }
  .ccard.t-faded .ccard__dot {
    background: var(--chart-grey-3);
    opacity: 0.5;
  }
  .ccard__pin {
    padding: 0;
    font: inherit;
    font-size: var(--t-meta);
    line-height: 1;
    border: 0;
    background: transparent;
    color: var(--app-accent);
    cursor: pointer;
  }
  .ccard__pin:focus-visible {
    outline: none;
    box-shadow: var(--ring);
    border-radius: var(--r-sm);
  }
  .ccard__pin:disabled {
    opacity: var(--opacity-busy);
    cursor: progress;
  }
  .ccard__snap {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }

  .ccard__stmt {
    margin: 0;
    flex: 1;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .ccard.t-faded .ccard__stmt {
    color: var(--app-text-muted);
  }

  .cbar {
    display: block;
    height: 5px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }
  .cbar i {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--app-accent);
  }
  .ccard.t-moderate .cbar i,
  .ccard.t-weak .cbar i,
  .ccard.t-faded .cbar i {
    background: var(--chart-grey-4);
  }

  .ccard__f {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }
  .ccard__pct {
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-strong);
  }
  .ccard.t-faded .ccard__pct {
    color: var(--app-text-muted);
  }
  .ccard__trend {
    width: 12px;
    flex: 0 0 auto;
    text-align: center;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
  }
  .ccard__trend.up {
    color: var(--app-accent);
  }
  .ccard__trend.steady {
    color: var(--app-text-subtle);
  }
  .ccard__trend.down {
    color: var(--app-text-faint);
  }
  /* The delta reading: where it came from, where it is, when it last moved. */
  .ccard__delta {
    margin-left: auto;
    text-align: right;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .ccard__delta .sep {
    color: var(--app-text-faint);
  }

  @media (prefers-reduced-motion: reduce) {
    .ccard {
      transition: none;
    }
  }
</style>
