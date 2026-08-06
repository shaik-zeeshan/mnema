<script lang="ts">
  // The row's hero: one polyline per conclusion, drawn from its real confidence
  // history, plus the 0.15 display floor as a dashed rule.
  //
  // Two rules from the mockup, both load-bearing:
  //   1. The lines encode MAGNITUDE, not identity — the highest-confidence one
  //      is accent, every other is one neutral grey. You read the shape of the
  //      bundle, not which line is which.
  //   2. The x-axis is SNAPSHOT INDEX, not time. Points are evenly spaced
  //      because the backend holds a list of snapshots, not a time series; the
  //      row states "3h ago" separately.
  //
  // ponytail: a local 40-line SVG rather than `insights/charts/Sparkline.svelte`
  // — that one styles strokes with an inline `style` attribute, which the
  // selected row (accent-contrast lines) and the wide hero (its 140px max-width)
  // would both have to fight with `!important`.

  interface TrajectoryLine {
    /** Oldest-first confidence points, 0..1. */
    points: number[];
    /** The subject's / conclusion's leading line — drawn in accent. */
    lead?: boolean;
    faded?: boolean;
  }

  interface Props {
    lines: TrajectoryLine[];
    /** The display floor, drawn dashed. Omit to hide it. */
    floor?: number;
    width?: number;
    height?: number;
    label?: string;
  }

  let { lines, floor = 0.15, width = 120, height = 32, label }: Props = $props();

  const W = 120;
  const H = 32;
  const PAD = 2;

  function y(v: number): number {
    return PAD + (1 - Math.max(0, Math.min(1, v))) * (H - PAD * 2);
  }

  function path(points: number[]): string {
    const n = points.length;
    if (n === 0) return "";
    return points
      .map((v, i) => {
        const x = n <= 1 ? PAD : PAD + (i / (n - 1)) * (W - PAD * 2);
        return `${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y(v).toFixed(1)}`;
      })
      .join(" ");
  }
</script>

<svg
  class="traj"
  style="width:{width}px;height:{height}px"
  viewBox="0 0 {W} {H}"
  preserveAspectRatio="none"
  role="img"
  aria-label={label ?? "Confidence over the recorded snapshots"}
>
  {#if floor !== undefined}
    <path class="floor" d="M0 {y(floor).toFixed(1)} H{W}" />
  {/if}
  {#each lines as line, i (i)}
    {#if line.points.length > 0}
      <path
        class="line"
        class:is-lead={line.lead}
        class:is-faded={line.faded}
        d={path(line.points)}
      />
    {/if}
  {/each}
</svg>

<style>
  .traj {
    flex: 0 0 auto;
    overflow: visible;
  }

  /* Non-scaling strokes: the viewBox is stretched (`preserveAspectRatio: none`)
     so a plain stroke-width would render thicker horizontally than vertically. */
  .traj path {
    fill: none;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }

  .line {
    stroke: var(--app-text-faint);
    opacity: 0.55;
  }

  .line.is-lead {
    stroke: var(--app-accent);
    stroke-width: 1.8;
    opacity: 1;
  }

  .line.is-faded {
    stroke-dasharray: 2 2;
  }

  .floor {
    stroke: var(--app-border-hover);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }
</style>
