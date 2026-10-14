<script lang="ts">
  import { formatClockRange, formatHour } from "$lib/time-format";
  // Heatmap — the focus heat (insights.html `.in-heat`): rows are captured days
  // (day / week) or ISO weeks (month), twelve 2-hour cells across the local day.
  // Hue = the cell's focus band, depth = how much activity it held.
  import { tip } from "$lib/components/tooltip";
  import { focusHint } from "$lib/insights/activity-helpers";
  import { fmtDuration, type HeatRow } from "$lib/insights/lede-stats";

  let { rows }: { rows: HeatRow[] } = $props();

  const VAR = { deep: "--focus-deep", mixed: "--focus-mid", distracted: "--focus-distracted" } as const;
  // Cell i covers [2i, 2i+2) local hours.
  const at = (i: number) => new Date(2000, 0, 1, i * 2);
</script>

<div class="heat">
  <span></span>
  {#each Array.from({ length: 12 }) as _, i (i)}<span class="h">{i % 3 === 0 ? formatHour(i * 2) : ""}</span>{/each}
  {#each rows as r, ri (ri)}
    <span>{r.label}</span>
    {#each r.cells as c, i (i)}
      {#if c}
        <i
          style="--c:color-mix(in srgb, var({VAR[c.band]}) {c.minutes >= 60 ? 88 : 42}%, var(--app-surface));--d:{ri * 12 + i}"
          use:tip={`${focusHint(c.band)} · ${fmtDuration(c.minutes * 60000)} · ${r.tip} ${formatClockRange(at(i), at(i + 1))}`}
        ></i>
      {:else}
        <i style="--d:{ri * 12 + i}"></i>
      {/if}
    {/each}
  {/each}
</div>

<style>
  .heat {
    display: grid;
    grid-template-columns: 30px repeat(12, 1fr);
    gap: 3px;
    margin-top: var(--s-4);
  }
  .heat span {
    font: 500 var(--text-xs) / 16px var(--font-mono);
    color: var(--app-text-subtle);
  }
  .heat .h {
    color: var(--app-text-faint);
  }
  .heat i {
    height: 20px;
    border-radius: 3px;
    background: var(--c, color-mix(in srgb, var(--app-fg) 4%, transparent));
    animation: fade var(--t-slow) var(--ease-quart) both;
    animation-delay: calc(200ms + var(--d) * 12ms);
    transition: transform var(--t-fast) var(--ease-expo);
  }
  .heat i[style*="--c"]:hover {
    transform: scale(1.18);
    box-shadow: 0 0 0 2px var(--app-bg);
    position: relative;
    z-index: 1;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .heat i {
      animation: none;
    }
  }
</style>
