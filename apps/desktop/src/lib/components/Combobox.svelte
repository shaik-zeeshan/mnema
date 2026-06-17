<script lang="ts">
  import { Combobox as BitsCombobox } from "bits-ui";

  interface Option {
    value: string;
    label: string;
  }

  interface Props {
    value: string | null;
    onValueChange?: (v: string) => void;
    options: Option[];
    placeholder?: string;
    label?: string;
    disabled?: boolean;
    emptyText?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    placeholder = "Search…",
    label,
    disabled = false,
    emptyText = "No matches",
  }: Props = $props();

  // The text typed into the input. Drives type-to-filter. Bits-ui keeps the
  // input's displayed text via `inputValue`; we mirror it here so the visible
  // list is the parent-filtered subset (bits-ui does not filter for us).
  let searchText = $state("");
  let open = $state(false);

  const selectedLabel = $derived(
    value ? (options.find((o) => o.value === value)?.label ?? value) : null,
  );

  // While the panel is closed, show the selected label in the input; while open,
  // show whatever the user is typing so they can filter freely.
  const inputValue = $derived(open ? searchText : (selectedLabel ?? ""));

  const filteredOptions = $derived.by(() => {
    const q = searchText.trim().toLowerCase();
    if (!q) return options;
    return options.filter((o) => o.label.toLowerCase().includes(q));
  });

  function handleValueChange(v: string) {
    value = v;
    onValueChange?.(v);
  }

  function handleOpenChange(next: boolean) {
    open = next;
    // Reset the filter each time the panel opens so the full list is available.
    if (next) searchText = "";
  }
</script>

<div class="select-wrapper" class:select-wrapper--disabled={disabled}>
  {#if label}
    <span class="select-label">{label}</span>
  {/if}
  <BitsCombobox.Root
    type="single"
    value={value ?? ""}
    onValueChange={handleValueChange}
    {open}
    onOpenChange={handleOpenChange}
    {disabled}
    {inputValue}
  >
    <div class="combobox-control">
      <BitsCombobox.Input
        class="select-trigger combobox-input"
        {placeholder}
        oninput={(e) => (searchText = e.currentTarget.value)}
        aria-label={label ?? placeholder}
      />
      <BitsCombobox.Trigger class="combobox-chevron" aria-label="Toggle list">
        <span aria-hidden="true">▾</span>
      </BitsCombobox.Trigger>
    </div>
    <BitsCombobox.Portal>
      <BitsCombobox.Content class="select-content" sideOffset={4}>
        <BitsCombobox.Viewport class="select-viewport">
          {#each filteredOptions as option (option.value)}
            <BitsCombobox.Item value={option.value} label={option.label} class="select-item">
              {#snippet children({ selected })}
                <span class="select-item-check" aria-hidden="true">{selected ? "✓" : ""}</span>
                {option.label}
              {/snippet}
            </BitsCombobox.Item>
          {:else}
            <span class="combobox-empty">{emptyText}</span>
          {/each}
        </BitsCombobox.Viewport>
      </BitsCombobox.Content>
    </BitsCombobox.Portal>
  </BitsCombobox.Root>
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
    color: var(--app-text-subtle);
  }

  .combobox-control {
    position: relative;
    width: 100%;
  }

  /* The input reuses the Select trigger look but must accept text + leave room
     for the chevron toggle on the right. */
  :global(.combobox-input.select-trigger) {
    width: 100%;
    padding-right: 32px;
    color: var(--app-text);
    cursor: text;
  }

  :global(.combobox-input.select-trigger::placeholder) {
    color: var(--app-text-subtle);
  }

  :global(.combobox-chevron) {
    position: absolute;
    top: 0;
    right: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--app-text-muted);
    font-size: 10px;
    cursor: pointer;
    outline: none;
    transition: transform 0.15s;
  }

  :global(.combobox-chevron[data-state="open"]) {
    transform: rotate(180deg);
  }

  .combobox-empty {
    display: block;
    padding: 8px 10px;
    font-size: 12px;
    font-style: italic;
    color: var(--app-text-faint);
  }
</style>
