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
    opacity: var(--app-disabled-opacity);
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
    font: var(--w-regular) var(--t-ui) / 1.25 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }

  .switch-description {
    font: var(--w-regular) var(--t-meta) / 1.35 var(--app-font-sans);
    letter-spacing: var(--ls-meta);
    color: var(--app-text-muted);
  }

  /* Stock macOS switch metrics: 38 × 22 with an 18px knob. */
  :global(.switch-track) {
    position: relative;
    display: inline-flex;
    align-items: center;
    width: 38px;
    height: 22px;
    background: var(--app-surface-hover);
    border: 0;
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
    border-radius: var(--r-pill);
    cursor: pointer;
    transition: background 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease,
      transform 0.18s ease;
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
    box-shadow: var(--app-ring);
  }

  /* Momentary press cue before the state flips. */
  :global(.switch-track:active:not([data-disabled])) {
    transform: scale(0.96);
  }

  /* On = a filled accent track with a white knob. A macOS switch says "on" by
     the whole track lighting up, not by a tinted outline. */
  :global(.switch-track[data-state="checked"]) {
    background: var(--app-accent);
    box-shadow: none;
  }

  :global(.switch-track[data-disabled]) {
    cursor: not-allowed;
  }

  :global(.switch-thumb) {
    position: absolute;
    left: 2px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
    transition: transform 0.18s ease;
    pointer-events: none;
  }

  :global(.switch-track[data-state="checked"] .switch-thumb) {
    transform: translateX(16px);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.switch-track),
    :global(.switch-thumb) {
      transition: none;
    }
  }
</style>
