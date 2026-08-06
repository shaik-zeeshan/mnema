<script lang="ts">
  // CONCLUSIONS — a 4×1 tile whose payload bleeds right and down, so the strip
  // scrolls horizontally and the card that doesn't fit is genuinely half-cut by
  // the tile radius. That half-cut card is the same overflow signifier the
  // Overview's moments strip uses, and it is real overflow, never a drawn stub.
  //
  // Ordering is the shared, tested `sortConclusions` (pinned always float first);
  // the trend glyph is the same first→last ±0.04 deadband every other surface
  // uses. Nothing here re-derives a threshold.
  import type { Conclusion, SubjectTrajectory } from "$lib/types/recording";
  import Segmented from "$lib/components/Segmented.svelte";
  import {
    sortConclusions,
    type ConclusionSort,
  } from "$lib/insights/subjectTimeline";
  import { ago, conf2, pct } from "./subjects-format";

  interface Props {
    conclusions: Conclusion[];
    trajectories: Map<number, SubjectTrajectory>;
    selectedId: number | null;
    onSelect: (id: number) => void;
  }
  let { conclusions, trajectories, selectedId, onSelect }: Props = $props();

  const SORT_OPTIONS = [
    { value: "confidence", label: "Confidence" },
    { value: "recent", label: "Recent" },
    { value: "warming", label: "Warming" },
  ];
  let sortValue = $state("confidence");
  const ordered = $derived(
    sortConclusions(conclusions, trajectories, sortValue as ConclusionSort),
  );

  /** The card's journey line — the first and last confidence in this
   *  conclusion's own snapshot history, and when it last moved. A conclusion
   *  with a single snapshot has no journey to claim, so it says where it sits. */
  function journey(c: Conclusion): string {
    const h = trajectories.get(c.id)?.history ?? [];
    const when = c.status === "faded" ? "below floor" : ago(c.lastSupportedAtMs);
    if (h.length < 2) return `at ${conf2(c.confidence)} · ${when}`;
    const first = h[0].confidence;
    const last = h[h.length - 1].confidence;
    if (pct(first) === pct(last)) return `steady near ${conf2(last)} · ${when}`;
    return `${conf2(first)} → ${conf2(last)} · ${when}`;
  }

  function trendGlyph(c: Conclusion): { g: string; cls: string } {
    const h = trajectories.get(c.id)?.history ?? [];
    if (h.length >= 2) {
      const d = h[h.length - 1].confidence - h[0].confidence;
      if (d > 0.04) return { g: "↑", cls: "up" };
      if (d < -0.04) return { g: "↓", cls: "down" };
    }
    return { g: "–", cls: "steady" };
  }
</script>

<div class="tile tile--w4 tile--static">
  <div class="tile__h">
    <span class="t-label">Conclusions</span>
    <span class="hright">
      <span class="tile__more is-mono is-num">{ordered.length}</span>
      <Segmented
        options={SORT_OPTIONS}
        bind:value={sortValue}
        compact
        ariaLabel="Sort conclusions"
      />
    </span>
  </div>

  {#if ordered.length}
    <div class="pay pay--bleedr pay--bleedb">
      <div class="cstrip scroll">
        {#each ordered as c (c.id)}
          {@const t = trendGlyph(c)}
          <button
            type="button"
            class="ccard"
            class:ccard--faded={c.status === "faded"}
            class:is-sel={c.id === selectedId}
            aria-pressed={c.id === selectedId}
            onclick={() => onSelect(c.id)}
          >
            <span class="ccard__h">
              <i class="sdot"></i>
              {#if c.pinned}<span class="spin" title="Pinned">★</span>{/if}
              <span class="scount">
                {#if c.status === "faded"}faded · {/if}
                {trajectories.get(c.id)?.history.length ?? 0} snap
              </span>
            </span>
            <!-- The clamp box is nested: a flex item's `-webkit-box` display is
                 blockified away, which silently kills -webkit-line-clamp. -->
            <span class="ccard__st"><span class="clamp">{c.statement}</span></span>
            <span class="ccard__m">
              <span class="bar"><i style="width:{pct(c.confidence)}%"></i></span>
              {pct(c.confidence)}%
              <span class="strend strend--{t.cls}">{t.g}</span>
            </span>
            <span class="ccard__d">{journey(c)}</span>
          </button>
        {/each}
      </div>
    </div>
  {:else}
    <div class="pay empty">
      <span class="t-meta">No conclusions under this subject yet.</span>
    </div>
  {/if}
</div>

<style>
  /* count + sort as one right-hand group, so the count sits beside the control
     rather than drifting to the middle of the header row. */
  .hright {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--s-8);
  }
  .hright .tile__more {
    margin-left: 0;
  }
  .cstrip {
    display: flex;
    gap: var(--s-12);
    height: 100%;
    overflow-x: auto;
    overflow-y: hidden;
    scroll-snap-type: x proximity;
  }
  .ccard {
    flex: 0 0 272px;
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: var(--s-12);
    border: 0;
    border-radius: var(--tile-r-in);
    background: var(--app-surface-subtle);
    color: var(--app-accent);
    text-align: left;
    cursor: pointer;
    scroll-snap-align: start;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .ccard:hover {
    background: var(--app-surface-hover);
  }
  /* Selection is a tinted card with a hairline, not a neon ring: the card's own
     confidence bar already owns the accent, and two full-strength accents on one
     object stop meaning anything. */
  .ccard.is-sel,
  .ccard.is-sel:hover {
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }
  .ccard:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }
  .ccard--faded {
    opacity: 0.62;
    color: var(--app-text-faint);
  }

  .ccard__h {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }
  .sdot {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
  }
  .spin {
    color: var(--app-warn);
    font-size: 11px;
  }
  .scount {
    margin-left: auto;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
    white-space: nowrap;
  }
  /* Clamp rather than clip: a statement cut mid-glyph reads as a rendering bug,
     an ellipsis reads as "there is more on the card below". */
  .ccard__st {
    flex: 0 1 auto;
    min-height: 0;
    overflow: hidden;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .ccard--faded .ccard__st {
    color: var(--app-text-muted);
  }
  /* The bar + journey are pinned to the card's floor, so every card in the strip
     reads its confidence off the same line no matter how long the statement is. */
  .ccard__m {
    margin-top: auto;
    display: flex;
    align-items: center;
    gap: var(--s-6);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }
  .bar {
    position: relative;
    flex: 1 1 auto;
    height: 4px;
    border-radius: 2px;
    background: var(--app-border-strong);
  }
  .bar i {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 2px;
    background: currentColor;
  }
  .strend--up {
    color: var(--app-accent);
  }
  .strend--down {
    color: var(--app-danger);
  }
  .strend--steady {
    color: var(--app-text-faint);
  }
  .ccard__d {
    font: var(--w-regular) var(--t-label) / 1.3 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }

  .empty {
    display: flex;
    align-items: center;
  }

  @media (max-width: 900px) {
    .ccard {
      flex: 0 0 224px;
    }
    .clamp {
      -webkit-line-clamp: 3;
      line-clamp: 3;
    }
  }
</style>
