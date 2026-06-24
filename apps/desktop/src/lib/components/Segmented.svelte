<script lang="ts">
  import { tick, type Snippet } from "svelte";

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

  // After a keyboard selection, move focus to the new segment — but only when
  // focus is already inside this group, so we never steal focus on mount or on
  // a programmatic value change.
  function focusSelected(index: number) {
    const group = segEls[index]?.closest(".segmented");
    if (group && group.contains(document.activeElement)) {
      segEls[index]?.focus();
    }
  }

  // Step from `from` in `dir` (+1/-1), skipping disabled options. Returns the
  // first enabled index, or null if every option is disabled.
  function nextEnabledIndex(from: number, dir: number): number | null {
    const n = options.length;
    for (let step = 1; step <= n; step += 1) {
      const candidate = (((from + dir * step) % n) + n) % n;
      if (!isOff(options[candidate].value)) return candidate;
    }
    return null;
  }

  // Roving tabindex: exactly one enabled segment is tab-reachable. Prefer the
  // active value, but if it's disabled (or there's no active value) fall back to
  // the first enabled segment — otherwise the whole group becomes
  // keyboard-unreachable when the selected value is also in disabledValues.
  // -1 when every option is disabled (nothing focusable, which is correct).
  const focusableIndex = $derived.by(() => {
    const activeIndex = options.findIndex((o) => o.value === value);
    if (activeIndex !== -1 && !isOff(options[activeIndex].value)) {
      return activeIndex;
    }
    return options.findIndex((o) => !isOff(o.value));
  });

  function onKeydown(event: KeyboardEvent, index: number) {
    if (disabled) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = nextEnabledIndex(index, 1);
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = nextEnabledIndex(index, -1);
    } else if (event.key === "Home") {
      nextIndex = isOff(options[0].value) ? nextEnabledIndex(0, 1) : 0;
    } else if (event.key === "End") {
      const last = options.length - 1;
      nextIndex = isOff(options[last].value) ? nextEnabledIndex(last, -1) : last;
    }
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
  class="segmented"
  class:segmented--compact={compact}
  class:segmented--disabled={disabled}
  role="radiogroup"
  aria-label={ariaLabel}
>
  {#each options as option, index (option.value)}
    <button
      type="button"
      bind:this={segEls[index]}
      class="seg"
      class:seg--active={value === option.value}
      class:seg--off={isOff(option.value)}
      role="radio"
      aria-checked={value === option.value}
      aria-label={option.ariaLabel ?? option.label}
      title={option.ariaLabel ?? option.label}
      tabindex={index === focusableIndex ? 0 : -1}
      disabled={disabled || isOff(option.value)}
      onclick={() => select(option.value)}
      onkeydown={(e) => onKeydown(e, index)}
    >
      {#if icon}
        <span class="seg__icon" aria-hidden="true">{@render icon(option.value)}</span>
      {/if}
      {#if option.label}
        <span class="seg__label">{option.label}</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: inline-flex;
    /* Hug the options even inside a stretch flex column, so the pill doesn't
       blow out to full width with the segments packed on one side. Callers that
       want a full-width control opt in by setting width:100% (+ flex segments),
       as ThemeModeControl does. */
    width: fit-content;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface);
  }

  .segmented--disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .seg {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 5px 12px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 12px;
    font-weight: 540;
    line-height: 1;
    cursor: pointer;
    user-select: none;
    outline: none;
    transition: background 0.15s, color 0.15s, box-shadow 0.15s;
  }

  .seg:hover:not(.seg--active) {
    color: var(--app-text);
    background: var(--app-surface-hover);
  }

  .seg:focus-visible {
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .seg--active {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 1px var(--app-accent-border);
  }

  .seg:disabled {
    cursor: not-allowed;
  }

  /* Individually disabled segment (group stays interactive). */
  .seg--off {
    opacity: 0.4;
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

  /* ── Compact (titlebar pill) ─────────────────────────────────── */
  .segmented--compact {
    padding: 2px;
    gap: 2px;
    border-radius: 999px;
    border-color: var(--app-icon-border-hover);
    background: var(--app-surface-raised);
  }

  .segmented--compact .seg {
    padding: 4px 7px;
    border-radius: 999px;
    color: var(--app-icon-fg);
  }

  .segmented--compact .seg:hover:not(.seg--active) {
    background: var(--app-icon-bg-hover);
    color: var(--app-icon-fg-hover);
  }

  .segmented--compact .seg--active {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 1px var(--app-accent-border);
  }

  .segmented--compact .seg__icon,
  .segmented--compact .seg__icon :global(svg) {
    width: 16px;
    height: 16px;
  }
</style>
