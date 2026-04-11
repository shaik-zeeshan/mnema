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
  }: Props = $props();

  function handleValueChange(v: number) {
    value = v;
    onValueChange?.(v);
  }

  const displayValue = $derived(formatValue ? formatValue(value) : `${value}${unit}`);
</script>

<div class="slider-wrapper" class:slider-wrapper--disabled={disabled}>
  {#if label}
    <div class="slider-header">
      <span class="slider-label">{label}</span>
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
    color: #c0c0d0;
    letter-spacing: 0.02em;
  }

  .slider-value {
    font-size: 11px;
    font-weight: 600;
    color: #3dffa0;
    letter-spacing: 0.04em;
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
    height: 3px;
    background: #1a1a2a;
    border-radius: 2px;
    transform: translateY(-50%);
    border: 1px solid #2a2a40;
  }

  :global(.slider-range) {
    position: absolute;
    top: 50%;
    left: 0;
    height: 3px;
    background: linear-gradient(90deg, #1a4a30, #3dffa0);
    border-radius: 2px;
    transform: translateY(-50%);
  }

  :global(.slider-thumb) {
    position: absolute;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #3dffa0;
    border: 2px solid #0c0c0e;
    box-shadow: 0 0 0 1px #1a4a30;
    cursor: pointer;
    transition: box-shadow 0.12s ease, transform 0.12s ease;
    transform: translateX(-50%);
    outline: none;
  }

  :global(.slider-thumb:focus-visible) {
    box-shadow: 0 0 0 3px rgba(61, 255, 160, 0.25);
  }

  :global(.slider-thumb:hover) {
    transform: translateX(-50%) scale(1.15);
  }
</style>
