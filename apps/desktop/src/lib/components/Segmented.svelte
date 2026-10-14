<script lang="ts">
  import { tip } from "./tooltip";
  import { tick, type Snippet } from "svelte";
  import {
    focusableIndex as computeFocusableIndex,
    navTargetIndex,
  } from "./segmented-nav";

  interface Option {
    value: string;
    label: string;
    /**
     * Accessible name for the segment, when it should differ from the visible
     * `label` (e.g. compact pills that show only an icon). Falls back to
     * `label` when omitted.
     */
    ariaLabel?: string;
  }

  interface Props {
    /** The selectable options, rendered left to right. */
    options: Option[];
    /** The currently selected value (bindable). */
    value: string;
    /** Called whenever a different segment is chosen. */
    onValueChange?: (v: string) => void;
    /** Disables the whole group. */
    disabled?: boolean;
    /**
     * Individual option values to disable while keeping the rest interactive.
     * Disabled segments can't be clicked and are skipped by keyboard nav.
     */
    disabledValues?: string[];
    /**
     * Optional leading-icon snippet, keyed by option value. Receives the
     * option `value` so a single snippet can switch on it:
     *   {#snippet icon(value)} … {/snippet}
     * Icons render at 12×12 inside each segment, before the label.
     */
    icon?: Snippet<[string]>;
    /** Optional aria-label for the group container. */
    ariaLabel?: string;
    /** Tighter segments (icon-only / dense rows). */
    compact?: boolean;
    /** Kit `--md` size: 32px instead of the default 26px. */
    md?: boolean;
    /** Kit `--mono`: data-valued options (1m · 5m, 0.5 fps). */
    mono?: boolean;
  }

  let {
    options,
    value = $bindable(),
    onValueChange,
    disabled = false,
    disabledValues = [],
    icon,
    ariaLabel,
    compact = false,
    md = false,
    mono = false,
  }: Props = $props();

  const isOff = (v: string): boolean => disabledValues.includes(v);

  // Per-segment button refs, so keyboard nav can move DOM focus onto the newly
  // active segment (focus-follows-selection — the roving tabindex alone leaves
  // focus stranded on the now tabindex=-1 button).
  let segEls = $state<(HTMLButtonElement | null)[]>([]);
  let groupEl = $state<HTMLDivElement | null>(null);

  // The kit's sliding thumb under the selected segment (kit.js `slide()`).
  let thumb = $state<{ x: number; w: number } | null>(null);
  function placeThumb() {
    const el = segEls[options.findIndex((o) => o.value === value)];
    thumb = el ? { x: el.offsetLeft, w: el.offsetWidth } : null;
  }
  $effect(placeThumb);
  // Re-measure when the group resizes (web fonts swap in after first layout).
  $effect(() => {
    if (!groupEl) return;
    const ro = new ResizeObserver(placeThumb);
    ro.observe(groupEl);
    return () => ro.disconnect();
  });

  function select(next: string) {
    if (disabled || isOff(next) || next === value) return;
    value = next;
    onValueChange?.(next);
  }

  // Click handler: select, then pull DOM focus onto the clicked segment. The
  // Tauri WKWebView doesn't focus a <button> on click, so without this the
  // roving tabindex has no anchor and a follow-up arrow key does nothing.
  function selectByClick(index: number) {
    select(options[index].value);
    segEls[index]?.focus();
  }

  // After a keyboard selection, move focus to the new segment — but only when
  // focus is already inside this group, so we never steal focus on mount or on
  // a programmatic value change.
  function focusSelected(index: number) {
    if (groupEl?.contains(document.activeElement)) {
      segEls[index]?.focus();
    }
  }

  // Roving tabindex: exactly one enabled segment is tab-reachable. Prefer the
  // active value, but if it's disabled (or there's no active value) fall back to
  // the first enabled segment — otherwise the whole group becomes
  // keyboard-unreachable when the selected value is also in disabledValues.
  // -1 when every option is disabled (nothing focusable, which is correct).
  // Index math lives in segmented-nav.ts so it's unit-testable.
  const focusableIndex = $derived(
    computeFocusableIndex(options, disabledValues, value),
  );

  function onKeydown(event: KeyboardEvent, index: number) {
    if (disabled) return;
    const nextIndex = navTargetIndex(options, disabledValues, index, event.key);
    if (nextIndex === null) return;
    const target = nextIndex;
    event.preventDefault();
    select(options[target].value);
    // Selecting flips the roving tabindex to `target`; follow it with DOM focus
    // so the new segment is what the user is actually on. tick() lets the
    // tabindex/active classes update first.
    void tick().then(() => focusSelected(target));
  }
</script>

<div
  bind:this={groupEl}
  class="mx-seg"
  class:mx-seg--md={md}
  class:mx-seg--mono={mono}
  class:seg-compact={compact}
  role="radiogroup"
  aria-label={ariaLabel}
  aria-disabled={disabled || undefined}
>
  {#if thumb}
    <span class="mx-seg__thumb" style:width="{thumb.w}px" style:transform="translateX({thumb.x}px)"></span>
  {/if}
  {#each options as option, index (option.value)}
    <button
      type="button"
      bind:this={segEls[index]}
      role="radio"
      aria-checked={value === option.value}
      aria-label={option.ariaLabel ?? option.label}
      use:tip={option.ariaLabel ?? option.label}
      tabindex={index === focusableIndex ? 0 : -1}
      disabled={disabled || isOff(option.value)}
      onclick={() => selectByClick(index)}
      onkeydown={(e) => onKeydown(e, index)}
    >
      {#if icon}
        <span class="seg__icon" aria-hidden="true">{@render icon(option.value)}</span>
      {/if}
      {#if option.label}
        <span>{option.label}</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  /* Look lives in kit.css (.mx-seg). Only component extras here. */
  .mx-seg[aria-disabled="true"] {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  /* Whole-group disable dims once, on the group (not again per segment). */
  .mx-seg[aria-disabled="true"] button:disabled {
    opacity: 1;
  }

  .seg-compact button {
    padding: 0 7px;
  }

  .seg__icon,
  .seg__icon :global(svg) {
    display: block;
    width: 13px;
    height: 13px;
    flex: 0 0 auto;
  }

  .mx-seg--md .seg__icon,
  .mx-seg--md .seg__icon :global(svg) {
    width: 14px;
    height: 14px;
  }
</style>
