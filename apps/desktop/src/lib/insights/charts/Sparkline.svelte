<script lang="ts">
  // Sparkline — compact multi-line micro-trajectory for cards. Values are 0..1
  // and plotted across a small fixed viewBox; the x-axis is just the point index
  // (evenly spaced). Optional faded styling per series and an optional dashed
  // floor line. No axes/labels — purely a glanceable trend.
  // Props:
  //   series: { colorVar: string; faded?: boolean; points: number[] }[]
  //           points are confidence values 0..1.
  //   floor?: number   — display-floor fraction 0..1; omit/undefined to hide.

  interface SparkSeries {
    colorVar: string;
    faded?: boolean;
    points: number[];
  }
  interface Props {
    series: SparkSeries[];
    floor?: number;
    // Contextual accessible label (e.g. the subject name + trend). When omitted a
    // generic trend description is derived from the lead series so the chart is
    // never announced as a bare "Trend".
    label?: string;
  }

  let { series, floor, label }: Props = $props();

  // Describe the lead (first) series' direction for the fallback aria-label, so
  // an unlabelled sparkline still announces a meaningful trend, not just "Trend".
  function derivedTrendLabel(): string {
    const lead = series.find((s) => s.points.length >= 2);
    if (!lead) return "Trend (no movement)";
    const first = lead.points[0];
    const last = lead.points[lead.points.length - 1];
    const delta = last - first;
    if (delta > 0.04) return "Trend rising";
    if (delta < -0.04) return "Trend falling";
    return "Trend steady";
  }

  const ariaLabel = $derived(label ?? derivedTrendLabel());

  const W = 120;
  const H = 32;
  const PAD = 2;

  function x(index: number, count: number): number {
    if (count <= 1) return PAD;
    return PAD + (index / (count - 1)) * (W - PAD * 2);
  }
  function y(value: number): number {
    const v = Math.max(0, Math.min(1, value));
    return PAD + (1 - v) * (H - PAD * 2);
  }
  function pointsAttr(points: number[]): string {
    return points
      .map((v, i) => `${x(i, points.length).toFixed(1)},${y(v).toFixed(1)}`)
      .join(" ");
  }
</script>

<svg
  class="sparkline"
  viewBox="0 0 {W} {H}"
  preserveAspectRatio="none"
  role="img"
  aria-label={ariaLabel}
>
  {#if floor !== undefined}
    <line class="floor" x1={PAD} y1={y(floor)} x2={W - PAD} y2={y(floor)} />
  {/if}
  {#each series as s, i (i)}
    {#if s.points.length > 0}
      <polyline
        class="line"
        class:line--faded={s.faded}
        points={pointsAttr(s.points)}
        style="stroke:var({s.colorVar});"
      />
    {/if}
  {/each}
</svg>

<style>
  /* Sized by the caller (09's row hero is 172 x 46); the viewBox stretches
     because preserveAspectRatio is none. */
  .sparkline {
    width: 100%;
    height: 100%;
    display: block;
  }
  /* The display floor — the ONE dashed line in the chart, so a trajectory
     crossing it reads as crossing something. */
  .floor {
    stroke: var(--app-text-faint);
    stroke-width: 1;
    stroke-dasharray: 2 3;
    opacity: 0.7;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  /* A faded conclusion's line stays SOLID and just recedes — dashing it would
     collide with the floor, the only dashed thing here. */
  .line--faded {
    opacity: 0.45;
  }
</style>
