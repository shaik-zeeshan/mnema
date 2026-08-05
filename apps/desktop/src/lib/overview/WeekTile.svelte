<script lang="ts">
  // This week (1×1) — IN per round-4 decision G11, and deliberately fed by the
  // SAME query family as the timeline jump menu (G6): `list_day_coverage` is one
  // cached GROUP BY over `capture_segments`, and seven `coveredMs` totals is
  // exactly what a seven-bar tile draws. A second aggregation would be this one
  // with a WHERE.
  //
  // The payload bleeds into the tile's bottom radius — the bars are cut by the
  // corner rather than sitting inside a padded box.
  import type { DayCoverage } from "$lib/types/app-infra";
  import { capturedLabel, weekBars, weekTotalMs } from "./overview-format";

  let { coverage, loaded }: { coverage: DayCoverage[]; loaded: boolean } = $props();

  const bars = $derived(weekBars(coverage, new Date()));
  const total = $derived(capturedLabel(weekTotalMs(bars)));
</script>

<div class="tile tile--static">
  <div class="tile__h">
    <span class="t-label">This week</span>
    {#if total}<span class="tile__more is-mono is-num">{total}</span>{/if}
  </div>

  {#if total}
    <div class="pay pay--bleed bars">
      <div class="days">
        {#each bars as bar (bar.key)}
          <i class:on={bar.isToday}>{bar.label}</i>
        {/each}
      </div>
      <div class="week">
        {#each bars as bar (bar.key)}
          <span
            class:on={bar.isToday}
            style="height:{Math.round(Math.max(bar.fraction, bar.ms > 0 ? 0.06 : 0) * 100)}%"
          ></span>
        {/each}
      </div>
    </div>
  {:else}
    <div class="pay quiet">
      <span class="t-meta subtle">{loaded ? "No capture in the last seven days" : "Reading…"}</span>
    </div>
  {/if}
</div>

<style>
  .bars {
    margin-left: 0;
  }
  .days {
    position: absolute;
    left: 0;
    right: 0;
    top: -2px;
    display: flex;
    gap: 6px;
  }
  .days i {
    flex: 1 1 0;
    text-align: center;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    font-style: normal;
    color: var(--app-text-faint);
  }
  .days i.on {
    color: var(--app-accent);
  }
  .week {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-end;
    gap: 6px;
  }
  .week span {
    flex: 1 1 0;
    border-radius: 3px 3px 0 0;
    background: var(--app-accent);
    opacity: 0.3;
  }
  .week span.on {
    opacity: 1;
  }
  .quiet {
    display: flex;
    align-items: center;
  }
  .subtle {
    color: var(--app-text-subtle);
  }
</style>
