<script lang="ts">
  // ══ CONFIDENCE TRACE ═══════════════════════════════════════════════════════
  //
  // One conclusion's confidence-over-time line, in a well. Direction 05 reads
  // it as an instrument face: a measured quantity (confidence, 0–1) plotted
  // against a printed constant (the engine's 0.15 display floor, drawn dashed).
  // It never turns — Overview instruments only READ.
  //
  // **The x-axis is real time** (`snapshotAtMs`), never point index. Snapshots
  // are written on irregular events (a distillation that raised confidence, a
  // decay pass), so evenly spacing them would misstate the sampling — a
  // six-week-old trace has to look six weeks long. This is an explicit
  // correction to the mockup's evenly-spaced specimen polylines.
  //
  // Dumb and presentational: it states no rate, no forecast, no trend arrow.
  // Fewer than two points renders nothing at all (G8 — never invent a shape).
  //
  // The domain defaults to each trace's OWN first→last snapshot, so spacing
  // inside one trace is honest. Pass `domain` to plot a whole LIST of traces on
  // one shared clock — that is what makes "a six-week-old trace looks six weeks
  // long" true across rows rather than only within one.
  import type { ConfidenceSnapshot } from "$lib/types/recording";

  interface Props {
    history: ConfidenceSnapshot[];
    /** Other conclusions on the same subject, drawn faint behind the lead. */
    others?: ConfidenceSnapshot[][];
    /** Shared time domain `[startMs, endMs]`; omitted = this trace's own span. */
    domain?: [number, number];
    /** The engine's display floor, drawn as the dashed reference line. */
    floor?: number;
    size?: "tile" | "row" | "hero";
    label?: string;
  }

  let {
    history,
    others = [],
    domain,
    floor = 0.15,
    size = "tile",
    label,
  }: Props = $props();

  // A fixed viewBox stretched by `preserveAspectRatio="none"`; strokes stay
  // 1px via `vector-effect`, so one geometry serves all three sizes.
  const W = 120;
  const H = 32;
  const PAD = 2;

  const clamp01 = (n: number): number => Math.min(1, Math.max(0, n));
  const y = (confidence: number): number =>
    H - PAD - clamp01(confidence) * (H - PAD * 2);

  const byTime = (h: ConfidenceSnapshot[]): ConfidenceSnapshot[] =>
    [...h].sort((a, b) => a.snapshotAtMs - b.snapshotAtMs);

  // [startMs, endMs] the x-axis maps across: the caller's shared clock when
  // given, else this trace's own first→last snapshot.
  const window = $derived.by<[number, number] | null>(() => {
    if (domain && domain[1] > domain[0]) return domain;
    const sorted = byTime(history);
    if (sorted.length < 2) return null;
    return [sorted[0].snapshotAtMs, sorted[sorted.length - 1].snapshotAtMs];
  });

  function polyline(h: ConfidenceSnapshot[]): string {
    const sorted = byTime(h);
    if (sorted.length < 2 || !window) return "";
    const [t0, t1] = window;
    const span = t1 - t0 || 1;
    return sorted
      .map((point) => {
        const raw = PAD + ((point.snapshotAtMs - t0) / span) * (W - PAD * 2);
        // A shared domain can be narrower than a stray snapshot; clamp rather
        // than let a line escape the well.
        const x = Math.min(W - PAD, Math.max(PAD, raw));
        return `${x.toFixed(2)},${y(point.confidence).toFixed(2)}`;
      })
      .join(" ");
  }

  const points = $derived(polyline(history));
  // One faint line per other conclusion about the same subject — the row states
  // "this subject holds several beliefs" without fanning into a rainbow.
  const otherPoints = $derived(
    others.map((h) => polyline(h)).filter((p) => p !== ""),
  );

  // At hero size the line alone is too thin to carry the shape, so the area
  // under it is filled — the same polyline closed to the baseline.
  const area = $derived(
    size === "hero" && points ? `${PAD},${H} ${points} ${W - PAD},${H}` : "",
  );
</script>

{#if points}
  <span class="ti-well trace trace--{size}" role="img" aria-label={label ?? "Confidence over time"}>
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
      {#if area}<polygon class="tr-area" points={area} />{/if}
      <line class="tr-floor" x1={PAD} y1={y(floor)} x2={W - PAD} y2={y(floor)} />
      {#each otherPoints as other, i (i)}
        <polyline class="tr-oth" points={other} />
      {/each}
      <polyline class="tr-lead" {points} />
    </svg>
  </span>
{/if}

<style>
  .trace {
    flex: 0 0 auto;
    display: block;
    padding: 3px 4px;
  }
  .trace svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .trace--tile {
    width: 92px;
    height: 26px;
  }
  .trace--row {
    width: 132px;
    height: 34px;
  }
  .trace--hero {
    width: 100%;
    height: 120px;
    padding: var(--s-8);
  }

  .tr-lead {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 1.5;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .tr-oth {
    fill: none;
    stroke: var(--app-text-faint);
    stroke-width: 1.1;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
    opacity: 0.75;
  }
  /* The floor is a real engine constant (DISPLAY_FLOOR), not a design flourish
     — hence a printed reference line rather than a gradient or a tint. */
  .tr-floor {
    stroke: var(--app-danger);
    stroke-width: 1;
    stroke-dasharray: 2 3;
    opacity: 0.5;
    vector-effect: non-scaling-stroke;
  }
  .tr-area {
    fill: var(--app-accent);
    opacity: 0.13;
    stroke: none;
  }
</style>
