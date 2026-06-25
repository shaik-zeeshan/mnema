<script lang="ts">
  import { Slider as BitsSlider } from "bits-ui";

  interface Props {
    value: number;
    onValueChange?: (v: number) => void;
    min?: number;
    max?: number;
    step?: number;
    disabled?: boolean;
    label?: string;
    unit?: string;
    formatValue?: (v: number) => string;
    // Accessible name for sliders rendered without a visible `label` to link via
    // aria-labelledby (otherwise BitsSlider.Root's role="slider" has no name).
    ariaLabel?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    min = 0,
    max = 100,
    step = 1,
    disabled = false,
    label,
    unit = "",
    formatValue,
    ariaLabel,
  }: Props = $props();

  // Stable id so the visible label (a plain <span>, not associated by
  // BitsSlider.Root) can be linked to the slider via aria-labelledby —
  // otherwise the role="slider" has no accessible name.
  const labelId = `slider-label-${Math.random().toString(36).slice(2, 9)}`;

  function handleValueChange(v: number) {
    value = v;
    onValueChange?.(v);
  }

  const displayValue = $derived(formatValue ? formatValue(value) : `${value}${unit}`);
</script>

<div class="slider-wrapper" class:slider-wrapper--disabled={disabled}>
  {#if label}
    <div class="slider-header">
      <span class="slider-label" id={labelId}>{label}</span>
      <span class="slider-value">{displayValue}</span>
    </div>
  {/if}
  <BitsSlider.Root
    type="single"
    bind:value
    onValueChange={handleValueChange}
    {min}
    {max}
    {step}
    {disabled}
    class="slider-root"
    aria-labelledby={label ? labelId : undefined}
    aria-label={!label && ariaLabel ? ariaLabel : undefined}
    aria-valuetext={`${displayValue}`}
  >
    {#snippet children({ thumbItems })}
      <BitsSlider.Range class="slider-range" />
      {#each thumbItems as { index }}
        <BitsSlider.Thumb {index} class="slider-thumb" />
      {/each}
    {/snippet}
  </BitsSlider.Root>
</div>

<style>
  .slider-wrapper {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .slider-wrapper--disabled {
    opacity: 0.38;
    pointer-events: none;
  }

  .slider-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .slider-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--app-text);
    letter-spacing: 0.02em;
  }

  .slider-value {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: 0.02em;
    font-variant-numeric: tabular-nums;
  }

  :global(.slider-root) {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    height: 20px;
    touch-action: none;
    user-select: none;
    cursor: pointer;
  }

  :global(.slider-root[data-disabled]) {
    cursor: not-allowed;
  }

  :global(.slider-root)::before {
    content: "";
    position: absolute;
    top: 50%;
    left: 0;
    right: 0;
    height: 4px;
    background: var(--app-border-strong);
    border-radius: 999px;
    transform: translateY(-50%);
  }

  :global(.slider-range) {
    position: absolute;
    top: 50%;
    left: 0;
    height: 4px;
    background: linear-gradient(90deg, var(--app-accent-strong), var(--app-accent));
    border-radius: 999px;
    transform: translateY(-50%);
  }

  :global(.slider-thumb) {
    position: absolute;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--app-accent);
    border: 2px solid var(--app-bg);
    box-shadow: 0 0 8px var(--app-accent-glow);
    cursor: pointer;
    transition: box-shadow 0.12s ease, transform 0.12s ease;
    transform: translateX(-50%);
    outline: none;
  }

  :global(.slider-thumb:focus-visible) {
    box-shadow: 0 0 0 4px var(--app-accent-glow);
  }

  :global(.slider-thumb:hover) {
    transform: translateX(-50%) scale(1.15);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.slider-thumb) {
      transition: none;
    }
  }
</style>
