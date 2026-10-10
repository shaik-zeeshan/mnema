<script lang="ts">
  // The Deep / Mixed / Scattered split bar + legend (insights.html `.in-dist`),
  // by activity COUNT — the same computation as the "Deep focus" stat.
  import { tip } from "$lib/components/tooltip";
  import type { FocusCounts } from "$lib/insights/lede-stats";

  let { counts }: { counts: FocusCounts } = $props();

  const BANDS = [
    ["deep", "Deep", "--focus-deep"],
    ["mixed", "Mixed", "--focus-mid"],
    ["distracted", "Scattered", "--focus-distracted"],
  ] as const;
  const pct = (k: "deep" | "mixed" | "distracted") => Math.round((counts[k] / Math.max(1, counts.n)) * 100);
</script>

<div class="dist">
  {#each BANDS as [k, label, v] (k)}
    {#if counts[k]}
      <i style="--c:var({v});flex:{counts[k]}" use:tip={`${label} · ${pct(k)}% · ${counts[k]} of ${counts.n} activities`}></i>
    {/if}
  {/each}
</div>
<div class="dist-l">
  {#each BANDS as [k, label, v] (k)}
    <span><span class="sw" style="--c:var({v})"></span>{label} <b>{pct(k)}%</b></span>
  {/each}
  <span class="of">of activities</span>
</div>

<style>
  .dist {
    display: flex;
    gap: 2px;
    height: 8px;
    margin: var(--s-3) 0 6px;
  }
  .dist i {
    background: var(--c);
    border-radius: 2px;
    transform-origin: left;
    animation: grow-x var(--t-slow) var(--ease-expo) both;
    animation-delay: 240ms;
  }
  .dist-l {
    display: flex;
    gap: var(--s-4);
    font: 500 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .dist-l > span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dist-l b {
    color: var(--app-text-strong);
    font: 500 var(--text-sm) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .dist-l .of {
    margin-left: auto;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }
  @keyframes grow-x {
    from {
      transform: scaleX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dist i {
      animation: none;
    }
  }
</style>
