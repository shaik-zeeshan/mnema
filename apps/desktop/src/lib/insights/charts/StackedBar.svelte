<script lang="ts">
  // StackedBar — a single horizontal stacked bar of category segments, with an
  // optional legend underneath. Segment widths are computed as a share of the
  // total value, so callers pass raw values (e.g. minutes/ms) rather than
  // percents — pass the FINEST-GRAINED magnitude available, not a pre-rounded
  // one, or sub-unit segments collapse to a 0-width sliver. `display` overrides
  // the legend readout when the raw value isn't human-friendly (e.g. ms).
  // Props:
  //   segments: { label: string; value: number; colorVar: string; display?: string }[]
  //             colorVar is a CSS custom-property NAME, e.g. "--cat-coding".
  //   showLegend?: boolean   — render the label/value legend below (default true).

  interface StackSegment {
    label: string;
    value: number;
    colorVar: string;
    display?: string;
  }

  interface Props {
    segments: StackSegment[];
    showLegend?: boolean;
  }

  let { segments, showLegend = true }: Props = $props();

  const total = $derived(segments.reduce((acc, s) => acc + Math.max(0, s.value), 0));

  function pct(value: number): number {
    if (total <= 0) return 0;
    return (Math.max(0, value) / total) * 100;
  }
</script>

<div class="stacked">
  <div
    class="bar"
    role="img"
    aria-label={segments.map((s) => `${s.label} ${s.display ?? s.value}`).join(", ")}
  >
    {#each segments as segment (segment.label)}
      <span style="width:{pct(segment.value)}%; background:var({segment.colorVar});"></span>
    {/each}
  </div>
  {#if showLegend}
    <div class="legend">
      {#each segments as segment (segment.label)}
        <span class="item">
          <span class="sw" style="background:var({segment.colorVar});"></span>
          {segment.label}
          <span class="v">{segment.display ?? segment.value}</span>
        </span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .stacked {
    width: 100%;
  }
  .bar {
    display: flex;
    width: 100%;
    height: 24px;
    border-radius: 6px;
    overflow: hidden;
    border: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }
  .bar > span {
    display: block;
    height: 100%;
    transition: width 0.18s ease;
  }
  .legend {
    margin-top: 11px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
  .v {
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
</style>
