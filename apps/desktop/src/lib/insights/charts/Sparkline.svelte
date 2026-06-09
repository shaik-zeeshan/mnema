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
  }

  let { series, floor }: Props = $props();

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
  aria-label="Trend"
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
  .sparkline {
    width: 100%;
    height: auto;
    max-width: 140px;
    display: block;
  }
  .floor {
    stroke: var(--app-text-subtle);
    stroke-width: 1;
    stroke-dasharray: 2 2;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  .line--faded {
    opacity: 0.4;
    stroke-dasharray: 2 2;
  }
</style>
