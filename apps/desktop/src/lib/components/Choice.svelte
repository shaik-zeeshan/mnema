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
</script>

<div class="mx-choice" class:mx-choice--list={list} role="radiogroup" aria-label={ariaLabel}>
  {#each options as option (option.value)}
    <label class="mx-choice__opt">
      <input
        type="radio"
        {name}
        value={option.value}
        bind:group={value}
        disabled={disabled || disabledValues.includes(option.value)}
        onchange={() => onValueChange?.(option.value)}
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
