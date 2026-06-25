<script lang="ts">
  import { Switch as BitsSwitch } from "bits-ui";

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
  // Forwarded to the bits-ui button (a labelable <button role="switch">) so the
  // visible <label for> is part of the toggle's hit target: clicking the text
  // natively activates the button. No JS click handler (keyboard/AT stay on the
  // button), so no duplicate tab stop and no double-toggle.
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
  <BitsSwitch.Root
    bind:checked
    id={switchId}
    {disabled}
    {onCheckedChange}
    class="switch-track"
    aria-labelledby={label ? labelId : undefined}
    aria-label={!label && ariaLabel ? ariaLabel : undefined}
    aria-describedby={description ? descriptionId : undefined}
  >
    <BitsSwitch.Thumb class="switch-thumb" />
  </BitsSwitch.Root>
</div>

<style>
  .switch-wrapper {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
  }

  .switch-wrapper--disabled {
    opacity: 0.38;
    cursor: not-allowed;
  }

  .switch-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    cursor: pointer;
  }

  .switch-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--app-text);
    letter-spacing: 0.02em;
  }

  .switch-description {
    font-size: 10px;
    color: var(--app-text-muted);
    letter-spacing: 0.03em;
  }

  :global(.switch-track) {
    position: relative;
    display: inline-flex;
    align-items: center;
    width: 36px;
    height: 20px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    cursor: pointer;
    transition: background 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease;
    flex-shrink: 0;
    padding: 0;
    outline: none;
  }

  :global(.switch-track:hover:not([data-disabled])) {
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }

  :global(.switch-track:focus-visible) {
    border-color: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  :global(.switch-track[data-state="checked"]) {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  :global(.switch-track[data-state="checked"]:hover:not([data-disabled])) {
    border-color: var(--app-accent);
  }

  :global(.switch-track[data-disabled]) {
    cursor: not-allowed;
  }

  :global(.switch-thumb) {
    position: absolute;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--app-text-subtle);
    transition: transform 0.18s ease, background 0.18s ease, box-shadow 0.18s ease;
    pointer-events: none;
  }

  :global(.switch-track[data-state="checked"] .switch-thumb) {
    transform: translateX(16px);
    background: var(--app-accent);
    box-shadow: 0 0 8px var(--app-accent-glow);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.switch-track),
    :global(.switch-thumb) {
      transition: none;
    }
  }
</style>
