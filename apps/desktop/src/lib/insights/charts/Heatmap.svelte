<script lang="ts">
  // Heatmap — a labelled grid of cells coloured by intensity, for focus or
  // activity heatmaps. Each row has a label and an array of cell intensities
  // (0..1). Two colour modes:
  //   'focus' — green (deep) → amber (mid) → red (distracted) via --focus-* tokens,
  //             with intensity driving opacity.
  //   'grey'  — steps up the --chart-grey ramp by intensity bucket.
  // Props:
  //   rows: { label: string; cells: number[] }[]   // cell values 0..1
  //   colorMode: 'focus' | 'grey'
  //   legend?: string                               // optional caption under the grid

  interface HeatRow {
    label: string;
    cells: number[];
  }
  interface Props {
    rows: HeatRow[];
    colorMode: "focus" | "grey";
    legend?: string;
  }

  let { rows, colorMode, legend }: Props = $props();

  const greyRamp = [
    "--chart-grey-1",
    "--chart-grey-2",
    "--chart-grey-3",
    "--chart-grey-4",
    "--chart-grey-5",
  ];

  function cellStyle(value: number): string {
    const v = Math.max(0, Math.min(1, value));
    if (v <= 0) {
      // Empty cell — neutral surface, no fill.
      return "background:var(--app-surface-hover);";
    }
    if (colorMode === "grey") {
      const idx = Math.min(greyRamp.length - 1, Math.floor(v * greyRamp.length));
      return `background:var(${greyRamp[idx]});`;
    }
    // focus mode: bucket into deep / mid / distracted bands, intensity → opacity.
    const colorVar =
      v >= 0.66 ? "--focus-deep" : v >= 0.33 ? "--focus-mid" : "--focus-distracted";
    const opacity = (0.4 + v * 0.55).toFixed(2);
    return `background:var(${colorVar}); opacity:${opacity}; border-color:transparent;`;
  }
</script>

<div class="heatmap">
  <div class="grid">
    {#each rows as row (row.label)}
      <div class="row">
        <span class="rlabel">{row.label}</span>
        <div class="cells">
          {#each row.cells as cell, i (i)}
            <span class="cell" style={cellStyle(cell)}></span>
          {/each}
        </div>
      </div>
    {/each}
  </div>
  {#if legend}
    <div class="legend">{legend}</div>
  {/if}
</div>

<style>
  .heatmap {
    width: 100%;
  }
  .grid {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .row {
    display: grid;
    grid-template-columns: 36px 1fr;
    align-items: center;
    gap: 8px;
  }
  .rlabel {
    font-size: 9.5px;
    color: var(--app-text-muted);
    white-space: nowrap;
  }
  .cells {
    display: flex;
    gap: 3px;
  }
  .cell {
    flex: 1 1 0;
    height: 12px;
    border-radius: 3px;
    border: 1px solid var(--app-border);
  }
  .legend {
    margin-top: 9px;
    font-size: 9.5px;
    color: var(--app-text-subtle);
  }
</style>
