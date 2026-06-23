// Pure scroll-spy helpers for the Settings shell.
//
// The shell's scroll-spy (`routes/settings/+page.svelte`) has two edge cases the
// IntersectionObserver alone can't handle, each fixed in a dedicated commit:
//
//  • Tail selection — the spy marks a section active only once its head clears
//    the top detection band (the negative-bottom `rootMargin`). The FINAL section
//    of a group has nothing below it to scroll its anchor that far up, so it never
//    wins the top-most test; on bottom-out we force-select it instead. The "which
//    section is the tail" decision is `lastSectionOfGroup`.
//
//  • Programmatic-scroll suppression / bottom-out detection — `isScrolledToBottom`
//    is the predicate both the scroll handler (to force the tail) and the observer
//    callback (to NOT override the forced tail back to a mid-band section) key off.
//
// These are framework-free so they're unit-testable in isolation (the shell's
// `$state`/effects aren't), mirroring how `rail-filter.ts` / `groups.ts` split
// their pure logic out of `SettingsRail.svelte` / the shell.

import {
  SETTINGS_GROUPS,
  type SettingsGroupId,
  type SettingsSectionId,
} from "./groups";

// The last sub-section of a group — the "tail" the scroll-spy can't otherwise
// reach (see module header). Returns null for an unknown group or an empty one.
export function lastSectionOfGroup(
  group: SettingsGroupId,
): SettingsSectionId | null {
  const sections = SETTINGS_GROUPS.find((g) => g.id === group)?.sections ?? [];
  return sections.length ? sections[sections.length - 1].id : null;
}

// True once the scroll region has effectively bottomed out (small tolerance for
// fractional scroll heights / sub-pixel rounding). Geometry-only: takes the three
// scroll metrics so it's testable without a real DOM element.
export function isScrolledToBottom(metrics: {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}): boolean {
  return (
    metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= 2
  );
}
