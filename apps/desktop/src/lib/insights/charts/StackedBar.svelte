<script lang="ts">
  // StackedBar — "Where the <unit> went" (insights.html `.in-week__plot`): one
  // stacked column per slot (a day in week/month, an hour in day), each
  // stacked by category in the order given. Empty slots draw as a dashed zero
  // column so a day without capture still reads as a day. `highlight` dims every
  // part but that key (category-row hover).
  import { tip } from "$lib/components/tooltip";
  import { fmtDuration, type Column, type PeriodUnit } from "$lib/insights/lede-stats";

  interface Props {
    columns: Column[];
    unit: PeriodUnit;
    highlight?: string | null;
  }

  let { columns, unit, highlight = null }: Props = $props();

  const H = 36e5;
  const month = $derived(unit === "month");
  const peak = $derived(Math.max(0, ...columns.map((c) => c.totalMs / H)));
  // Day mode can pass 1h per column when activities overlap — the axis grows to fit.
  const max = $derived(unit === "day" ? Math.max(1, Math.ceil(peak * 4) / 4) : Math.max(8, Math.ceil(peak / 2) * 2));
  const ticks = $derived([max, max * 0.75, max / 2, max / 4, 0]);
  const tick = (v: number) => (unit === "day" ? `${Math.round(v * 60)}m` : `${Math.round(v)}h`);
  const gap = $derived(month ? "4px" : "var(--s-3)");
</script>

<div class="plot" style="--n:{columns.length || 1};--_g:{gap}" class:dim={highlight != null}>
  <div class="axis" aria-hidden="true">
    {#each ticks as v, i (i)}<span style="bottom:{(v / max) * 100}%">{tick(v)}</span>{/each}
  </div>
  <div class="grid" aria-hidden="true">
    {#each ticks.slice(0, 4) as v, i (i)}<i style="bottom:{(v / max) * 100}%"></i>{/each}
  </div>
  {#each columns as c, di (c.startMs)}
    {#if c.totalMs <= 0}
      <div class="col">
        <div
          class="col-empty"
          class:faint={c.future && month}
          use:tip={`${c.tip} · ${c.future ? "not yet" : "nothing captured"}`}
        ></div>
      </div>
    {:else}
      <div
        class="col"
        class:today={c.live && unit === "week"}
        style="height:{(c.totalMs / H / max) * 100}%;align-self:end"
      >
        {#each c.parts as p, i (p.key)}
          <i
            class:on={highlight === p.key}
            style="--c:var({p.colorVar});flex:{p.ms};--d:{di * 2 + i * 0.3}"
            use:tip={`${p.label} · ${fmtDuration(p.ms)} · ${c.tip} · ${Math.round((p.ms / c.totalMs) * 100)}% of ${fmtDuration(c.totalMs)}`}
          ></i>
        {/each}
      </div>
    {/if}
  {/each}
</div>
<div class="days" style="--n:{columns.length || 1};--_g:{gap}">
  <span></span>
  {#each columns as c (c.startMs)}
    {@const weekend = month && [0, 6].includes(new Date(c.startMs).getDay())}
    <span class:faint={weekend}
      ><b>{c.label}</b>{#if !month}{c.totalMs > 0
          ? fmtDuration(c.totalMs) + (c.live && unit === "day" ? " so far" : "")
          : "—"}{/if}</span
    >
  {/each}
</div>

<style>
  .plot {
    display: grid;
    grid-template-columns: 34px repeat(var(--n), 1fr);
    gap: 0 var(--_g);
    height: 210px;
    position: relative;
    margin-top: var(--s-4);
  }
  .axis {
    position: relative;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-faint);
  }
  .axis span {
    position: absolute;
    right: 0;
    transform: translateY(50%);
  }
  .grid {
    position: absolute;
    left: 46px;
    right: 0;
    top: 0;
    bottom: 0;
    pointer-events: none;
  }
  .grid i {
    position: absolute;
    left: 0;
    right: 0;
    height: 1px;
    background: var(--mx-hairline);
  }
  .col {
    position: relative;
    display: flex;
    flex-direction: column-reverse;
    gap: 2px;
    height: 100%;
  }
  .col > i {
    display: block;
    background: var(--c);
    transform-origin: bottom;
    animation: grow-y var(--t-slow) var(--ease-expo) both;
    animation-delay: calc(120ms + var(--d) * 40ms);
    transition: opacity var(--t-fast) var(--ease-quart);
  }
  .col > i:last-child {
    border-radius: 4px 4px 0 0;
  }
  .plot:hover .col > i,
  .plot.dim .col > i {
    opacity: 0.45;
  }
  .plot.dim .col > i:not(.on) {
    opacity: 0.2;
  }
  .plot .col > i:hover,
  .plot.dim .col > i.on {
    opacity: 1;
  }
  .col-empty {
    border: 1px dashed var(--app-border);
    border-radius: var(--r-xs);
    height: 24px;
  }
  .col-empty.faint {
    opacity: 0.4;
  }
  .today::after {
    content: "live";
    position: absolute;
    top: -18px;
    left: 50%;
    translate: -50% 0;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-accent);
  }
  .days {
    display: grid;
    grid-template-columns: 34px repeat(var(--n), 1fr);
    gap: var(--_g);
    margin-top: var(--s-2);
    font: 500 var(--text-xs) / 1.3 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .days span {
    text-align: center;
  }
  .days b {
    display: block;
    color: var(--app-text);
    font-weight: 500;
  }
  .days .faint {
    color: var(--app-text-faint);
  }
  @keyframes grow-y {
    from {
      transform: scaleY(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .col > i {
      animation: none;
    }
  }
</style>
