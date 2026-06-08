<script lang="ts">
  // Static 9-bar waveform glyph shared by the answer-source and search-result
  // audio cards. The bar geometry is fixed; only the rendered width differs per
  // call site (the strip tile and the list row size their tiles differently),
  // so `widthPercent` is the one knob. `class` lets each card keep its own BEM
  // class (`source-card__wave` / `search-card__wave`) for surrounding styles.
  const BAR_HEIGHTS = [7, 13, 20, 10, 23, 9, 16, 12, 8];

  let {
    widthPercent,
    class: className = "",
  }: {
    widthPercent: number;
    class?: string;
  } = $props();
</script>

<svg
  class={className}
  style="width: {widthPercent}%"
  viewBox="0 0 44 24"
  aria-hidden="true"
>
  {#each BAR_HEIGHTS as barHeight, barIndex (barIndex)}
    <rect x={2 + barIndex * 4.8} y={(24 - barHeight) / 2} width="2.4" height={barHeight} rx="1.2" />
  {/each}
</svg>

<style>
  svg {
    height: auto;
    fill: currentColor;
  }
</style>
