<script lang="ts">
  import { Checkbox as BitsCheckbox } from "bits-ui";

  interface Props {
    /** Checked state (bindable). */
    checked: boolean;
    /** Called when the checked state changes. */
    onCheckedChange?: (v: boolean) => void;
    /** Disables the control. */
    disabled?: boolean;
    /** Optional primary label rendered to the right of the box. */
    label?: string;
    /** Optional secondary description under the label. */
    description?: string;
    /**
     * Optional indeterminate (mixed) state. When `true` a dash is shown
     * instead of the check; the box still reports `checked` via binding.
     */
    indeterminate?: boolean;
  }

  let {
    checked = $bindable(),
    onCheckedChange,
    disabled = false,
    label,
    description,
    indeterminate = false,
  }: Props = $props();
</script>

<label class="checkbox-wrapper" class:checkbox-wrapper--disabled={disabled}>
  <BitsCheckbox.Root
    bind:checked
    {disabled}
    {indeterminate}
    {onCheckedChange}
    class="checkbox-box"
  >
    {#snippet children({ checked: isChecked, indeterminate: isIndeterminate })}
      {#if isIndeterminate}
        <span class="checkbox-dash" aria-hidden="true"></span>
      {:else if isChecked}
        <svg
          class="checkbox-check"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="3"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M20 6L9 17l-5-5" />
        </svg>
      {/if}
    {/snippet}
  </BitsCheckbox.Root>
  {#if label}
    <span class="checkbox-text">
      <span class="checkbox-label">{label}</span>
      {#if description}
        <span class="checkbox-description">{description}</span>
      {/if}
    </span>
  {/if}
</label>

<style>
  .checkbox-wrapper {
    display: inline-flex;
    align-items: flex-start;
    gap: 9px;
    cursor: pointer;
    user-select: none;
  }

  .checkbox-wrapper--disabled {
    opacity: 0.4;
    cursor: not-allowed;
    pointer-events: none;
  }

  :global(.checkbox-box) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 16px;
    width: 16px;
    height: 16px;
    margin-top: 1px;
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    background: var(--app-surface);
    padding: 0;
    cursor: pointer;
    outline: none;
    transition: background 0.15s, border-color 0.15s, box-shadow 0.15s;
  }

  :global(.checkbox-box:hover) {
    border-color: var(--app-border-hover);
  }

  :global(.checkbox-box:focus-visible) {
    border-color: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  :global(.checkbox-box[data-state="checked"]),
  :global(.checkbox-box[data-state="indeterminate"]) {
    background: var(--app-accent);
    border-color: var(--app-accent);
    box-shadow: 0 0 8px var(--app-accent-glow);
  }

  :global(.checkbox-box[data-state="checked"]:hover),
  :global(.checkbox-box[data-state="indeterminate"]:hover) {
    border-color: var(--app-accent);
  }

  :global(.checkbox-box[data-disabled]) {
    cursor: not-allowed;
  }

  .checkbox-check {
    display: block;
    width: 11px;
    height: 11px;
    color: var(--app-bg);
  }

  .checkbox-dash {
    width: 8px;
    height: 2px;
    border-radius: 2px;
    background: var(--app-bg);
  }

  .checkbox-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .checkbox-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--app-text);
    letter-spacing: 0.02em;
    line-height: 1.3;
  }

  .checkbox-description {
    font-size: 10px;
    color: var(--app-text-muted);
    letter-spacing: 0.03em;
    line-height: 1.4;
  }
</style>
