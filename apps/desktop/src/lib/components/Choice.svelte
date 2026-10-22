<script lang="ts">
  import type { Component } from "svelte";

  // Kit `.mx-choice`: options with descriptions (replaces RadioGroup). Real,
  // visually hidden radios carry the keyboard (←→↑↓) and form state natively.
  interface Option {
    value: string;
    label: string;
    description?: string;
    /** Quiet mono note after the title ("default", "on-device · 800 MB"). */
    tag?: string;
    icon?: Component;
  }

  interface Props {
    value: string;
    onValueChange?: (v: string) => void;
    options: Option[];
    disabled?: boolean;
    /** Individual values rendered as disabled options. */
    disabledValues?: string[];
    ariaLabel?: string;
    /** Stacked rows (long descriptions / more than 4 options) instead of tiles. */
    list?: boolean;
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    disabled = false,
    disabledValues = [],
    ariaLabel,
    list = false,
  }: Props = $props();

  const name = `choice-${Math.random().toString(36).slice(2, 9)}`;

  // With `onValueChange` the parent owns the value and may refuse it (a confirm
  // dialog the user cancels). The browser has already moved the native check, so
  // snap the DOM back to `value`; an accepted change flows back through the prop.
  // Without it, the click simply writes the bindable `value`.
  function pick(next: string) {
    if (!onValueChange) {
      value = next;
      return;
    }
    for (const input of document.getElementsByName(name)) {
      (input as HTMLInputElement).checked = (input as HTMLInputElement).value === value;
    }
    onValueChange(next);
  }
</script>

<div class="mx-choice" class:mx-choice--list={list} role="radiogroup" aria-label={ariaLabel}>
  {#each options as option (option.value)}
    <label class="mx-choice__opt">
      <input
        type="radio"
        {name}
        value={option.value}
        checked={value === option.value}
        disabled={disabled || disabledValues.includes(option.value)}
        onchange={() => pick(option.value)}
      />
      {#if option.icon}
        <span class="mx-choice__icon" aria-hidden="true"><option.icon /></span>
      {/if}
      <span class="mx-choice__title">
        {option.label}
        {#if option.tag}<span class="mx-choice__tag">{option.tag}</span>{/if}
      </span>
      {#if option.description}
        <span class="mx-choice__desc">{option.description}</span>
      {/if}
    </label>
  {/each}
</div>

<style>
  /* Look lives in kit.css (.mx-choice). As a flex item (a settings control
     slot) the grid must still take the full row, or auto-fit wraps early. */
  .mx-choice {
    flex: 1 1 100%;
    min-width: 0;
  }
</style>
