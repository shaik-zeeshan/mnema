<script lang="ts">
  import { Select as BitsSelect } from "bits-ui";
  import { pinAncestorScrollOnOpen } from "./pin-scroll-on-open";
  import { shouldOpenUpward } from "./popover-direction";

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
    /** Accessible name for the trigger when there is no visible `label`. */
    ariaLabel?: string;
    warn?: boolean;
    /** Show a loading row instead of an empty/options list while async-fetching. */
    loading?: boolean;
    /** Copy for the no-options row (when not loading). */
    emptyText?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    options,
    placeholder = "Select…",
    disabled = false,
    label,
    ariaLabel,
    warn = false,
    loading = false,
    emptyText = "No options",
  }: Props = $props();

  let openUp = $state(false);
  let wrapperEl = $state<HTMLDivElement | null>(null);

  // Stable id so the visible label can be programmatically associated with the
  // trigger via aria-labelledby (the label renders as a plain <span>).
  const labelId = `select-label-${Math.random().toString(36).slice(2, 9)}`;

  function handleValueChange(v: string) {
    value = v;
    onValueChange?.(v);
  }

  const selectedLabel = $derived(
    value ? (options.find((o) => o.value === value)?.label ?? value) : null
  );

  // The inline popover can't drift, so it can clip at the bottom of Settings'
  // inner scroll container. On open, measure room below vs. above the trigger
  // and flip upward when there isn't enough room below (and there's more above).
  // `max-height` (CSS) still bounds the panel; this just picks the anchor edge.
  function recomputeOpenDirection() {
    const trigger = wrapperEl?.querySelector<HTMLElement>(".select-trigger");
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    // Keep in sync with the .select-content max-height (220px).
    const needed = 220;
    openUp = shouldOpenUpward(spaceBelow, spaceAbove, needed);
  }

  function handleOpenChange(next: boolean) {
    if (next) {
      recomputeOpenDirection();
      pinAncestorScrollOnOpen(wrapperEl);
    }
  }
</script>

<div
  class="select-wrapper"
  class:select-wrapper--disabled={disabled}
  class:select-wrapper--busy={loading && !disabled}
  class:select-wrapper--up={openUp}
  bind:this={wrapperEl}
>
  {#if label}
    <span class="mx-label" id={labelId}>{label}</span>
  {/if}
  <!-- Inner positioning context wrapping only the trigger (Root renders no box),
       so the non-portaled popover anchors to the trigger rather than the
       label+trigger — otherwise a flipped-up menu floats off by the label
       height. -->
  <div class="select-anchor">
  <BitsSelect.Root
    type="single"
    value={value ?? ""}
    onValueChange={handleValueChange}
    onOpenChange={handleOpenChange}
    disabled={disabled || loading}
  >
    <BitsSelect.Trigger
      class={warn ? "select-trigger select-trigger--warn" : "select-trigger"}
      aria-labelledby={label ? labelId : undefined}
      aria-label={label ? undefined : ariaLabel}
    >
      <span class={selectedLabel ? "select-trigger-text" : "select-trigger-text select-trigger-text--placeholder"}>
        {selectedLabel ?? placeholder}
      </span>
      {#if loading}
        <span class="mx-spin mx-spin--sm" aria-hidden="true"></span>
      {:else}
        <svg class="select-chevron" viewBox="0 0 24 24" aria-hidden="true">
          <path d="m6 9 6 6 6-6" />
        </svg>
      {/if}
    </BitsSelect.Trigger>
    <!-- Render inline (no body portal). bits-ui defaults to portaling the
         content to <body>; across Settings' inner `.settings-scroll` container
         that body-relative positioning lands the popover off-screen in the
         Tauri WKWebView (the trigger's rect is measured in a different scroll
         coordinate space). The cards deliberately don't clip overflow, so an
         inline popover positioned within the row's local context shows
         correctly — this matches ModelPickerMenu's "positioned, not portaled"
         approach for Settings. -->
    <BitsSelect.Portal disabled>
      <BitsSelect.Content class="select-content" sideOffset={4}>
        <BitsSelect.Viewport class="select-viewport">
          {#each options as option (option.value)}
            <BitsSelect.Item value={option.value} label={option.label} class="select-item">
              {#snippet children({ selected })}
                <span class="select-item-check" aria-hidden="true">{selected ? "✓" : ""}</span>
                {option.label}
              {/snippet}
            </BitsSelect.Item>
          {/each}
          {#if loading}
            <div class="select-empty" role="status">Loading…</div>
          {:else if options.length === 0}
            <div class="select-empty">{emptyText}</div>
          {/if}
        </BitsSelect.Viewport>
      </BitsSelect.Content>
    </BitsSelect.Portal>
  </BitsSelect.Root>
  </div>
</div>

<style>
  .select-wrapper {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }

  /* Positioning context for the (non-portaled) popover. Wraps ONLY the trigger
     so both the downward `top` and upward `bottom` rules resolve against the
     trigger box — not the label+trigger, which would float a flipped-up menu
     off by the label height. */
  .select-anchor {
    position: relative;
    width: 100%;
  }

  /* bits-ui positions the popover with floating-ui (JS measurement of the
     trigger rect). Inside Settings' inner `.settings-scroll` container that
     measurement is wrong in the Tauri WKWebView, so the menu floats away from
     its trigger. Since we render inline (Portal disabled), pin the floating
     wrapper to the trigger with pure CSS instead — deterministic, no JS rect,
     matching ModelPickerMenu's non-portaled positioning. */
  .select-anchor :global([data-bits-floating-content-wrapper]) {
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
  .select-wrapper--up .select-anchor :global([data-bits-floating-content-wrapper]) {
    top: auto !important;
    bottom: calc(100% + 4px) !important;
  }

  .select-wrapper--disabled {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  /* In-flight (e.g. an async ActionSelect pick): locked like disabled but dimmed
     less, so it reads as "working" rather than "unavailable". The trigger shows
     a spinner in place of the chevron. */
  .select-wrapper--busy {
    opacity: var(--app-busy-opacity);
    pointer-events: none;
    cursor: progress;
  }

  /* Kit control: --h-md, flat 1px border, sans value (settings.html .st-select). */
  :global(.select-trigger) {
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

  :global(.select-trigger:hover),
  :global(.select-trigger[data-state="open"]) {
    border-color: var(--app-border-hover);
  }

  :global(.select-trigger:focus-visible) {
    border-color: var(--app-accent-border);
    box-shadow: var(--app-ring);
  }

  :global(.select-trigger--warn) {
    border-color: var(--app-warn-border);
  }

  .select-trigger-text {
    color: var(--app-text-strong);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .select-trigger-text--placeholder {
    color: var(--app-text-subtle);
  }

  .select-chevron {
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

  :global(.select-trigger[data-state="open"]) .select-chevron {
    transform: rotate(180deg);
  }

  /* Popover + rows follow the kit's .mx-pop / .mx-menu. Selected = --mx-selected
     fill + stronger text; the green check is the selection mark. */
  :global(.select-content) {
    background: var(--app-surface-raised);
    border: 1px solid var(--app-overlay-border);
    border-radius: var(--r-lg);
    padding: 4px;
    box-shadow: var(--app-shadow-popover);
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

  :global(.select-item:hover),
  :global(.select-item[data-highlighted]) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  :global(.select-item[data-selected]) {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }

  .select-item-check {
    width: 12px;
    font-size: 10px;
    color: var(--app-accent);
    flex-shrink: 0;
    font-family: inherit;
  }

  /* Mirrors Combobox's .combobox-empty so a blank/loading popover reads as a
     state, not a dead-end. */
  .select-empty {
    padding: 14px 10px;
    text-align: center;
    font: 400 var(--text-base)/1.3 var(--font-sans);
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.select-trigger),
    .select-chevron {
      transition: none;
    }
  }
</style>
