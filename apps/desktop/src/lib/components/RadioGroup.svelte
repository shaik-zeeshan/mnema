<script lang="ts">
  import { RadioGroup as BitsRadioGroup } from "bits-ui";

  interface Option {
    value: string;
    label: string;
    description?: string;
  }

  interface Props {
    value: string;
    onValueChange?: (v: string) => void;
    options: Option[];
    disabled?: boolean;
    /** Individual values that should be rendered as non-interactive disabled items. */
    disabledValues?: string[];
    label?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    disabled = false,
    disabledValues = [],
    label,
  }: Props = $props();

  function handleValueChange(v: string) {
    value = v;
    onValueChange?.(v);
  }
</script>

<div class="rg-wrapper" class:rg-wrapper--disabled={disabled}>
  {#if label}
    <span class="rg-label">{label}</span>
  {/if}
  <BitsRadioGroup.Root
    bind:value
    onValueChange={handleValueChange}
    {disabled}
    class="rg-root"
  >
    {#each options as option (option.value)}
      {@const isItemDisabled = disabledValues.includes(option.value)}
      <BitsRadioGroup.Item
        value={option.value}
        disabled={isItemDisabled}
        class="rg-item"
        data-item-disabled={isItemDisabled ? "" : undefined}
      >
        {#snippet children({ checked })}
          <span class="rg-indicator" class:rg-indicator--checked={checked && !isItemDisabled} class:rg-indicator--disabled={isItemDisabled}>
            {#if checked && !isItemDisabled}
              <span class="rg-dot"></span>
            {/if}
          </span>
          <div class="rg-item-text">
            <span class="rg-item-label" class:rg-item-label--disabled={isItemDisabled}>{option.label}</span>
            {#if option.description}
              <span class="rg-item-desc" class:rg-item-desc--disabled={isItemDisabled}>{option.description}</span>
            {/if}
          </div>
        {/snippet}
      </BitsRadioGroup.Item>
    {/each}
  </BitsRadioGroup.Root>
</div>

<style>
  .rg-wrapper {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .rg-wrapper--disabled {
    opacity: 0.38;
    pointer-events: none;
  }

  .rg-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  :global(.rg-root) {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  :global(.rg-item) {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border-radius: 4px;
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s;
    text-align: left;
    width: 100%;
    outline: none;
  }

  :global(.rg-item:hover) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
  }

  :global(.rg-item[data-state="checked"]) {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  :global(.rg-item:focus-visible) {
    outline: 1px solid var(--app-accent);
    outline-offset: 1px;
  }

  .rg-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid var(--app-border-strong);
    background: var(--app-surface);
    flex-shrink: 0;
    transition: border-color 0.12s, background 0.12s;
  }

  .rg-indicator--checked {
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
  }

  .rg-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
  }

  .rg-item-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .rg-item-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--app-text);
    letter-spacing: 0.02em;
  }

  .rg-item-desc {
    font-size: 10px;
    color: var(--app-text-muted);
    letter-spacing: 0.02em;
  }
  :global(.rg-item[data-disabled]) {
    opacity: 0.38;
    cursor: not-allowed;
    pointer-events: none;
  }

  .rg-indicator--disabled {
    border-color: var(--app-border);
    background: var(--app-surface);
  }

  .rg-item-label--disabled {
    color: var(--app-text-faint);
  }

  .rg-item-desc--disabled {
    color: var(--app-text-faint);
  }
</style>
