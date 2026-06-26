<script lang="ts">
  // MiniBars — horizontal labelled bars for time-per-app.
  // Each item gets a labelled track whose fill width is proportional to the
  // largest value in the set. Fills are a single-hue ramp keyed to rank: the
  // top app is pure accent and each lower rank blends further toward the track
  // background. This deliberately does NOT use the --cat-* category palette —
  // these bars encode magnitude (rank), not category, so they never collide
  // with category colours elsewhere in the UI. The dominant (largest-value)
  // row reads as the focal point: brightest fill + stronger label.
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

  const max = $derived(items.reduce((acc, it) => Math.max(acc, it.value), 0));

  function widthFor(value: number): number {
    if (max <= 0) return 0;
    return Math.max(0, Math.min(100, (value / max) * 100));
  }

  // Single-hue ramp keyed to rank: rank 0 is pure accent; each lower rank
  // blends further toward the track surface. Encodes magnitude, not category.
  function fillFor(index: number): string {
    const steps = Math.max(items.length - 1, 1);
    const t = index / steps; // 0 at the top of the ranking → 1 at the bottom
    const fade = Math.round(t * 58); // blend up to 58% into the track surface
    return `color-mix(in oklab, var(--app-accent) ${100 - fade}%, var(--app-surface-hover))`;
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
      <span
        class="track"
        role="img"
        aria-label={`${item.label}: ${item.sublabel ?? item.value} (${Math.round(widthFor(item.value))}% of the top value)`}
      >
        <span
          class="fill"
          style="width:{widthFor(item.value)}%; background:{fillFor(i)};"
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
  /* The ramp itself carries rank (rank 0 = brightest), so the fill needs no
     per-row opacity dimming — only a subtle global softening. */
  .fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    opacity: 0.92;
    transition: width 0.18s ease;
  }
  .val {
    font-size: 10px;
    color: var(--app-text-muted);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
