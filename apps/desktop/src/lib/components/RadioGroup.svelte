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
    /**
     * Visual shape. `list` (default, the original behaviour) renders a
     * vertical stack of compact rows. `card` renders a grid of bordered
     * selectable tiles (the gallery card-radio).
     */
    variant?: "list" | "card";
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    disabled = false,
    disabledValues = [],
    label,
    variant = "list",
  }: Props = $props();

  // Stable id so the visible label can be programmatically associated with the
  // radiogroup container via aria-labelledby (the label renders as a plain
  // <span>). BitsRadioGroup.Root renders the role="radiogroup" element and
  // spreads extra attributes onto it.
  const labelId = `rg-label-${Math.random().toString(36).slice(2, 9)}`;

  function handleValueChange(v: string) {
    value = v;
    onValueChange?.(v);
  }
</script>

<div class="rg-wrapper" class:rg-wrapper--disabled={disabled}>
  {#if label}
    <span class="rg-label" id={labelId}>{label}</span>
  {/if}
  <BitsRadioGroup.Root
    bind:value
    onValueChange={handleValueChange}
    {disabled}
    aria-labelledby={label ? labelId : undefined}
    class={variant === "card" ? "rg-root rg-root--card" : "rg-root"}
  >
    {#each options as option (option.value)}
      {@const isItemDisabled = disabledValues.includes(option.value)}
      <BitsRadioGroup.Item
        value={option.value}
        disabled={isItemDisabled}
        class={variant === "card" ? "rg-item rg-item--card" : "rg-item"}
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
    /* Match Select/Combobox wrappers: stretch to fill the row's control slot
       instead of shrinking to the widest item's content width. */
    width: 100%;
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
    outline: none;
    box-shadow: var(--app-ring);
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
    font-size: var(--text-base);
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

  /* ── Card variant ────────────────────────────────────────────── */
  :global(.rg-root--card) {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  :global(.rg-item--card) {
    align-items: flex-start;
    gap: 11px;
    padding: 14px;
    border-radius: 10px;
    border: 1px solid var(--app-border-strong);
    background: var(--app-surface);
  }

  :global(.rg-item--card:hover) {
    background: var(--app-surface);
    border-color: var(--app-border-hover);
  }

  :global(.rg-item--card[data-state="checked"]) {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  :global(.rg-item--card) .rg-indicator {
    margin-top: 1px;
    width: 16px;
    height: 16px;
    border-color: var(--app-border-hover);
  }

  :global(.rg-item--card) .rg-indicator--checked {
    border-color: var(--app-accent);
    background: transparent;
    box-shadow: 0 0 8px var(--app-accent-glow);
  }

  :global(.rg-item--card) .rg-item-text {
    gap: 3px;
  }

  :global(.rg-item--card) .rg-item-label {
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--app-text-strong);
  }

  :global(.rg-item--card[data-state="checked"]) .rg-item-label {
    color: var(--app-accent);
  }

  :global(.rg-item--card) .rg-item-desc {
    font-size: 11px;
    line-height: 1.4;
    color: var(--app-text-muted);
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
