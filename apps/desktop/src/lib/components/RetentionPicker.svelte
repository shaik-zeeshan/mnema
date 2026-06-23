<script lang="ts">
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

  function select(next: RetentionPolicy) {
    if (disabled || next === value) return;
    value = next;
    onValueChange?.(next);
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
    event.preventDefault();
    select(presets[nextIndex].value);
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
      class="preset"
      class:preset--active={active}
      role="radio"
      aria-checked={active}
      aria-label={preset.label}
      title={preset.label}
      tabindex={active || (value == null && index === 0) ? 0 : -1}
      {disabled}
      onclick={() => select(preset.value)}
      onkeydown={(e) => onKeydown(e, index)}
    >
      <span class="preset__label">{preset.label}</span>
    </button>
  {/each}
</div>

<style>
  /* A duration-preset picker over the four supported RetentionPolicy values,
     styled in the gallery's preset-chip / segmented language. Chips wrap so the
     control stays readable inside the full-width settings row. */
  .retention-picker {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .retention-picker--disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .preset {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 7px 14px;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font: inherit;
    font-size: 12px;
    font-weight: 540;
    line-height: 1;
    letter-spacing: 0.01em;
    cursor: pointer;
    user-select: none;
    outline: none;
    transition: background 0.15s, color 0.15s, border-color 0.15s, box-shadow 0.15s;
  }

  .preset:hover:not(.preset--active) {
    color: var(--app-text);
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }

  .preset:focus-visible {
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .preset--active {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 1px var(--app-accent-border),
      0 0 10px color-mix(in srgb, var(--app-accent) 18%, transparent);
  }

  .preset:disabled {
    cursor: not-allowed;
  }

  .preset__label {
    display: block;
  }
</style>
