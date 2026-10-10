<script lang="ts">
  import IconMinus from "~icons/lucide/minus";
  import IconPlus from "~icons/lucide/plus";
  import { clampNumber, clampToRange, parseStepperRaw, stepRaw } from "./stepper-clamp";

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
    errorId,
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
    // Id of the element holding the validation message; wired to
    // aria-describedby/aria-errormessage while `invalid`.
    errorId?: string;
  } = $props();

  // Commit clamps the raw string into range (blank stays blank, non-numeric is
  // left for the upstream validator to flag). Typing updates `value` live via
  // bind:value so the parent's effects react exactly as with the old <input>.
  // BOTH bounds are deferred to commit (blur/Enter): rewriting the field while
  // the user is mid-keystroke jumps the caret in WKWebView, and an integer
  // above `max` is no different from one below `min` — both are numbers the
  // user may still be typing toward, so neither is clamped live.
  function commit() {
    const clamped = clampToRange(value, { min, max });
    if (clamped !== value) value = clamped;
  }

  // The current numeric value (or null when blank/non-numeric), surfaced to
  // assistive tech via the spinbutton aria-value* attributes so the +/- buttons
  // announce the resulting value and its bounds.
  const numericValue = $derived(parseStepperRaw(value));

  // Bound state for the +/- buttons: once the value sits at (or past) a bound,
  // the corresponding button is disabled so pressing it again gives an honest
  // "can't go further" cue instead of a dead no-op. A blank/non-numeric field
  // leaves both live so the user can step up or down from empty.
  const atMax = $derived(max !== undefined && numericValue !== null && numericValue >= max);
  const atMin = $derived(min !== undefined && numericValue !== null && numericValue <= min);

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
    class="mx-btn mx-btn--icon"
    disabled={disabled || atMin}
    aria-label={`Decrease${ariaLabel ? ` ${ariaLabel}` : ""}`}
    onclick={() => bump(-1)}
  >
    <IconMinus width="15" height="15" aria-hidden="true" />
  </button>

  <div class="field" class:field--has-unit={!!unit}>
    <input
      {id}
      type="text"
      inputmode="numeric"
      role="spinbutton"
      class="mx-input num"
      bind:value
      {placeholder}
      {disabled}
      aria-label={ariaLabel}
      aria-invalid={invalid}
      aria-describedby={invalid && errorId ? errorId : undefined}
      aria-errormessage={invalid && errorId ? errorId : undefined}
      aria-valuenow={numericValue !== null
        ? clampNumber(numericValue, min, max)
        : undefined}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuetext={numericValue !== null && unit
        ? `${numericValue} ${unit}`
        : undefined}
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
    class="mx-btn mx-btn--icon"
    disabled={disabled || atMax}
    aria-label={`Increase${ariaLabel ? ` ${ariaLabel}` : ""}`}
    onclick={() => bump(1)}
  >
    <IconPlus width="15" height="15" aria-hidden="true" />
  </button>
</div>

<style>
  /* Flat kit pieces: mx-btn icon buttons around an mx-input. */
  .stepper {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .stepper--disabled {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  .field {
    position: relative;
    display: inline-flex;
    min-width: 0;
    flex: 1 1 auto;
  }

  .mx-input {
    width: 100%;
    min-width: 0;
    text-align: center;
  }

  /* leave room for the unit chip and left-align the digits beside it */
  .field--has-unit .mx-input {
    padding-right: 52px;
    text-align: left;
  }

  .unit-chip {
    position: absolute;
    top: 50%;
    right: var(--s-3);
    transform: translateY(-50%);
    font: 500 var(--text-xs)/1 var(--font-mono);
    color: var(--app-text-subtle);
    pointer-events: none;
  }
</style>
