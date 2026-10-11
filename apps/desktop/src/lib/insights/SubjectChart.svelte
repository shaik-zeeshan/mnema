<script lang="ts">
  // Subject detail "Confidence over time" (insights.html `.in-cc`): every
  // conclusion's Confidence History on one time axis (earliest formation →
  // now). The selected line is strong with a 7% area, the rest grey; one dot
  // per supporting evidence on the selected line, an accent dot at formation.
  // y ticks are the shared Subjects cuts plus the dashed display floor.
  import type { Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import { tip } from "$lib/components/tooltip";
  import { chartPoints, chartX, interpolateConfidence, type ChartPoint } from "$lib/insights/subjectTimeline";
  import { DISPLAY_FLOOR, INITIAL_BASE, STRONGLY_HELD } from "$lib/insights/subjectsTiers";

  interface Props {
    conclusions: Conclusion[];
    histories: Map<number, ConfidenceSnapshot[]>;
    selectedId: number | null;
  }
  let { conclusions, histories, selectedId }: Props = $props();

  const W = 600;
  const H = 230;
  const pct = (v: number) => Math.round(v * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  // A conclusion with no retained history still draws one point at formation.
  const historyOf = (c: Conclusion): ConfidenceSnapshot[] => {
    const h = histories.get(c.id);
    return h?.length ? h : [{ snapshotAtMs: c.formedAtMs, confidence: c.confidence }];
  };

  const toMs = Date.now();
  const fromMs = $derived(
    Math.min(toMs, ...conclusions.map((c) => Math.min(c.formedAtMs, historyOf(c)[0].snapshotAtMs))),
  );

  const path = (pts: ChartPoint[]) =>
    pts.map((p, i) => `${i ? "L" : "M"}${(p.x * W).toFixed(1)} ${((1 - p.y) * H).toFixed(1)}`).join(" ");

  const selected = $derived(conclusions.find((c) => c.id === selectedId) ?? null);
  const selPts = $derived(selected ? chartPoints(historyOf(selected), fromMs, toMs) : []);
  const area = $derived(
    selPts.length ? `${path(selPts)} L${(selPts[selPts.length - 1].x * W).toFixed(1)} ${H} L${(selPts[0].x * W).toFixed(1)} ${H}Z` : "",
  );

  // Dots on the selected line: its formation (accent), then each supporting
  // Activity at its start time, read off the line.
  const dots = $derived.by(() => {
    if (!selected) return [];
    const h = historyOf(selected);
    const at = (ms: number) => ({ x: chartX(ms, fromMs, toMs), y: interpolateConfidence(h, ms, selected.confidence) });
    const out = [{ ...at(selected.formedAtMs), origin: true, label: `Formed · ${pct(at(selected.formedAtMs).y)}%` }];
    for (const ref of selected.evidence) {
      if (ref.stance !== "support" || ref.activityStartedAtMs == null) continue;
      const p = at(ref.activityStartedAtMs);
      out.push({ ...p, origin: false, label: `${ref.activityTitle ?? "Activity"} · ${pct(p.y)}%` });
    }
    return out;
  });

  const midMonth = $derived(new Date((fromMs + toMs) / 2).toLocaleDateString(undefined, { month: "short" }));
  const TICKS = [100, pct(STRONGLY_HELD), pct(INITIAL_BASE), pct(DISPLAY_FLOOR), 0];
  const GRID = [pct(INITIAL_BASE), pct(STRONGLY_HELD), 100];
  const y = (v: number) => H - (v / 100) * H;
</script>

<div class="in-cc">
  <div class="in-cc__y">
    {#each TICKS as v (v)}<span style:bottom="{v}%">{v}</span>{/each}
  </div>
  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
    {#each GRID as v (v)}
      <line x1="0" x2={W} y1={y(v)} y2={y(v)} stroke="var(--mx-hairline)" vector-effect="non-scaling-stroke" />
    {/each}
    <line x1="0" x2={W} y1={H} y2={H} stroke="var(--app-border)" vector-effect="non-scaling-stroke" />
    <line
      x1="0"
      x2={W}
      y1={y(pct(DISPLAY_FLOOR))}
      y2={y(pct(DISPLAY_FLOOR))}
      stroke="var(--app-border-strong)"
      stroke-dasharray="3 4"
      vector-effect="non-scaling-stroke"
    />
    {#each conclusions as c (c.id)}
      {#if c.id !== selectedId}
        <path d={path(chartPoints(historyOf(c), fromMs, toMs))} stroke="var(--chart-grey-2)" vector-effect="non-scaling-stroke" />
      {/if}
    {/each}
    {#if selPts.length}
      <path d={area} fill="color-mix(in srgb, var(--app-text-strong) 7%, transparent)" stroke="none" />
      <path d={path(selPts)} stroke="var(--app-text-strong)" stroke-width="2" vector-effect="non-scaling-stroke" />
    {/if}
  </svg>
  <span class="in-cc__fade" style:top="{100 - pct(DISPLAY_FLOOR)}%">display floor {pct(DISPLAY_FLOOR)}%</span>
  {#each dots as d, i (i)}
    <i
      class="in-cc__ev"
      class:in-cc__ev--origin={d.origin}
      style:left="{d.x * 100}%"
      style:top="{(1 - d.y) * 100}%"
      style:--d={i}
      use:tip={d.label}
    ></i>
  {/each}
</div>
<div class="in-cc__x"><span>{day(fromMs)}</span><span>{midMonth}</span><span>today</span></div>

<style>
  /* insights.html `.in-cc`. */
  .in-cc { position: relative; height: 230px; margin: var(--s-5) 0 0 34px; }
  .in-cc svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
  .in-cc path { fill: none; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .in-cc__y { position: absolute; left: -34px; top: 0; bottom: 0; width: 26px; }
  .in-cc__y span { position: absolute; right: 0; translate: 0 50%; font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-faint); }
  .in-cc__ev {
    position: absolute; width: 9px; height: 9px; margin: -4.5px 0 0 -4.5px; border-radius: 50%;
    background: var(--app-surface); border: 2px solid var(--app-text-strong); cursor: default;
    animation: in-fade var(--t-slow) var(--ease-quart) both; animation-delay: calc(300ms + var(--d) * 50ms);
    transition: transform var(--t-fast) var(--ease-expo);
  }
  .in-cc__ev:hover { transform: scale(1.35); }
  .in-cc__ev--origin { border-color: var(--app-accent); background: var(--app-accent); box-shadow: 0 0 0 3px var(--app-accent-glow); }
  .in-cc__fade { position: absolute; right: 0; translate: 0 -120%; font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-faint); }
  .in-cc__x { display: flex; justify-content: space-between; margin: var(--s-2) 0 0 34px; font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-faint); }
  @keyframes in-fade { from { opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .in-cc__ev { animation: none; } }
</style>
