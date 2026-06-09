<script lang="ts">
  // MiniBars — horizontal labelled bars for time-per-app (free / grayscale tier).
  // Each item gets a labelled track whose fill width is proportional to the
  // largest value in the set; fills step down the grayscale ramp by rank.
  // Props:
  //   items: { label: string; value: number; sublabel?: string }[]
  //          value drives the bar width (largest = full); sublabel is the
  //          trailing readout (e.g. "3h12m"). If omitted, the numeric value is shown.

  interface MiniBarItem {
    label: string;
    value: number;
    sublabel?: string;
  }

  interface Props {
    items: MiniBarItem[];
  }

  let { items }: Props = $props();

  const greyRamp = [
    "--chart-grey-5",
    "--chart-grey-4",
    "--chart-grey-3",
    "--chart-grey-2",
    "--chart-grey-1",
  ];

  const max = $derived(items.reduce((acc, it) => Math.max(acc, it.value), 0));

  function widthFor(value: number): number {
    if (max <= 0) return 0;
    return Math.max(0, Math.min(100, (value / max) * 100));
  }

  function colorVarFor(index: number): string {
    return greyRamp[Math.min(index, greyRamp.length - 1)];
  }
</script>

<div class="mini-bars">
  {#each items as item, i (item.label + i)}
    <div class="mini-bar">
      <span class="label" title={item.label}>{item.label}</span>
      <span class="track">
        <span
          class="fill"
          style="width:{widthFor(item.value)}%; background:var({colorVarFor(i)});"
        ></span>
      </span>
      <span class="val">{item.sublabel ?? item.value}</span>
    </div>
  {/each}
</div>

<style>
  .mini-bars {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mini-bar {
    display: grid;
    grid-template-columns: 64px 1fr 56px;
    align-items: center;
    gap: 8px;
  }
  .label {
    font-size: 11px;
    color: var(--app-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .track {
    height: 8px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    transition: width 0.18s ease;
  }
  .val {
    font-size: 10px;
    color: var(--app-text-muted);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
