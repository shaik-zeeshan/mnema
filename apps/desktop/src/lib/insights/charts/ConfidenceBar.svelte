<script lang="ts">
  // ConfidenceBar — inline mini confidence bar with a percentage readout and an
  // optional trend glyph. The fill is accent-coloured; a 'faded' trend dims the
  // whole control (below the display floor).
  // Props:
  //   confidence: number                          // 0..1
  //   trend?: 'up' | 'steady' | 'down' | 'faded'  // ▲ / – / ▼ glyph (default none)

  interface Props {
    confidence: number;
    trend?: "up" | "steady" | "down" | "faded";
  }

  let { confidence, trend }: Props = $props();

  // Guard non-finite input (NaN/Infinity → 0) before clamping to [0, 1] so the
  // fill width can never become `NaN%`/`Infinity%`.
  const clamped = $derived(
    Number.isFinite(confidence) ? Math.max(0, Math.min(1, confidence)) : 0,
  );
  const pct = $derived(Math.round(clamped * 100));
  const isFaded = $derived(trend === "faded");

  const glyph = $derived(
    trend === "up"
      ? "▲"
      : trend === "down"
        ? "▼"
        : trend === "steady"
          ? "–"
          : "",
  );
</script>

<span class="confidence" class:confidence--faded={isFaded}>
  <span class="bar">
    <span class="fill" style="width:{pct}%;"></span>
  </span>
  <span class="pct">{pct}%</span>
  {#if glyph}
    <span class="trend trend--{trend}" aria-hidden="true">{glyph}</span>
  {/if}
</span>

<style>
  .confidence {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .confidence--faded {
    opacity: 0.5;
  }
  .bar {
    position: relative;
    width: 56px;
    height: 5px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
    flex: 0 0 auto;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    height: 100%;
    border-radius: 999px;
    background: var(--app-accent);
    box-shadow: 0 0 8px var(--app-accent-glow);
  }
  .pct {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .trend {
    font-size: 9px;
    line-height: 1;
  }
  .trend--up {
    color: var(--app-accent);
  }
  /* "cooling" (▼) is normal decay, not an error — read it QUIET (muted neutral)
     so the saturated --app-danger token stays reserved for destructive/
     contradiction states and the two never collide (matches the Subjects index). */
  .trend--down {
    color: var(--app-text-muted);
  }
  .trend--steady {
    color: var(--app-text-subtle);
  }
</style>
