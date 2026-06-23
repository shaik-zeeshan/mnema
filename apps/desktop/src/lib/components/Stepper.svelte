<script lang="ts">
  import { clampToRange, stepRaw } from "./stepper-clamp";

  // `value` is a RAW STRING so it can flow upward unchanged into the settings
  // shell's raw fields (customWidthRaw / customHeightRaw / draftCustomMbpsRaw),
  // which parse strings with their own integer regex. Empty string = unset.
  let {
    value = $bindable(""),
    min,
    max,
    step = 1,
    unit,
    placeholder,
    disabled = false,
    invalid = false,
    id,
    ariaLabel,
  }: {
    value: string;
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
    placeholder?: string;
    disabled?: boolean;
    invalid?: boolean;
    id?: string;
    ariaLabel?: string;
  } = $props();

  // Commit clamps the raw string into range (blank stays blank, non-numeric is
  // left for the upstream validator to flag). Typing updates `value` live via
  // bind:value so the parent's effects react exactly as with the old <input>.
  function commit() {
    const clamped = clampToRange(value, { min, max });
    if (clamped !== value) value = clamped;
  }

  function bump(direction: 1 | -1) {
    if (disabled) return;
    value = stepRaw(value, direction, step, { min, max });
  }

  function onKeydown(event: KeyboardEvent) {
    if (disabled) return;
    if (event.key === "Enter") {
      commit();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      bump(1);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      bump(-1);
    }
  }
</script>

<div class="stepper" class:stepper--disabled={disabled} class:stepper--invalid={invalid}>
  <button
    type="button"
    class="step-btn"
    {disabled}
    aria-label={`Decrease${ariaLabel ? ` ${ariaLabel}` : ""}`}
    onclick={() => bump(-1)}
  >
    <span aria-hidden="true">−</span>
  </button>

  <div class="field" class:field--has-unit={!!unit}>
    <input
      {id}
      type="text"
      inputmode="numeric"
      class="num-input"
      class:num-input--invalid={invalid}
      bind:value
      {placeholder}
      {disabled}
      aria-label={ariaLabel}
      aria-invalid={invalid}
      autocomplete="off"
      onblur={commit}
      onkeydown={onKeydown}
    />
    {#if unit}
      <span class="unit-chip" aria-hidden="true">{unit}</span>
    {/if}
  </div>

  <button
    type="button"
    class="step-btn"
    {disabled}
    aria-label={`Increase${ariaLabel ? ` ${ariaLabel}` : ""}`}
    onclick={() => bump(1)}
  >
    <span aria-hidden="true">+</span>
  </button>
</div>

<style>
  .stepper {
    display: inline-flex;
    align-items: stretch;
    gap: 6px;
    min-width: 0;
  }

  .stepper--disabled {
    opacity: 0.45;
    pointer-events: none;
  }

  .step-btn {
    flex: 0 0 30px;
    width: 30px;
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 16px;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .step-btn:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }

  .step-btn:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .step-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .field {
    position: relative;
    display: inline-flex;
    min-width: 0;
    flex: 1 1 auto;
  }

  .num-input {
    width: 100%;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text);
    font: inherit;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    text-align: center;
    outline: none;
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: border-color 0.12s, box-shadow 0.12s, background 0.12s;
  }

  /* leave room for the unit chip and left-align the digits beside it */
  .field--has-unit .num-input {
    padding-right: 52px;
    text-align: left;
  }

  .num-input::placeholder {
    color: var(--app-text-faint);
  }

  .num-input:focus {
    border-color: var(--app-accent);
    background: var(--app-surface-raised);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25), 0 0 0 3px var(--app-accent-glow);
  }

  .num-input--invalid {
    border-color: var(--app-danger);
  }

  .num-input--invalid:focus {
    border-color: var(--app-danger);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25),
      0 0 0 3px color-mix(in srgb, var(--app-danger) 30%, transparent);
  }

  .num-input:disabled {
    cursor: not-allowed;
  }

  .unit-chip {
    position: absolute;
    top: 50%;
    right: 8px;
    transform: translateY(-50%);
    color: var(--app-text-faint);
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.06em;
    pointer-events: none;
  }
</style>
