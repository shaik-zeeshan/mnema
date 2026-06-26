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

  // The focus mode's three intensity bands, in cellStyle's bucket order
  // (deep ≥ 0.66, mid ≥ 0.33, distracted < 0.33). When a focus-mode legend is
  // supplied we render its `·`-separated labels next to these swatches so the
  // colour key is shown, not just named. Extra label segments fall back to a
  // plain dot; missing ones simply aren't rendered.
  const focusLegendVars = ["--focus-deep", "--focus-mid", "--focus-distracted"];

  // Parsed legend items for focus mode: each `·`-separated label paired with its
  // band colour token. Empty for grey mode or when no legend is given.
  const focusLegendItems = $derived.by<{ label: string; colorVar: string }[]>(
    () => {
      if (colorMode !== "focus" || !legend) return [];
      return legend
        .split("·")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
        .map((label, i) => ({
          label,
          colorVar: focusLegendVars[i] ?? focusLegendVars[focusLegendVars.length - 1],
        }));
    },
  );

  // Per-cell readout so the heatmap is never decode-by-hue only: every cell
  // exposes its row label + a readable value via title (hover) and aria-label.
  function cellTitle(rowLabel: string, value: number): string {
    const v = Math.max(0, Math.min(1, value));
    if (v <= 0) return `${rowLabel} — no signal`;
    const pct = Math.round(v * 100);
    if (colorMode === "focus") {
      const band = v >= 0.66 ? "deep focus" : v >= 0.33 ? "mixed focus" : "scattered";
      return `${rowLabel} — ${band} (${pct}%)`;
    }
    return `${rowLabel} — ${pct}% activity`;
  }

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
            <span
              class="cell"
              style={cellStyle(cell)}
              role="img"
              title={cellTitle(row.label, cell)}
              aria-label={cellTitle(row.label, cell)}
            ></span>
          {/each}
        </div>
      </div>
    {/each}
  </div>
  {#if focusLegendItems.length > 0}
    <!-- Focus mode: colored swatches so each band's hue is shown, not just named. -->
    <div class="legend legend--swatched">
      {#each focusLegendItems as item (item.label)}
        <span class="legend__item">
          <span
            class="legend__swatch"
            style="background:var({item.colorVar});"
            aria-hidden="true"
          ></span>
          {item.label}
        </span>
      {/each}
    </div>
  {:else if legend}
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
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
  }
  /* Focus-mode key: colored swatch + label per intensity band. */
  .legend--swatched {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
  }
  .legend__item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .legend__swatch {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
</style>
