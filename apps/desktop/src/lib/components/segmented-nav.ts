// Pure, framework-free index math for the Segmented control's keyboard nav.
// Extracted out of Segmented.svelte so it can be unit-tested without
// Svelte/runes — the component keeps its DOM/focus orchestration and only the
// index math lives here. Operates on the options array plus the list of
// disabled values, so there is no DOM dependency.

export interface SegmentedNavOption {
  value: string;
}

/** True when `value` is in the disabled list and so should be skipped. */
function isOff(disabledValues: string[], value: string): boolean {
  return disabledValues.includes(value);
}

// Step from `from` in `dir` (+1/-1), skipping disabled options. Returns the
// first enabled index, or null if every option is disabled.
export function nextEnabledIndex(
  options: SegmentedNavOption[],
  disabledValues: string[],
  from: number,
  dir: number,
): number | null {
  const n = options.length;
  for (let step = 1; step <= n; step += 1) {
    const candidate = (((from + dir * step) % n) + n) % n;
    if (!isOff(disabledValues, options[candidate].value)) return candidate;
  }
  return null;
}

// Roving tabindex: exactly one enabled segment is tab-reachable. Prefer the
// active value, but if it's disabled (or there's no active value) fall back to
// the first enabled segment — otherwise the whole group becomes
// keyboard-unreachable when the selected value is also in disabledValues.
// -1 when every option is disabled (nothing focusable, which is correct).
export function focusableIndex(
  options: SegmentedNavOption[],
  disabledValues: string[],
  value: string,
): number {
  const activeIndex = options.findIndex((o) => o.value === value);
  if (activeIndex !== -1 && !isOff(disabledValues, options[activeIndex].value)) {
    return activeIndex;
  }
  return options.findIndex((o) => !isOff(disabledValues, o.value));
}

// Resolve the keyboard target index for a nav key, or null when the key is not
// a nav key or there is no enabled segment to move to. Home/End land on the
// first/last ENABLED segment (falling back inward when the edge is disabled).
export function navTargetIndex(
  options: SegmentedNavOption[],
  disabledValues: string[],
  index: number,
  key: string,
): number | null {
  if (key === "ArrowRight" || key === "ArrowDown") {
    return nextEnabledIndex(options, disabledValues, index, 1);
  }
  if (key === "ArrowLeft" || key === "ArrowUp") {
    return nextEnabledIndex(options, disabledValues, index, -1);
  }
  if (key === "Home") {
    return isOff(disabledValues, options[0].value)
      ? nextEnabledIndex(options, disabledValues, 0, 1)
      : 0;
  }
  if (key === "End") {
    const last = options.length - 1;
    return isOff(disabledValues, options[last].value)
      ? nextEnabledIndex(options, disabledValues, last, -1)
      : last;
  }
  return null;
}
