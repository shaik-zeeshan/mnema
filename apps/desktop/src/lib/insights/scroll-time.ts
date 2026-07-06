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

/** The row occupying viewport line `y` (client coords): the first, in document
 * order, whose bottom edge is below `y`. Pass the viewport's vertical center to
 * read the centered row. Null when scrolled past all rows. */
export function rowAtViewportY(
  rows: { atMs: number; top: number; bottom: number }[],
  y: number,
): number | null {
  for (const row of rows) {
    if (row.bottom > y) return row.atMs;
  }
  return null;
}
