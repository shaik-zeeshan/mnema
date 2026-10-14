<script lang="ts">
  interface Props {
    checked: boolean;
    onCheckedChange?: (v: boolean) => void;
    disabled?: boolean;
    label?: string;
    description?: string;
    // Accessible name for the switch when there is no visible `label` to link
    // via `aria-labelledby` (e.g. icon-only / externally-labelled toggles).
    ariaLabel?: string;
  }

  let {
    checked = $bindable(),
    onCheckedChange,
    disabled = false,
    label,
    description,
    ariaLabel,
  }: Props = $props();

  // Stable ids so the visible label/description (plain <span>s, not associated
  // by BitsSwitch.Root) can be linked to the switch via aria-labelledby /
  // aria-describedby — otherwise the role="switch" has no accessible name.
  const labelId = `switch-label-${Math.random().toString(36).slice(2, 9)}`;
  const descriptionId = `switch-desc-${Math.random().toString(36).slice(2, 9)}`;
  // The kit's `.mx-switch` is a native checkbox (role="switch"), so the visible
  // <label for> is part of the hit target natively: no JS click handler, no
  // duplicate tab stop, no double-toggle. Space toggles.
  const switchId = `switch-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="switch-wrapper" class:switch-wrapper--disabled={disabled}>
  {#if label || description}
    <label class="switch-text" for={switchId}>
      {#if label}
        <span class="switch-label" id={labelId}>{label}</span>
      {/if}
      {#if description}
        <span class="switch-description" id={descriptionId}>{description}</span>
      {/if}
    </label>
  {/if}
  <input
    type="checkbox"
    role="switch"
    class="mx-switch"
    bind:checked
    id={switchId}
    {disabled}
    onchange={(e) => onCheckedChange?.(e.currentTarget.checked)}
    aria-labelledby={label ? labelId : undefined}
    aria-label={!label && ariaLabel ? ariaLabel : undefined}
    aria-describedby={description ? descriptionId : undefined}
  />
</div>

<style>
  .switch-wrapper {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
  }

  /* The switch itself dims via kit `.mx-switch:disabled`; dim the text here. */
  .switch-wrapper--disabled .switch-text {
    opacity: var(--app-disabled-opacity);
  }

  .switch-wrapper--disabled {
    cursor: not-allowed;
    /* Kill the label's `cursor: pointer` (and any hit-target activation) while
       disabled, matching the Select/Combobox `--disabled` wrappers. */
    pointer-events: none;
  }

  .switch-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    cursor: pointer;
  }

  .switch-label {
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--app-text);
  }

  .switch-description {
    font-size: var(--text-xs);
    color: var(--app-text-muted);
  }
</style>
