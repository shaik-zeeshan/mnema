<script lang="ts" module>
  // Pure filter logic lives in combobox-filter.ts so it can be unit-tested
  // without Svelte/runes. Re-exported here so existing importers (`import {
  // ComboboxOption, filterComboboxOptions } from ".../Combobox.svelte"`) keep
  // working unchanged.
  export {
    filterComboboxOptions,
    type ComboboxOption,
  } from "./combobox-filter";
</script>

<script lang="ts">
  import { Combobox as BitsCombobox } from "bits-ui";
  import {
    filterComboboxOptions,
    type ComboboxOption,
  } from "./combobox-filter";
  import { pinAncestorScrollOnOpen } from "./pin-scroll-on-open";
  import { shouldOpenUpward } from "./popover-direction";

  interface Props {
    value?: string | null;
    onValueChange?: (v: string) => void;
    options: ComboboxOption[];
    placeholder?: string;
    disabled?: boolean;
    label?: string;
    warn?: boolean;
    /** Placeholder text for the in-menu search input. */
    searchPlaceholder?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    placeholder = "Select…",
    disabled = false,
    label,
    warn = false,
    searchPlaceholder = "Search…",
  }: Props = $props();

  let open = $state(false);
  let search = $state("");
  let openUp = $state(false);
  let wrapperEl = $state<HTMLDivElement | null>(null);

  // Stable id so the visible label can be programmatically associated with the
  // trigger via aria-labelledby (the label renders as a plain <span>).
  const labelId = `combobox-label-${Math.random().toString(36).slice(2, 9)}`;

  function handleValueChange(v: string) {
    value = v;
    onValueChange?.(v);
  }

  // bits-ui drives `inputValue` from the search box; mirror it for filtering.
  const filtered = $derived(filterComboboxOptions(options, search));

  const selectedLabel = $derived(
    value ? (options.find((o) => o.value === value)?.label ?? value) : null,
  );

  // The inline popover can't drift, so it can clip at the bottom of Settings'
  // inner scroll container. On open, measure room below vs. above the trigger
  // and flip upward when there isn't enough room below (and there's more above).
  // `max-height` (CSS) still bounds the panel; this just chooses which edge it
  // anchors to. Conservative: default is the existing downward open.
  function recomputeOpenDirection() {
    const trigger = wrapperEl?.querySelector<HTMLElement>(".combobox-trigger");
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    // Keep in sync with the .combobox-content max-height (260px).
    const needed = 260;
    openUp = shouldOpenUpward(spaceBelow, spaceAbove, needed);
  }

  // Reset the query whenever the popover closes so the next open starts clean.
  function handleOpenChange(next: boolean) {
    open = next;
    if (next) {
      recomputeOpenDirection();
      pinAncestorScrollOnOpen(wrapperEl);
    } else search = "";
  }
</script>

<div
  class="combobox-wrapper"
  class:combobox-wrapper--disabled={disabled}
  class:combobox-wrapper--up={openUp}
  bind:this={wrapperEl}
>
  {#if label}
    <span class="mx-label" id={labelId}>{label}</span>
  {/if}
  <!-- Inner positioning context wrapping only the trigger (Root renders no box),
       so the non-portaled popover anchors to the trigger rather than the
       label+trigger — otherwise a flipped-up menu floats off by the label
       height. -->
  <div class="combobox-anchor">
  <BitsCombobox.Root
    type="single"
    value={value ?? ""}
    onValueChange={handleValueChange}
    bind:open
    onOpenChange={handleOpenChange}
    inputValue={search}
    items={options}
    {disabled}
  >
    <BitsCombobox.Trigger
      class={warn ? "combobox-trigger combobox-trigger--warn" : "combobox-trigger"}
      aria-labelledby={label ? labelId : undefined}
    >
      <span
        class={selectedLabel
          ? "combobox-trigger-text"
          : "combobox-trigger-text combobox-trigger-text--placeholder"}
      >
        {selectedLabel ?? placeholder}
      </span>
      <svg class="combobox-chevron" viewBox="0 0 24 24" aria-hidden="true">
        <path d="m6 9 6 6 6-6" />
      </svg>
    </BitsCombobox.Trigger>
    <!-- Render inline (no body portal) — see Select.svelte: body-portaling
         lands the popover off-screen across Settings' inner scroll container in
         the Tauri WKWebView. The cards don't clip, so inline positioning shows. -->
    <BitsCombobox.Portal disabled>
      <BitsCombobox.Content class="combobox-content" sideOffset={4}>
        <div class="combobox-search">
          <svg
            class="combobox-search-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <circle cx="11" cy="11" r="7" />
            <path d="M21 21l-4.3-4.3" />
          </svg>
          <BitsCombobox.Input
            class="combobox-search-input"
            placeholder={searchPlaceholder}
            aria-label={label ? `${label} — ${searchPlaceholder}` : searchPlaceholder}
            oninput={(e) => (search = e.currentTarget.value)}
          />
        </div>
        <BitsCombobox.Viewport class="combobox-viewport">
          {#each filtered as option (option.value)}
            <BitsCombobox.Item
              value={option.value}
              label={option.label}
              class="combobox-item"
            >
              {#snippet children({ selected })}
                <span class="combobox-item-check" aria-hidden="true">{selected ? "✓" : ""}</span>
                {option.label}
              {/snippet}
            </BitsCombobox.Item>
          {/each}
          {#if filtered.length === 0}
            <div class="combobox-empty">No matches</div>
          {/if}
        </BitsCombobox.Viewport>
      </BitsCombobox.Content>
    </BitsCombobox.Portal>
  </BitsCombobox.Root>
  </div>
</div>

<style>
  .combobox-wrapper {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }

  /* Positioning context for the (non-portaled) popover. Wraps ONLY the trigger
     so both the downward `top` and upward `bottom` rules resolve against the
     trigger box — not the label+trigger, which would float a flipped-up menu
     off by the label height. */
  .combobox-anchor {
    position: relative;
    width: 100%;
  }

  /* See Select.svelte: floating-ui's JS positioning is wrong across Settings'
     inner scroll container in the WKWebView, so pin the inline popover to the
     trigger with pure CSS instead. */
  .combobox-anchor :global([data-bits-floating-content-wrapper]) {
    position: absolute !important;
    inset: auto auto auto 0 !important;
    top: calc(100% + 4px) !important;
    transform: none !important;
    width: 100% !important;
    min-width: 0 !important;
  }

  /* Flip upward when there isn't enough room below the trigger (measured on
     open). Anchors the panel above the trigger instead of below — still pinned,
     never drifting. */
  .combobox-wrapper--up .combobox-anchor :global([data-bits-floating-content-wrapper]) {
    top: auto !important;
    bottom: calc(100% + 4px) !important;
  }

  .combobox-wrapper--disabled {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  /* Trigger mirrors Select's kit control exactly (--h-md, flat border, sans). */
  :global(.combobox-trigger) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    height: var(--h-md);
    padding: 0 10px 0 var(--s-3);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: var(--r-md);
    cursor: pointer;
    outline: none;
    font: 400 var(--text-md)/1 var(--font-sans);
    color: var(--app-text-muted);
    text-align: left;
    transition: border-color var(--t-fast) var(--ease-quart), box-shadow var(--t-med) var(--ease-quart);
  }

  :global(.combobox-trigger:hover),
  :global(.combobox-trigger[data-state="open"]) {
    border-color: var(--app-border-hover);
  }

  :global(.combobox-trigger:focus-visible) {
    border-color: var(--app-accent-border);
    box-shadow: var(--app-ring);
  }

  :global(.combobox-trigger--warn) {
    border-color: var(--app-warn-border);
  }

  .combobox-trigger-text {
    color: var(--app-text-strong);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .combobox-trigger-text--placeholder {
    color: var(--app-text-subtle);
  }

  .combobox-chevron {
    display: block;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    fill: none;
    stroke: var(--app-text-subtle);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform var(--t-med) var(--ease-expo);
  }

  :global(.combobox-trigger[data-state="open"]) .combobox-chevron {
    transform: rotate(180deg);
  }

  /* Popover + rows follow the kit's .mx-pop / .mx-menu (see Select). */
  :global(.combobox-content) {
    background: var(--app-surface-raised);
    border: 1px solid var(--app-overlay-border);
    border-radius: var(--r-lg);
    padding: 4px;
    box-shadow: var(--app-shadow-popover);
    z-index: 100;
    min-width: var(--bits-floating-anchor-width, 200px);
    max-height: 260px;
    overflow: hidden;
  }

  /* Search row pinned to the top of the popover; flush to the menu edges. */
  .combobox-search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 9px;
    margin: -4px -4px 4px;
    border-bottom: 1px solid var(--mx-hairline);
    border-radius: var(--r-lg) var(--r-lg) 0 0;
  }

  .combobox-search-icon {
    width: 13px;
    height: 13px;
    flex: 0 0 13px;
    color: var(--app-text-subtle);
  }

  :global(.combobox-search-input) {
    flex: 1 1 auto;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    font: 400 var(--text-md)/1.3 var(--font-sans);
    color: var(--app-text-strong);
    padding: 0;
  }

  :global(.combobox-search-input::placeholder) {
    color: var(--app-text-subtle);
  }

  :global(.combobox-viewport) {
    overflow-y: auto;
    max-height: 210px;
  }

  :global(.combobox-item) {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: none;
    border-radius: var(--r-sm);
    background: transparent;
    font: 400 var(--text-md)/1.3 var(--font-sans);
    color: var(--app-text);
    text-align: left;
    cursor: pointer;
    outline: none;
    user-select: none;
  }

  :global(.combobox-item:hover),
  :global(.combobox-item[data-highlighted]) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  :global(.combobox-item[data-selected]) {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }

  .combobox-item-check {
    width: 12px;
    font-size: 10px;
    color: var(--app-accent);
    flex-shrink: 0;
    font-family: inherit;
  }

  .combobox-empty {
    padding: 14px 10px;
    text-align: center;
    font: 400 var(--text-base)/1.3 var(--font-sans);
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.combobox-trigger),
    .combobox-chevron {
      transition: none;
    }
  }
</style>
