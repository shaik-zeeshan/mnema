<script lang="ts">
  // SubjectRow — one row of the Subjects index (page 09).
  //
  // The row is a TRAJECTORY, not a record: the sparkline is the widest, tallest,
  // only-drawn thing in it — the hero — and the mono number beside it is the
  // sparkline's CAPTION. That number is the TOP CONCLUSION's confidence (no
  // rolled-up subject score exists), which is why it never gets a bar of its
  // own. The dashed line inside the sparkline is the 0.15 display floor, so a
  // fading subject's trajectory is drawn CROSSING it rather than deleted.
  //
  // The whole row is ONE button: one tab stop, one hit area, and the chevron
  // says where it goes.
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import { DISPLAY_FLOOR } from "$lib/insights/subjectsTiers";
  import {
    type SubjectRow,
    TREND_GLYPH,
    countLabel,
    metaLabel,
    trendClass,
    trendWord,
  } from "$lib/insights/subject-rows";

  interface Props {
    row: SubjectRow;
    onOpen: (subject: string) => void;
  }

  let { row, onOpen }: Props = $props();
</script>

<button
  type="button"
  class="srow"
  class:is-faded={row.faded}
  onclick={() => onOpen(row.subject)}
>
  <span class="srow__t">
    <span class="n"
      >{row.subject}{#if row.pinned}<span
          class="pin"
          title="Has a pinned conclusion">★</span
        >{/if}</span
    >
    <span class="s">{row.headline}</span>
    <span class="m">
      <span class="meta">{metaLabel(row)}</span>
      {#if row.belowFloorCount > 0}
        <span class="meta">{row.belowFloorCount} below floor</span>
      {/if}
    </span>
  </span>

  <span class="hero">
    <Sparkline
      series={row.spark}
      floor={DISPLAY_FLOOR}
      label={`${row.subject} — confidence trajectory, ${trendWord(row.trend)}, across ${countLabel(row.conclusionCount)}`}
    />
  </span>
  <span class="conf is-num" class:dim={row.faded}
    >{row.topConfidence.toFixed(2)}</span
  >
  <span class="trend {trendClass(row.trend)}" aria-hidden="true"
    >{TREND_GLYPH[row.trend]}</span
  >
  <span class="chev" aria-hidden="true">›</span>
</button>

<style>
  .is-num {
    font-variant-numeric: tabular-nums;
  }

  .srow {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 14px;
    min-height: 52px;
    padding: 9px 0;
    border: 0;
    background: transparent;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: var(--r-md);
    transition: background-color var(--dur-quick) var(--ease);
  }
  .srow:hover {
    background: var(--app-surface-hover);
  }
  .srow:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  .srow__t {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .srow__t .n {
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .srow__t .s {
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .srow__t .m {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-top: 1px;
  }
  .srow__t .meta {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .pin {
    margin-left: var(--s-6);
    color: var(--app-accent);
  }
  .srow.is-faded .srow__t .n,
  .srow.is-faded .srow__t .s {
    color: var(--app-text-faint);
  }

  /* THE HERO. */
  .hero {
    width: 172px;
    height: 46px;
    flex: 0 0 auto;
    display: block;
  }
  /* Its caption — mono, tabular, hard against the sparkline's end. */
  .conf {
    width: 44px;
    flex: 0 0 auto;
    text-align: right;
    font: var(--w-semi) var(--t-title) / 1 var(--app-font-mono);
    color: var(--app-text-strong);
  }
  .conf.dim {
    font-weight: var(--w-regular);
    color: var(--app-text-faint);
  }
  .trend {
    width: 16px;
    flex: 0 0 auto;
    text-align: center;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
  }
  .tr-up {
    color: var(--app-accent);
  }
  .tr-st {
    color: var(--app-text-subtle);
  }
  /* Cooling is normal decay, not an error — it stays quiet. */
  .tr-dn {
    color: var(--app-text-faint);
  }
  .chev {
    flex: 0 0 auto;
    width: 12px;
    text-align: center;
    font-size: var(--t-title);
    line-height: 1;
    color: var(--app-text-faint);
  }
  .srow:hover .chev {
    color: var(--app-text-muted);
  }

  @media (prefers-reduced-motion: reduce) {
    .srow {
      transition: none;
    }
  }
</style>
