// scroll-time — pure math for the Journal scroll-time bubble
// (ScrollTimeBubble.svelte). No DOM types so it stays bun-testable.

/** Scroll progress 0..1, clamped; 0 when the content doesn't scroll. */
export function scrollFraction(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
): number {
  const maxScroll = scrollHeight - clientHeight;
  if (maxScroll <= 0) return 0;
  return Math.min(1, Math.max(0, scrollTop / maxScroll));
}

/** Inverse mapping for dragging the bubble: pointer Y on the track → scrollTop,
 * clamped to [0, maxScroll]. */
export function dragToScrollTop(
  pointerY: number,
  trackTop: number,
  trackHeight: number,
  scrollHeight: number,
  clientHeight: number,
): number {
  const maxScroll = scrollHeight - clientHeight;
  if (maxScroll <= 0 || trackHeight <= 0) return 0;
  const fraction = Math.min(1, Math.max(0, (pointerY - trackTop) / trackHeight));
  return fraction * maxScroll;
}

/** First row (document order) still visible at the viewport top: the first
 * whose bottom edge is below `viewportTop`. Null when scrolled past all rows. */
export function topmostVisibleAtMs(
  rows: { atMs: number; top: number; bottom: number }[],
  viewportTop: number,
): number | null {
  for (const row of rows) {
    if (row.bottom > viewportTop) return row.atMs;
  }
  return null;
}
