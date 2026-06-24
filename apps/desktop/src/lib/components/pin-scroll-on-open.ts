// Neutralizes the outer-container scroll jump that bits-ui Select/Combobox cause
// when opened inline (Portal disabled) inside an overflow-y:auto container: bits-ui
// calls scrollIntoView({block:"nearest"}) on the highlighted item, which scrolls
// every scrollable ancestor. The jump can land on any frame after async floating-ui
// placement, so instead of racing a fixed number of frames we snapshot the nearest
// scrollable ancestor's scrollTop on open and hold it there via a scroll listener
// for a brief window — the popover's own viewport still scrolls its items freely.
// The pin releases immediately on a genuine user gesture (wheel/touch/pointer/key)
// so it never fights intentional scrolling, and always tears down after the window.

// Snap-back decision for the pin's scroll listener, extracted as a pure
// predicate so it's unit-testable without the DOM. Snap the ancestor back to
// the pinned `top` only while the pin is NOT released and the scroll has
// actually drifted off the pin. A real user gesture releases the pin first
// (so `released` short-circuits), and a scroll already sitting at `top`
// (e.g. our own corrective write) is left alone — never fight it.
export function shouldSnapBack(
  released: boolean,
  currentScrollTop: number,
  pinnedTop: number,
): boolean {
  return !released && currentScrollTop !== pinnedTop;
}

function nearestScrollableAncestor(el: HTMLElement | null): HTMLElement | null {
  let node: HTMLElement | null = el?.parentElement ?? null;
  while (node) {
    const oy = getComputedStyle(node).overflowY;
    if ((oy === "auto" || oy === "scroll") && node.scrollHeight > node.clientHeight) return node;
    node = node.parentElement;
  }
  return null;
}

// Start from the component WRAPPER element (so the popover's own viewport, a
// descendant, is never picked). Call when the dropdown transitions to open.
export function pinAncestorScrollOnOpen(wrapper: HTMLElement | null, windowMs = 300): void {
  if (typeof window === "undefined") return;
  const ancestor = nearestScrollableAncestor(wrapper);
  if (!ancestor) return;

  const top = ancestor.scrollTop;
  let released = false;

  const onScroll = () => {
    // A programmatic scrollIntoView fired — snap back. A real user gesture would
    // have released the pin first (handlers below), so this only counters the jump.
    if (shouldSnapBack(released, ancestor.scrollTop, top)) ancestor.scrollTop = top;
  };

  const release = () => {
    if (released) return;
    released = true;
    ancestor.removeEventListener("scroll", onScroll);
    ancestor.removeEventListener("wheel", release);
    ancestor.removeEventListener("touchstart", release);
    ancestor.removeEventListener("pointerdown", release);
    ancestor.removeEventListener("keydown", release);
  };

  ancestor.addEventListener("scroll", onScroll, { passive: true });
  ancestor.addEventListener("wheel", release, { passive: true });
  ancestor.addEventListener("touchstart", release, { passive: true });
  ancestor.addEventListener("pointerdown", release);
  ancestor.addEventListener("keydown", release);
  window.setTimeout(release, windowMs);
}
