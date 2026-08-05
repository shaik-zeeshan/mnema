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
    /** Visual size; `compact` is the tighter pill used in titlebars. */
    compact?: boolean;
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
  }: Props = $props();

  const isOff = (v: string): boolean => disabledValues.includes(v);

  // Per-segment button refs, so keyboard nav can move DOM focus onto the newly
  // active segment (focus-follows-selection — the roving tabindex alone leaves
  // focus stranded on the now tabindex=-1 button).
  let segEls = $state<(HTMLButtonElement | null)[]>([]);

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
    const group = segEls[index]?.closest(".seg");
    if (group && group.contains(document.activeElement)) {
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

<!--
  Bento Native: this component IS the direction's NSSegmentedControl. The
  container is `.seg` and each option is `.seg__i` / `.seg__i.on`, so the look
  comes from `lib/bento/bento.css` and nothing about it is forked here. The
  legacy `.segmented` class rides along as an alias — consumers style through it
  (e.g. ThemeModeControl's full-width variant) and the props/API are unchanged.
-->
<div
  class="seg segmented"
  class:seg--sm={compact}
  class:segmented--compact={compact}
  class:segmented--disabled={disabled}
  role="radiogroup"
  aria-label={ariaLabel}
>
  {#each options as option, index (option.value)}
    <button
      type="button"
      bind:this={segEls[index]}
      class="seg__i"
      class:on={value === option.value}
      class:seg--off={isOff(option.value)}
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
  /* Everything visual — the track, the raised "on" chip, the type, the focus
     ring — is bento.css `.seg` / `.seg__i`. What lives here is only what a
     shared stylesheet cannot know about this component. */
  .segmented {
    /* Hug the options even inside a stretch flex column, so the track doesn't
       blow out to full width with the segments packed on one side. Callers that
       want a full-width control opt in by setting width:100%, and the segments
       share it (`.seg__i` is flex:1 1 auto). */
    width: fit-content;
    user-select: none;
    -webkit-user-select: none;
  }

  .segmented--disabled {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  /* Individually disabled segment (group stays interactive). */
  .seg--off {
    opacity: var(--app-disabled-opacity);
  }

  .seg__icon,
  .seg__icon :global(svg) {
    display: block;
    width: 12px;
    height: 12px;
    flex: 0 0 auto;
  }

  .seg__icon :global(svg) {
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Compact = the mockup's `.seg--sm` at 24px; its icons are the only local
     override, since an icon-only segment needs a bigger glyph than a label. */
  .segmented--compact .seg__icon,
  .segmented--compact .seg__icon :global(svg) {
    width: 16px;
    height: 16px;
  }
</style>
