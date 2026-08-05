<script lang="ts">
  import { tip } from "./tooltip";
  import { tick } from "svelte";
  import type { RetentionPolicy } from "$lib/types";
  import { retentionPresets } from "./retention";

  interface Props {
    /** The selected retention policy (bindable). */
    value: RetentionPolicy;
    /** Called whenever a different preset is chosen. */
    onValueChange?: (v: RetentionPolicy) => void;
    /** Disables the whole picker. */
    disabled?: boolean;
    /** Optional aria-label for the group container. */
    ariaLabel?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    disabled = false,
    ariaLabel = "Retention duration",
  }: Props = $props();

  const presets = retentionPresets();

  // Per-chip button refs, so keyboard nav can move DOM focus onto the newly
  // active chip (focus-follows-selection — the roving tabindex alone leaves
  // focus stranded on the now tabindex=-1 button).
  let chipEls = $state<(HTMLButtonElement | null)[]>([]);

  function select(next: RetentionPolicy) {
    if (disabled || next === value) return;
    value = next;
    onValueChange?.(next);
  }

  // Click handler: select, then pull DOM focus onto the clicked chip. The Tauri
  // WKWebView doesn't focus a <button> on click, so without this the roving
  // tabindex has no anchor and a follow-up arrow key does nothing.
  function selectByClick(index: number) {
    select(presets[index].value);
    chipEls[index]?.focus();
  }

  // After a keyboard selection, move focus to the new chip — but only when
  // focus is already inside this picker, so we never steal focus on mount or on
  // a programmatic value change.
  function focusSelected(index: number) {
    const group = chipEls[index]?.closest(".retention-picker");
    if (group && group.contains(document.activeElement)) {
      chipEls[index]?.focus();
    }
  }

  function onKeydown(event: KeyboardEvent, index: number) {
    if (disabled) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (index + 1) % presets.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (index - 1 + presets.length) % presets.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = presets.length - 1;
    }
    if (nextIndex === null) return;
    const target = nextIndex;
    event.preventDefault();
    select(presets[target].value);
    // Selecting flips the roving tabindex to `target`; follow it with DOM focus
    // so the new chip is what the user is actually on. tick() lets the
    // tabindex/active classes update first.
    void tick().then(() => focusSelected(target));
  }
</script>

<div
  class="retention-picker"
  class:retention-picker--disabled={disabled}
  role="radiogroup"
  aria-label={ariaLabel}
>
  {#each presets as preset, index (preset.value)}
    {@const active = value === preset.value}
    <button
      type="button"
      bind:this={chipEls[index]}
      class="preset"
      class:preset--active={active}
      role="radio"
      aria-checked={active}
      aria-label={preset.label}
      use:tip={preset.label}
      tabindex={active ||
      (!presets.some((p) => p.value === value) && index === 0)
        ? 0
        : -1}
      {disabled}
      onclick={() => selectByClick(index)}
      onkeydown={(e) => onKeydown(e, index)}
    >
      <span class="preset__label">{preset.label}</span>
    </button>
  {/each}
</div>

<style>
  /* Custom input 2 of 5 — the retention LADDER.
     Not a row of chips: the four keep-windows are ordered stops on one axis, so
     they are drawn as one 28px segmented track with equal-width stops, in
     ascending order. The caller draws the footprint bar directly underneath, on
     the same axis, so the window and what it costs read as one instrument. */
  .retention-picker {
    display: flex;
    width: 100%;
    height: var(--h-md);
    padding: 2px;
    border-radius: calc(var(--r-md) + 1px);
    background: color-mix(in srgb, var(--app-text-strong) 7%, transparent);
  }

  .retention-picker--disabled {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  .preset {
    flex: 1 1 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    padding: 0 var(--s-8);
    border: 0;
    border-radius: var(--r-md);
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: var(--t-ui);
    font-weight: var(--w-medium);
    line-height: 1;
    letter-spacing: var(--ls-ui);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    user-select: none;
    outline: none;
    transition: color var(--dur-quick) var(--ease);
  }

  .preset:hover:not(.preset--active) {
    color: var(--app-text-strong);
  }

  .preset:focus-visible {
    box-shadow: 0 0 0 3.5px var(--app-accent-glow);
  }

  /* The AppKit selected-segment treatment: a raised neutral cap, not a colour.
     The accent is spent on the switch and the footprint bar. */
  .preset--active {
    background: var(--seg-on-bg);
    color: var(--app-text-strong);
    box-shadow: var(--seg-on-shadow);
  }

  .preset:disabled {
    cursor: not-allowed;
  }

  .preset__label {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .preset {
      transition: none;
    }
  }
</style>
