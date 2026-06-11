<script lang="ts">
  // MiniBars — horizontal labelled bars for time-per-app.
  // Each item gets a labelled track whose fill width is proportional to the
  // largest value in the set; fills rotate through the app's --cat-* category
  // palette by rank. The dominant (largest-value) row reads as the focal point:
  // full-opacity fill + stronger label, while the rest are slightly dimmed.
  // Props:
  //   items: { label: string; value: number; sublabel?: string;
  //            iconSrc?: string | null; fallback?: string }[]
  //          value drives the bar width (largest = full); sublabel is the
  //          trailing readout (e.g. "3h12m"). If omitted, the numeric value is shown.
  //          iconSrc/fallback are optional: when either is provided, a small
  //          avatar (image, else fallback letter) renders before the label.
  //          Items without them render exactly as before.

  interface MiniBarItem {
    label: string;
    value: number;
    sublabel?: string;
    iconSrc?: string | null;
    fallback?: string;
  }

  interface Props {
    items: MiniBarItem[];
  }

  let { items }: Props = $props();

  const catPalette = [
    "--cat-creating",
    "--cat-research",
    "--cat-learning",
    "--cat-communication",
    "--cat-organizing",
    "--cat-personal",
    "--cat-meetings",
    "--cat-entertainment",
  ];

  const max = $derived(items.reduce((acc, it) => Math.max(acc, it.value), 0));

  function widthFor(value: number): number {
    if (max <= 0) return 0;
    return Math.max(0, Math.min(100, (value / max) * 100));
  }

  function colorVarFor(index: number): string {
    return catPalette[index % catPalette.length];
  }

  // The largest-value row is the focal point. Guard max > 0 so an all-zero set
  // doesn't mark every row dominant.
  function isDominant(value: number): boolean {
    return max > 0 && value === max;
  }
</script>

<div class="mini-bars">
  {#each items as item, i (item.label + i)}
    <div class="mini-bar" class:dominant={isDominant(item.value)}>
      <span class="label" title={item.label}>
        {#if item.iconSrc != null || item.fallback !== undefined}
          <span class="icon" aria-hidden="true">
            {#if item.iconSrc}
              <img src={item.iconSrc} alt="" loading="lazy" />
            {:else}
              <span>{item.fallback ?? ""}</span>
            {/if}
          </span>
        {/if}
        <span class="label-text">{item.label}</span>
      </span>
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
    grid-template-columns: 110px 1fr 56px;
    align-items: center;
    gap: 8px;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }
  /* The dominant row reads as the focal point: stronger label, full-opacity fill. */
  .mini-bar.dominant .label {
    color: var(--app-text);
  }
  .label-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Optional app avatar — mirrors the lede strip's bordered rounded square,
     scaled to the bar row. Letter fallback when no image resolves. */
  .icon {
    display: grid;
    width: 16px;
    height: 16px;
    flex: 0 0 16px;
    place-items: center;
    overflow: hidden;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-size: 7.5px;
    font-weight: 800;
    line-height: 1;
  }
  .icon img {
    width: 13px;
    height: 13px;
    object-fit: contain;
  }
  .track {
    height: 10px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    opacity: 0.72;
    transition:
      width 0.18s ease,
      opacity 0.18s ease;
  }
  .mini-bar.dominant .fill {
    opacity: 1;
  }
  .val {
    font-size: 10px;
    color: var(--app-text-muted);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
