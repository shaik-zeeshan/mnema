<script lang="ts">
  import type { Snippet } from "svelte";

  interface Option {
    value: string;
    label: string;
  }

  interface Props {
    /** The selectable options, rendered left to right. */
    options: Option[];
    /** The currently selected value (bindable). */
    value: string;
    /** Called whenever a different segment is chosen. */
    onValueChange?: (v: string) => void;
    /** Disables the whole group. */
    disabled?: boolean;
    /**
     * Optional leading-icon snippet, keyed by option value. Receives the
     * option `value` so a single snippet can switch on it:
     *   {#snippet icon(value)} … {/snippet}
     * Icons render at 12×12 inside each segment, before the label.
     */
    icon?: Snippet<[string]>;
    /** Optional aria-label for the group container. */
    ariaLabel?: string;
    /** Visual size; `compact` is the tighter pill used in titlebars. */
    compact?: boolean;
  }

  let {
    options,
    value = $bindable(),
    onValueChange,
    disabled = false,
    icon,
    ariaLabel,
    compact = false,
  }: Props = $props();

  function select(next: string) {
    if (disabled || next === value) return;
    value = next;
    onValueChange?.(next);
  }

  function onKeydown(event: KeyboardEvent, index: number) {
    if (disabled) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (index + 1) % options.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (index - 1 + options.length) % options.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = options.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    select(options[nextIndex].value);
  }
</script>

<div
  class="segmented"
  class:segmented--compact={compact}
  class:segmented--disabled={disabled}
  role="radiogroup"
  aria-label={ariaLabel}
>
  {#each options as option, index (option.value)}
    <button
      type="button"
      class="seg"
      class:seg--active={value === option.value}
      role="radio"
      aria-checked={value === option.value}
      aria-label={option.label}
      title={option.label}
      tabindex={value === option.value || (value == null && index === 0) ? 0 : -1}
      {disabled}
      onclick={() => select(option.value)}
      onkeydown={(e) => onKeydown(e, index)}
    >
      {#if icon}
        <span class="seg__icon" aria-hidden="true">{@render icon(option.value)}</span>
      {/if}
      {#if option.label}
        <span class="seg__label">{option.label}</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface);
  }

  .segmented--disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .seg {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 5px 12px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 12px;
    font-weight: 540;
    line-height: 1;
    cursor: pointer;
    user-select: none;
    outline: none;
    transition: background 0.15s, color 0.15s, box-shadow 0.15s;
  }

  .seg:hover:not(.seg--active) {
    color: var(--app-text);
    background: var(--app-surface-hover);
  }

  .seg:focus-visible {
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .seg--active {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 1px var(--app-accent-border);
  }

  .seg:disabled {
    cursor: not-allowed;
  }

  .seg__icon,
  .seg__icon :global(svg) {
    display: block;
    width: 12px;
    height: 12px;
    flex: 0 0 auto;
  }

  .seg__icon :global(svg) {
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* ── Compact (titlebar pill) ─────────────────────────────────── */
  .segmented--compact {
    padding: 2px;
    gap: 2px;
    border-radius: 999px;
    border-color: var(--app-icon-border-hover);
    background: var(--app-surface-raised);
  }

  .segmented--compact .seg {
    padding: 4px 7px;
    border-radius: 999px;
    color: var(--app-icon-fg);
  }

  .segmented--compact .seg:hover:not(.seg--active) {
    background: var(--app-icon-bg-hover);
    color: var(--app-icon-fg-hover);
  }

  .segmented--compact .seg--active {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 1px var(--app-accent-border);
  }

  .segmented--compact .seg__icon,
  .segmented--compact .seg__icon :global(svg) {
    width: 16px;
    height: 16px;
  }
</style>
