<script lang="ts">
  // The feature cards' stat grid (mockup A): a row of label / value / sub cells.
  //
  // ponytail: one shared grid for the five slice-6 cards, not a card framework.
  // Everything else on a card (rows, error lines, actions) is plain markup over
  // the existing `.kv-list` / `.badge` / `.btn` classes.

  import type { DebugStat } from "../format";

  interface Props {
    stats: DebugStat[];
    /** Grid columns. Mockup A's default card is a 4-stat grid. */
    columns?: number;
  }

  let { stats, columns = 4 }: Props = $props();
</script>

<div class="debug-stats" style="--debug-stats-cols: {columns}">
  {#each stats as stat (stat.key)}
    <div class="debug-stat">
      <div class="debug-stat__k">
        {stat.label}{#if stat.isNew}<span class="new-chip">new</span>{/if}
      </div>
      <div class="debug-stat__v" class:debug-stat__v--ok={stat.tone === "ok"} class:debug-stat__v--warn={stat.tone === "warn"} class:debug-stat__v--err={stat.tone === "err"}>
        {stat.value}{#if stat.unit}<small>{stat.unit}</small>{/if}
      </div>
      {#if stat.sub}
        <div class="debug-stat__sub">{stat.sub}</div>
      {/if}
    </div>
  {/each}
</div>
