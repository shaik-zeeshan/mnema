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
    <span class="combobox-label" id={labelId}>{label}</span>
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

  .combobox-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  /* Trigger mirrors Select's recessed trigger exactly so the two read as one
     family (radius 8, mono value, chevron, inset shadow, accent focus ring). */
  :global(.combobox-trigger) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 7px 10px;
    background: var(--app-surface);
    border: 0;
    border-radius: 8px;
    cursor: pointer;
    outline: none;
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-border-strong);
    transition: box-shadow 0.15s;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    gap: 8px;
    text-align: left;
  }

  :global(.combobox-trigger:hover) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-border-hover);
  }

  :global(.combobox-trigger:active) {
    background: var(--app-surface-active);
  }

  :global(.combobox-trigger:focus-visible),
  :global(.combobox-trigger[data-state="open"]) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-accent),
      0 0 0 3px var(--app-accent-glow);
    outline: none;
  }

  :global(.combobox-trigger--warn) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-warn-border);
  }

  :global(.combobox-trigger--warn:focus-visible) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-warn-strong),
      0 0 0 3px color-mix(in srgb, var(--app-warn) 18%, transparent);
  }

  .combobox-trigger-text {
    color: var(--app-text);
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
    stroke: var(--app-text-muted);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 0.15s, stroke 0.15s;
  }

  :global(.combobox-trigger[data-state="open"]) .combobox-chevron {
    transform: rotate(180deg);
    stroke: var(--app-accent);
  }

  /* Level 5: a searchable pop-up is still an NSMenu — material, 5px padding. */
  :global(.combobox-content) {
    background: var(--glass-pop);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border: 0;
    border-radius: var(--r-lg);
    padding: 5px;
    box-shadow: var(--sh-float), inset 0 0 0 var(--hairline) var(--glass-line);
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
    margin: -5px -5px 5px;
    background: var(--glass-tint);
    box-shadow: inset 0 -1px 0 var(--glass-line);
    border-radius: var(--r-lg) var(--r-lg) 0 0;
  }

  .combobox-search-icon {
    width: 13px;
    height: 13px;
    flex: 0 0 13px;
    color: var(--app-text-muted);
  }

  :global(.combobox-search-input) {
    flex: 1 1 auto;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    color: var(--app-text);
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
    padding: 6px 10px;
    border-radius: 3px;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    color: var(--app-text);
    cursor: pointer;
    transition: background 0.1s, color 0.1s;
    outline: none;
    user-select: none;
    border: none;
    background: transparent;
    width: 100%;
    text-align: left;
  }

  /* Selected FIRST so a selected+highlighted row still takes the highlight. */
  :global(.combobox-item[data-selected]) {
    color: var(--app-text-strong);
  }

  :global(.combobox-item:hover),
  :global(.combobox-item[data-highlighted]) {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }

  :global(.combobox-item:hover) .combobox-item-check,
  :global(.combobox-item[data-highlighted]) .combobox-item-check {
    color: inherit;
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
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-meta);
    font-style: italic;
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.combobox-trigger),
    .combobox-chevron,
    :global(.combobox-item) {
      transition: none;
    }
  }
</style>
