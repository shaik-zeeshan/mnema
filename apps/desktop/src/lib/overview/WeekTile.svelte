<script lang="ts">
  // Seven days of capture, one bar per day (G11). Same read as the timeline
  // jump menu — `list_day_coverage`, last seven local days, no second
  // aggregation. Absent days draw a zero bar.
  //
  // ⌃W is the one tile key with no destination behind it: it focuses the tile
  // and stops there (there is no week surface to open). That is a real,
  // visible behaviour, which is why the keycap is allowed to exist — Enter on
  // this tile does nothing, and it draws no chevron to promise otherwise.
  import Tile from "./Tile.svelte";
  import { busiestDay, formatHoursMinutes, weekBars } from "./format";
  import type { Cell } from "./data";
  import type { DayCoverage } from "$lib/types/app-infra";

  interface Props {
    coverage: Cell<DayCoverage[]>;
    now: Date;
    loaded: boolean;
  }

  let { coverage, now, loaded }: Props = $props();

  const bars = $derived(weekBars(coverage.data ?? [], now));
  const peak = $derived(Math.max(1, ...bars.map((b) => b.coveredMs)));
  const total = $derived(bars.reduce((sum, b) => sum + b.coveredMs, 0));
  const busiest = $derived(busiestDay(bars));
</script>

<Tile id="week" title="This week" kbd="⌃W">
  {#if coverage.error}
    <p class="tile-empty t-meta">Coverage unavailable — {coverage.error}</p>
  {:else}
    <div class="wk">
      {#each bars as bar (bar.key)}
        <span class="spark" class:spark--today={bar.isToday}>
          <i style="height:{Math.max(2, (bar.coveredMs / peak) * 100)}%"></i>
        </span>
      {/each}
    </div>
    <div class="wkl">
      {#each bars as bar (bar.key)}
        <span class="t-label" class:wkl--today={bar.isToday}>{bar.label}</span>
      {/each}
    </div>
    <div class="tile-row wk__foot">
      <span class="t-meta is-mono is-num">{formatHoursMinutes(total)}</span>
      {#if loaded && busiest}
        <span class="t-meta wk__busiest">busiest {busiest}</span>
      {/if}
    </div>
  {/if}
</Tile>
