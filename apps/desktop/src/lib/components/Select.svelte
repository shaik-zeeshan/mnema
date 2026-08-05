<script lang="ts">
  import { Select as BitsSelect } from "bits-ui";
  import IconSpinner from "~icons/lucide/loader-circle";
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
    <span class="select-label" id={labelId}>{label}</span>
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
        <span class="select-spinner" aria-hidden="true"><IconSpinner /></span>
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

  .select-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  :global(.select-trigger) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 7px 10px;
    background: var(--app-surface);
    /* Borderless: the control's inset rim IS its ring, layered over the recess
       that says "this is a field you can open". */
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

  :global(.select-trigger:hover) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-border-hover);
  }

  :global(.select-trigger:active) {
    background: var(--app-surface-active);
  }

  :global(.select-trigger:focus-visible),
  :global(.select-trigger[data-state="open"]) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-accent),
      0 0 0 3px var(--app-accent-glow);
    outline: none;
  }

  :global(.select-trigger--warn) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-warn-border);
  }

  :global(.select-trigger--warn:focus-visible) {
    box-shadow: inset 0 1px 2px var(--app-input-recess, rgba(0, 0, 0, 0.25)),
      inset 0 0 0 var(--hairline) var(--app-warn-strong),
      0 0 0 3px color-mix(in srgb, var(--app-warn) 18%, transparent);
  }

  .select-trigger-text {
    color: var(--app-text);
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
    stroke: var(--app-text-muted);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 0.15s, stroke 0.15s;
  }

  :global(.select-trigger[data-state="open"]) .select-chevron {
    transform: rotate(180deg);
    stroke: var(--app-accent);
  }

  /* Busy spinner shown on the closed trigger while a pick is in flight. Rotate
     the wrapper span (not the <svg>): WKWebView doesn't reliably spin an <svg>
     around its own center. Mirrors ButtonSpinner. */
  .select-spinner {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--app-text-muted);
    animation: select-spinner-spin 0.7s linear infinite;
  }

  .select-spinner :global(svg) {
    width: 14px;
    height: 14px;
    stroke-width: 2;
  }

  @keyframes select-spinner-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Level 5: a pop-up button's list is an NSMenu — material, 5px padding. */
  :global(.select-content) {
    background: var(--glass-pop);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border: 0;
    border-radius: var(--r-lg);
    padding: 5px;
    box-shadow: var(--sh-float), inset 0 0 0 var(--hairline) var(--glass-line);
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

  /* NSMenu: the checkmark gutter is what says "this is the current value" — the
     row under the pointer is the one that takes a fill. Selected is declared
     FIRST so a selected+highlighted row still highlights. */
  :global(.select-item[data-selected]) {
    color: var(--app-text-strong);
  }

  :global(.select-item:hover),
  :global(.select-item[data-highlighted]) {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }

  :global(.select-item:hover) .select-item-check,
  :global(.select-item[data-highlighted]) .select-item-check {
    color: inherit;
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
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-meta);
    font-style: italic;
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.select-trigger),
    .select-chevron,
    :global(.select-item) {
      transition: none;
    }
    .select-spinner {
      animation: none;
    }
  }
</style>
