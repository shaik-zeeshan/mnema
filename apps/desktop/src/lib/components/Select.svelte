<script lang="ts">
  import { Select as BitsSelect } from "bits-ui";

  interface Option {
    value: string;
    label: string;
  }

  interface Props {
    value: string | null;
    onValueChange?: (v: string) => void;
    options: Option[];
    placeholder?: string;
    disabled?: boolean;
    label?: string;
    warn?: boolean;
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    placeholder = "Select…",
    disabled = false,
    label,
    warn = false,
  }: Props = $props();

  function handleValueChange(v: string) {
    value = v;
    onValueChange?.(v);
  }

  const selectedLabel = $derived(
    value ? (options.find((o) => o.value === value)?.label ?? value) : null
  );
</script>

<div class="select-wrapper" class:select-wrapper--disabled={disabled}>
  {#if label}
    <span class="select-label">{label}</span>
  {/if}
  <BitsSelect.Root
    type="single"
    value={value ?? ""}
    onValueChange={handleValueChange}
    {disabled}
  >
    <BitsSelect.Trigger class={warn ? "select-trigger select-trigger--warn" : "select-trigger"}>
      <span class={selectedLabel ? "select-trigger-text" : "select-trigger-text select-trigger-text--placeholder"}>
        {selectedLabel ?? placeholder}
      </span>
      <span class="select-chevron" aria-hidden="true">▾</span>
    </BitsSelect.Trigger>
    <BitsSelect.Content class="select-content">
      <BitsSelect.Viewport class="select-viewport">
        {#each options as option (option.value)}
          <BitsSelect.Item value={option.value} label={option.label} class="select-item">
            {#snippet children({ selected })}
              <span class="select-item-check" aria-hidden="true">{selected ? "✓" : ""}</span>
              {option.label}
            {/snippet}
          </BitsSelect.Item>
        {/each}
      </BitsSelect.Viewport>
    </BitsSelect.Content>
  </BitsSelect.Root>
</div>

<style>
  .select-wrapper {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }

  .select-wrapper--disabled {
    opacity: 0.38;
    pointer-events: none;
  }

  .select-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #44445a;
  }

  :global(.select-trigger) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 7px 10px;
    background: #0e0e16;
    border: 1px solid #2a2a3a;
    border-radius: 4px;
    cursor: pointer;
    outline: none;
    transition: border-color 0.12s;
    font-family: inherit;
    font-size: 12px;
    gap: 8px;
    text-align: left;
  }

  :global(.select-trigger:hover) {
    border-color: #3a3a5a;
  }

  :global(.select-trigger:focus-visible) {
    border-color: #3dffa0;
    outline: none;
  }

  :global(.select-trigger[data-state="open"]) {
    border-color: #3dffa0;
  }

  :global(.select-trigger--warn) {
    border-color: #7a4a18;
  }

  :global(.select-trigger--warn:focus-visible) {
    border-color: #c47a30;
  }

  .select-trigger-text {
    color: #c0c0d0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .select-trigger-text--placeholder {
    color: #44445a;
  }

  .select-chevron {
    color: #55556a;
    font-size: 10px;
    flex-shrink: 0;
    transition: transform 0.15s;
  }

  :global(.select-trigger[data-state="open"] .select-chevron) {
    transform: rotate(180deg);
  }

  :global(.select-content) {
    background: #13131a;
    border: 1px solid #2a2a3a;
    border-radius: 6px;
    padding: 4px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
    z-index: 100;
    min-width: var(--bits-select-anchor-width);
    max-height: 220px;
    overflow: hidden;
  }

  :global(.select-viewport) {
    overflow-y: auto;
    max-height: 210px;
  }

  :global(.select-item) {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 3px;
    font-family: inherit;
    font-size: 12px;
    color: #c0c0d0;
    cursor: pointer;
    transition: background 0.1s;
    outline: none;
    user-select: none;
    border: none;
    background: transparent;
    width: 100%;
    text-align: left;
  }

  :global(.select-item:hover),
  :global(.select-item[data-highlighted]) {
    background: #1a1a2a;
    color: #e0e0f0;
  }

  :global(.select-item[data-selected]) {
    color: #3dffa0;
    background: #0d1f15;
  }

  .select-item-check {
    width: 12px;
    font-size: 10px;
    color: #3dffa0;
    flex-shrink: 0;
    font-family: inherit;
  }
</style>
