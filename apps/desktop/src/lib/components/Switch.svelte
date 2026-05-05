<script lang="ts">
  import { Switch as BitsSwitch } from "bits-ui";

  interface Props {
    checked: boolean;
    onCheckedChange?: (v: boolean) => void;
    disabled?: boolean;
    label?: string;
    description?: string;
  }

  let {
    checked = $bindable(),
    onCheckedChange,
    disabled = false,
    label,
    description,
  }: Props = $props();
</script>

<div class="switch-wrapper" class:switch-wrapper--disabled={disabled}>
  {#if label}
    <div class="switch-text">
      <span class="switch-label">{label}</span>
      {#if description}
        <span class="switch-description">{description}</span>
      {/if}
    </div>
  {/if}
  <BitsSwitch.Root
    bind:checked
    {disabled}
    {onCheckedChange}
    class="switch-track"
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
    border-radius: 10px;
    cursor: pointer;
    transition: background 0.18s ease, border-color 0.18s ease;
    flex-shrink: 0;
    padding: 0;
    outline: none;
  }

  :global(.switch-track:focus-visible) {
    outline: 1px solid var(--app-accent);
    outline-offset: 2px;
  }

  :global(.switch-track[data-state="checked"]) {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
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
    transition: transform 0.18s ease, background 0.18s ease;
    pointer-events: none;
  }

  :global(.switch-track[data-state="checked"] .switch-thumb) {
    transform: translateX(16px);
    background: var(--app-accent);
  }
</style>
