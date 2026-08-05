<script lang="ts">
  // Seven days of capture, one bar per day (round-4 decision **G11**).
  //
  // The source is `list_day_coverage` — the SAME cached GROUP BY the jump menu
  // reads (G6's query family). Seven `coveredMs` totals is exactly what seven
  // bars draw; a second aggregation would be this one with a WHERE.
  //
  // A day absent from the response held no capture, so it draws a zero bar. If
  // the whole read failed the tile draws no bars at all — a fake bar chart would
  // be worse than an empty heading.
  import type { DayCoverage } from "$lib/types/app-infra";
  import type { LoadState, Selection } from "./overview-data.svelte";
  import {
    busiestDayPhrase,
    formatDayTitle,
    formatLongHours,
    weekBars,
    weekTotalMs,
    type WeekBar,
  } from "./overview-format";
  import TileShell from "./TileShell.svelte";

  interface Props {
    coverage: LoadState<DayCoverage[]>;
    anchorKey: string;
    selectedKey: string | null;
    /** The 800px floor: bars and the total, no weekday axis. */
    compact?: boolean;
    onselect: (selection: Selection) => void;
    /** Selecting a bar also moves the surface to that day. */
    onpickday: (dayKey: string) => void;
  }

  let { coverage, anchorKey, selectedKey, compact = false, onselect, onpickday }: Props = $props();

  const bars = $derived(coverage.status === "ok" ? weekBars(coverage.value, anchorKey) : []);
  const totalMs = $derived(weekTotalMs(bars));
  const busiest = $derived(busiestDayPhrase(bars));

  const quiet = $derived(
    coverage.status === "failed"
      ? "Couldn't read capture coverage."
      : coverage.status === "loading"
        ? null
        : totalMs === 0
          ? "No capture in the last seven days."
          : null,
  );

  function keyOf(b: WeekBar): string {
    return `week:${b.key}`;
  }

  function pick(b: WeekBar): void {
    onselect({
      key: keyOf(b),
      source: "This week",
      title: formatDayTitle(b.key),
      sections: [
        {
          label: "Day",
          rows: [
            { k: "Captured", v: b.coveredMs > 0 ? formatLongHours(b.coveredMs) : "nothing", mono: true },
            { k: "Share", v: `${Math.round(b.ratio * 100)}% of the busiest day`, mono: true },
          ],
        },
      ],
    });
    if (b.coveredMs > 0) onpickday(b.key);
  }
</script>

<TileShell label="This week" {quiet}>
  {#if bars.length > 0 && totalMs > 0}
    <div class="ss-spark spark" class:spark--compact={compact}>
      {#each bars as b (b.key)}
        <button
          type="button"
          class="bar"
          class:is-on={b.isAnchor}
          class:is-sel={selectedKey === keyOf(b)}
          style="--h:{Math.max(2, Math.round(b.ratio * 100))}%"
          aria-label="{b.label}: {b.coveredMs > 0 ? formatLongHours(b.coveredMs) : 'nothing'}"
          onclick={() => pick(b)}
        ><i></i></button>
      {/each}
    </div>
    {#if !compact}
    <div class="axis">
      {#each bars as b (b.key)}
        <span class="t-label" class:is-on={b.isAnchor}>{b.label}</span>
      {/each}
    </div>
    {/if}
    <div class="ss-trow foot">
      <span class="t-meta is-mono is-num">{formatLongHours(totalMs)}</span>
      {#if busiest}<span class="t-meta ss-r">{busiest}</span>{/if}
    </div>
  {/if}
</TileShell>

<style>
  /* The kit gives `.ss-spark` a 34px height; inside a flex column that is a
     BASIS, not a floor — at the 800px floor it was being squeezed to nothing and
     a seven-day chart with no bars reads as "no capture". Pin it. */
  .spark {
    flex: 0 0 auto;
  }

  .spark--compact {
    height: 22px;
  }

  /* `.ss-spark` sizes its own children; the button is the hit target and the
     inner <i> is the drawn bar, so a 2px bar still has 34px of click area. */
  .bar {
    flex: 1 1 auto;
    height: 100%;
    display: flex;
    align-items: flex-end;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: default;
  }

  .bar i {
    display: block;
    width: 100%;
    height: var(--h);
    border-radius: 1px;
    background: var(--app-text-faint);
  }

  .bar.is-on i {
    background: var(--app-accent);
  }

  .bar.is-sel i {
    background: var(--app-accent);
    box-shadow: 0 0 0 1px var(--app-accent-border);
  }

  .axis {
    display: flex;
    gap: 4px;
  }

  .axis span {
    flex: 1 1 0;
    text-align: center;
    color: var(--app-text-faint);
  }

  .axis span.is-on {
    color: var(--app-accent);
  }

  .foot {
    margin-top: auto;
  }
</style>
