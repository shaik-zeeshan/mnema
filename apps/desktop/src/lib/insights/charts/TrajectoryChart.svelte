<script lang="ts">
  // TrajectoryChart — multi-line confidence-over-time chart, hand-built SVG.
  // Renders horizontal gridlines at 0/25/50/75/100%, a dashed "display floor"
  // line (default 15%), a time x-axis with start/end labels, one polyline per
  // series, and a dot at each series' latest point. Confidence values are 0..1.
  // Faded series (below the display floor) render dimmed.
  // Props:
  //   series: {
  //     id: number | string;
  //     label: string;
  //     colorVar: string;        // CSS var NAME, e.g. "--cat-coding"
  //     faded?: boolean;
  //     points: { atMs: number; confidence: number }[]  // confidence 0..1
  //   }[]
  //   floor?: number             // display-floor fraction 0..1 (default 0.15)

  interface TrajectoryPoint {
    atMs: number;
    confidence: number;
  }
  interface TrajectorySeries {
    id: number | string;
    label: string;
    colorVar: string;
    faded?: boolean;
    points: TrajectoryPoint[];
  }
  interface Props {
    series: TrajectorySeries[];
    floor?: number;
  }

  let { series, floor = 0.15 }: Props = $props();

  // viewBox coordinate space; the SVG scales responsively via width:100%.
  const W = 600;
  const H = 240;
  const PAD_L = 34;
  const PAD_R = 12;
  const PAD_T = 12;
  const PAD_B = 24;
  const plotW = W - PAD_L - PAD_R;
  const plotH = H - PAD_T - PAD_B;

  const gridLevels = [0, 0.25, 0.5, 0.75, 1];

  const allPoints = $derived(series.flatMap((s) => s.points));
  const minMs = $derived(
    allPoints.length ? Math.min(...allPoints.map((p) => p.atMs)) : 0,
  );
  const maxMs = $derived(
    allPoints.length ? Math.max(...allPoints.map((p) => p.atMs)) : 1,
  );
  const span = $derived(Math.max(1, maxMs - minMs));

  function x(atMs: number): number {
    return PAD_L + ((atMs - minMs) / span) * plotW;
  }
  function y(confidence: number): number {
    const c = Math.max(0, Math.min(1, confidence));
    return PAD_T + (1 - c) * plotH;
  }

  function pointsAttr(pts: TrajectoryPoint[]): string {
    return pts.map((p) => `${x(p.atMs).toFixed(1)},${y(p.confidence).toFixed(1)}`).join(" ");
  }

  function lastPoint(pts: TrajectoryPoint[]): TrajectoryPoint | null {
    return pts.length ? pts[pts.length - 1] : null;
  }

  function fmtTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
</script>

<div class="trajectory">
  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label="Confidence over time">
    <!-- gridlines + y labels -->
    {#each gridLevels as level (level)}
      <line
        class="grid"
        x1={PAD_L}
        y1={y(level)}
        x2={W - PAD_R}
        y2={y(level)}
      />
      <text class="ylabel" x={PAD_L - 6} y={y(level) + 3} text-anchor="end">
        {Math.round(level * 100)}%
      </text>
    {/each}

    <!-- display-floor dashed line -->
    <line
      class="floor"
      x1={PAD_L}
      y1={y(floor)}
      x2={W - PAD_R}
      y2={y(floor)}
    />

    <!-- series -->
    {#each series as s (s.id)}
      {#if s.points.length > 0}
        <polyline
          class="line"
          class:line--faded={s.faded}
          points={pointsAttr(s.points)}
          style="stroke:var({s.colorVar});"
        />
        {#each [lastPoint(s.points)] as last}
          {#if last}
            <circle
              class="dot"
              class:dot--faded={s.faded}
              cx={x(last.atMs)}
              cy={y(last.confidence)}
              r="3"
              style="fill:var({s.colorVar});"
            />
          {/if}
        {/each}
      {/if}
    {/each}
  </svg>
  <div class="xaxis">
    <span>{fmtTime(minMs)}</span>
    <span>{fmtTime(maxMs)}</span>
  </div>
</div>

<style>
  .trajectory {
    width: 100%;
  }
  svg {
    width: 100%;
    height: auto;
    display: block;
  }
  .grid {
    stroke: var(--app-border);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .ylabel {
    fill: var(--app-text-faint);
    font-family: inherit;
    font-size: 9px;
  }
  .floor {
    stroke: var(--app-text-subtle);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke-width: 1.75;
    vector-effect: non-scaling-stroke;
  }
  .line--faded {
    opacity: 0.4;
    stroke-dasharray: 2 3;
  }
  .dot {
    stroke: var(--app-bg);
    stroke-width: 1;
  }
  .dot--faded {
    opacity: 0.4;
  }
  .xaxis {
    display: flex;
    justify-content: space-between;
    margin-top: 2px;
    padding: 0 2px;
    font-size: 9.5px;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }
</style>
