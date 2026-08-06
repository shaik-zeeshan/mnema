<script lang="ts">
  // The row's hero: one line per conclusion, 168px wide — wider than the number
  // beside it, because the SHAPE of the belief is the fact worth reading.
  //
  // Two things page 09 binds and the shipping sparkline doesn't do:
  //   1. X is spaced by TIME, not by point index. Each snapshot already carries
  //      its timestamp; index-spacing stretches a three-point hour and a
  //      64-point month to the same width and makes slope meaningless.
  //   2. The display floor (15% — the real engine constant) is DRAWN, so a line
  //      sinking below "kept for history" is visible rather than implied.
  import { DISPLAY_FLOOR } from "$lib/insights/subjectsTiers";
  import type { SparkSeries } from "./data";

  interface Props {
    series: SparkSeries[];
    label: string;
    width?: number;
    height?: number;
  }

  let { series, label, width = 168, height = 34 }: Props = $props();

  const PAD_X = 4;
  const TOP = 4;
  const BOTTOM = 30;

  const y = (confidence: number): number =>
    Number((BOTTOM - Math.max(0, Math.min(1, confidence)) * (BOTTOM - TOP)).toFixed(2));

  // One shared time window across the subject's conclusions, so two lines that
  // moved at different times don't both get stretched to the full width.
  const window = $derived.by(() => {
    let min = Number.POSITIVE_INFINITY;
    let max = Number.NEGATIVE_INFINITY;
    for (const s of series) {
      for (const p of s.points) {
        if (p.snapshotAtMs < min) min = p.snapshotAtMs;
        if (p.snapshotAtMs > max) max = p.snapshotAtMs;
      }
    }
    return Number.isFinite(min) && max > min ? { min, span: max - min } : null;
  });

  function path(s: SparkSeries): string {
    const inner = width - 2 * PAD_X;
    return s.points
      .map((p, i) => {
        // No window (every point at the same instant) — spread evenly so a
        // single-snapshot conclusion still reads as a flat line, not a dot.
        const t = window
          ? (p.snapshotAtMs - window.min) / window.span
          : s.points.length > 1
            ? i / (s.points.length - 1)
            : 0;
        return `${(PAD_X + t * inner).toFixed(2)},${y(p.confidence)}`;
      })
      .join(" ");
  }

  const floorY = y(DISPLAY_FLOOR);
</script>

<svg
  class="spk"
  viewBox="0 0 {width} {height}"
  preserveAspectRatio="none"
  role="img"
  aria-label={label}
>
  <line class="spk__floor" x1="0" y1={floorY} x2={width} y2={floorY} />
  {#each series as s, i (i)}
    <polyline
      class="spk__line {s.faded ? 'spk__fade' : i === 0 ? 'spk__lead' : 'spk__rest'}"
      points={path(s)}
    />
  {/each}
</svg>

<style>
  .spk {
    flex: 0 0 auto;
    display: block;
    width: 168px;
    height: 30px;
  }
  .spk__floor {
    stroke: var(--app-text-faint);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }
  .spk__line {
    fill: none;
    stroke-linecap: round;
    stroke-linejoin: round;
    /* preserveAspectRatio="none" scales the stroke with the box; keep the line
       weight honest by drawing in the unscaled user space. */
    vector-effect: non-scaling-stroke;
  }
  .spk__lead {
    stroke: var(--app-accent);
    stroke-width: 1.6;
  }
  .spk__rest {
    stroke: var(--app-text-subtle);
    stroke-width: 1;
    opacity: 0.55;
  }
  .spk__fade {
    stroke: var(--app-text-subtle);
    stroke-width: 1;
    opacity: 0.4;
    stroke-dasharray: 2 2;
  }
</style>
