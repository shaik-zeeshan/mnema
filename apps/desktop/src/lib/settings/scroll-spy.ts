// Pure scroll-spy helpers for the Settings shell (`routes/settings/+page.svelte`).
// Framework-free so they're unit-testable without the shell's `$state`/effects.

import type { SettingsSectionId } from "./groups";

// How far below the scroll region's top edge (which sits just under the 52px
// header) a section's top must cross to become the active one: once its heading
// sits in the column's top band, with the section above all but scrolled away.
// Past the anchors' `scroll-margin-top` (24px), so a rail click / deeplink,
// which parks the section at that margin, counts as crossed.
export const SPY_LINE_PX = 80;

// The active section for the current scroll position: the LAST section (in
// render order) whose top has crossed `line`; the first section when none has
// yet; the final section once the region bottoms out (a short tail can't scroll
// its top up to the line). `top`/`line` share one coordinate space.
export function activeSectionAt(
  sections: { id: SettingsSectionId; top: number }[],
  line: number,
  atBottom: boolean,
): SettingsSectionId | null {
  if (sections.length === 0) return null;
  if (atBottom) return sections[sections.length - 1].id;
  let id = sections[0].id;
  for (const s of sections) if (s.top <= line) id = s.id;
  return id;
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

// True once a programmatic scroll has effectively reached its target anchor —
// the scroll region's `scrollTop` is within a few px of where the anchor will
// rest at the top of the viewport. The scroll-spy uses this to clear its
// suppression flag on a scroll-SETTLE signal rather than a blind timer: a long
// smooth jump across a large distance can outlast a fixed timeout, and clearing
// suppression mid-animation lets the spy re-derive a mid-path
// section (a transient rail-highlight flicker). Geometry-only so it's testable
// without a real DOM element. `target` is null when no programmatic scroll is in
// flight (returns false — nothing to settle against).
export function isAtScrollTarget(
  scrollTop: number,
  target: number | null,
  tolerance = 4,
): boolean {
  if (target === null) return false;
  return Math.abs(scrollTop - target) <= tolerance;
}
