<script lang="ts">
  // The opened subject's master column: its beliefs, ranked. Selecting a card
  // drives the story column and the inspector.
  //
  // Ordering is `sortConclusions` (pinned always first, then the chosen key) —
  // the same tested function the old Subject-detail strip uses.
  import type { Conclusion } from "$lib/types/recording";
  import { deltaLine, pct } from "./format";
  import type { SubjectDetailData } from "./subject-detail-data.svelte";
  import type { ConclusionSort } from "$lib/insights/subjectTimeline";

  interface Props {
    data: SubjectDetailData;
  }

  let { data }: Props = $props();

  const SORTS: { value: ConclusionSort; label: string }[] = [
    { value: "confidence", label: "Confidence" },
    { value: "recent", label: "Recent" },
    { value: "warming", label: "Warming" },
  ];

  // The trend glyph, from the belief's own arc with the same ±0.04 deadband the
  // subject rows use.
  function trendGlyph(c: Conclusion): { glyph: string; cls: string } {
    const h = data.historyOf(c.id);
    if (h.length >= 2) {
      const delta = h[h.length - 1] - h[0];
      if (delta > 0.04) return { glyph: "▲", cls: "is-up" };
      if (delta < -0.04) return { glyph: "▼", cls: "is-down" };
    }
    return { glyph: "–", cls: "" };
  }
</script>

<div class="col">
  <div class="head">
    <span class="t-label">Conclusions</span>
    <span class="t-meta is-mono">{data.conclusionCount}</span>
    <span class="ss-tstrip__spacer"></span>
    <div class="ss-seg" role="group" aria-label="Sort conclusions">
      {#each SORTS as option (option.value)}
        <button
          type="button"
          class="ss-seg__i"
          class:is-on={data.sort === option.value}
          aria-pressed={data.sort === option.value}
          onclick={() => (data.sort = option.value)}>{option.label}</button
        >
      {/each}
    </div>
  </div>

  <div class="cards">
    {#each data.ordered as c (c.id)}
      {@const t = trendGlyph(c)}
      {@const snaps = data.snapshotCount(c.id)}
      {@const faded = c.status === "faded"}
      <button
        type="button"
        class="card"
        class:is-sel={data.selectedId === c.id}
        class:is-faded={faded}
        aria-pressed={data.selectedId === c.id}
        onclick={() => (data.selectedId = c.id)}
      >
        <span class="card__top">
          <span class="dot" class:is-strong={!faded && c.confidence >= 0.68}></span>
          {#if c.pinned}<span class="glyph">★</span>{/if}
          <span class="snap">{#if faded}faded · {/if}{snaps} snap</span>
          <span class="ss-tstrip__spacer"></span>
          <span class="glyph {t.cls}">{t.glyph}</span>
        </span>
        <span class="card__st">{c.statement}</span>
        <span class="card__m">
          <span class="cbar"
            ><i class:is-dim={faded} style="width:{pct(c.confidence)}%"></i></span
          >
          <span class="card__p">{pct(c.confidence)}%</span>
        </span>
        <span class="card__d"
          >{deltaLine({
            history: data.historyOf(c.id),
            confidence: c.confidence,
            faded,
            lastSupportedAtMs: c.lastSupportedAtMs,
          })}</span
        >
      </button>
    {/each}
    {#if data.ordered.length === 0}
      <p class="empty t-meta">Nothing concluded about this subject yet.</p>
    {/if}
  </div>
</div>

<style>
  .col {
    width: 288px;
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: var(--hairline) solid var(--app-border);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    height: 26px;
    flex: 0 0 auto;
    padding: 0 var(--s-10);
    border-bottom: var(--hairline) solid var(--app-border);
  }

  .cards {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    padding: 8px var(--s-10);
    border: 0;
    border-top: var(--hairline) solid var(--app-border);
    background: transparent;
    text-align: left;
    font: inherit;
    cursor: default;
  }

  .card:first-child {
    border-top: 0;
  }

  .card:hover {
    background: var(--app-surface-hover);
  }

  .card:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  .card.is-sel {
    background: var(--app-surface-active);
    box-shadow: inset 2px 0 0 var(--app-accent);
  }

  .card.is-faded {
    opacity: 0.55;
  }

  .card__top {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--app-accent-strong);
  }

  .dot.is-strong {
    background: var(--app-accent);
  }

  .glyph {
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
  }

  .glyph.is-up {
    color: var(--app-accent-strong);
  }

  .glyph.is-down {
    color: var(--app-warn);
  }

  .snap {
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    text-transform: uppercase;
    letter-spacing: var(--ls-label);
    color: var(--app-text-subtle);
  }

  .card__st {
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text);
  }

  .card__m {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }

  .cbar {
    position: relative;
    flex: 1 1 auto;
    height: 4px;
    border-radius: 2px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }

  .cbar i {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--app-accent);
  }

  .cbar i.is-dim {
    background: var(--app-text-faint);
  }

  .card__p {
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }

  .card__d {
    font: var(--w-regular) var(--t-label) / 1.4 var(--app-font-mono);
    color: var(--app-text-faint);
  }

  .empty {
    margin: 0;
    padding: var(--s-12) var(--s-10);
  }
</style>
