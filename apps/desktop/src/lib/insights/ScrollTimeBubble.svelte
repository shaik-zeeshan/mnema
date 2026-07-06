<script lang="ts">
  // ScrollTimeBubble — floating scrub pill for the Journal river. While the
  // scrollport scrolls it shows the clock time of the topmost visible row,
  // positioned like a scrollbar thumb (top fraction = scroll fraction), and
  // fades out ~1s after scrolling stops. Dragging it scrubs `scrollTop`.
  // Rendered as a zero-height sticky wrapper (first child of `.river`) so the
  // absolutely-positioned bubble pins to the scrollport's top-right.
  import { dragToScrollTop, scrollFraction, rowAtViewportY } from "./scroll-time";

  // Horizontal inset from the scrollport's right edge — clears the macOS
  // overlay scrollbar so the pill sits beside the thumb, not under it.
  const SCROLLBAR_INSET = 14;
  // Approximate native overlay-scrollbar minimum thumb height.
  const MIN_THUMB = 20;
  const HIDE_DELAY_MS = 1400;
  const FALLBACK_BUBBLE_H = 24;

  // Model the native thumb (height ∝ viewport²/content) so the pill's center
  // can ride the thumb's center instead of drifting apart mid-scroll.
  function thumbHeight(scrollHeight: number, clientHeight: number): number {
    return Math.max((clientHeight * clientHeight) / scrollHeight, MIN_THUMB);
  }

  let anchorEl = $state<HTMLElement | null>(null);
  let bubbleEl = $state<HTMLElement | null>(null);
  let container = $state<HTMLElement | null>(null);
  let visible = $state(false);
  let scrollable = $state(false);
  let dragging = $state(false);
  // Pointer resting on the pill freezes the auto-fade so it can't vanish while
  // the user is reaching to grab it; leaving restarts the timer.
  let hovering = $state(false);
  let topPx = $state(0);
  // The anchor is only as wide as the river column, which sits centered inside
  // a wider scrollport — a negative `right` pushes the bubble out to the
  // scrollport's right edge, where the native scrollbar lives.
  let rightPx = $state(0);
  let label = $state("");

  let hideTimer: ReturnType<typeof setTimeout> | undefined;

  // Same format as the river's clock() helper.
  function clock(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    });
  }

  function findScrollContainer(from: HTMLElement): HTMLElement | null {
    let el: HTMLElement | null = from.parentElement;
    while (el) {
      const overflowY = getComputedStyle(el).overflowY;
      if (overflowY === "auto" || overflowY === "scroll") return el;
      el = el.parentElement;
    }
    return null;
  }

  function sync() {
    const c = container;
    const anchor = anchorEl;
    if (!c || !anchor) return;
    const { scrollTop, scrollHeight, clientHeight } = c;
    // Under ~2 viewports of content the bubble is noise — never show it.
    scrollable = scrollHeight - clientHeight >= clientHeight;
    if (!scrollable) {
      visible = false;
      return;
    }
    const bubbleH = bubbleEl?.offsetHeight ?? FALLBACK_BUBBLE_H;
    const thumbH = thumbHeight(scrollHeight, clientHeight);
    const fraction = scrollFraction(scrollTop, scrollHeight, clientHeight);
    // Pill center = thumb center: thumb top + half thumb, minus half pill.
    topPx = fraction * (clientHeight - thumbH) + thumbH / 2 - bubbleH / 2;

    // The anchor is the first child of the river section, so its parent is
    // the section holding all `[data-at-ms]` row wrappers.
    const river = anchor.parentElement;
    if (!river) return;
    const cRect = c.getBoundingClientRect();
    const containerTop = cRect.top;
    rightPx = anchor.getBoundingClientRect().right - cRect.right + SCROLLBAR_INSET;
    const rows = Array.from(river.querySelectorAll<HTMLElement>("[data-at-ms]")).map((el) => {
      const r = el.getBoundingClientRect();
      return { atMs: Number(el.dataset.atMs), top: r.top, bottom: r.bottom };
    });
    // Read the row at the viewport's vertical center so the pill's time matches
    // what the eye lands on mid-scroll, not the partly-scrolled row up top.
    const atMs = rowAtViewportY(rows, containerTop + clientHeight / 2);
    if (atMs !== null) label = clock(atMs);
  }

  function restartHideTimer() {
    clearTimeout(hideTimer);
    if (dragging || hovering) return;
    hideTimer = setTimeout(() => {
      visible = false;
    }, HIDE_DELAY_MS);
  }

  // Coalesce measurement to one pass per animation frame. `sync()` reads layout
  // for every `[data-at-ms]` row (getBoundingClientRect per row) — a full day is
  // hundreds of rows — so running it synchronously on every scroll event thrashes
  // layout. One rAF-scheduled pass per frame bounds it to the display refresh.
  let syncRaf: number | null = null;
  function scheduleSync() {
    if (syncRaf !== null) return;
    syncRaf = requestAnimationFrame(() => {
      syncRaf = null;
      sync();
      if (scrollable) {
        visible = true;
        restartHideTimer();
      }
    });
  }

  function onScroll() {
    scheduleSync();
  }

  $effect(() => {
    const anchor = anchorEl;
    if (!anchor) return;
    const c = findScrollContainer(anchor);
    container = c;
    if (!c) return;
    c.addEventListener("scroll", onScroll, { passive: true });
    // Fires once on observe, giving us the initial scrollable/position state.
    const ro = new ResizeObserver(() => scheduleSync());
    ro.observe(c);
    return () => {
      c.removeEventListener("scroll", onScroll);
      ro.disconnect();
      if (syncRaf !== null) cancelAnimationFrame(syncRaf);
      syncRaf = null;
      clearTimeout(hideTimer);
      container = null;
    };
  });

  function onPointerDown(e: PointerEvent) {
    if (!container) return;
    // Prevent text selection while scrubbing.
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragging = true;
    visible = true;
    clearTimeout(hideTimer);
  }

  function onPointerMove(e: PointerEvent) {
    const c = container;
    if (!dragging || !c) return;
    const rect = c.getBoundingClientRect();
    const thumbH = thumbHeight(c.scrollHeight, c.clientHeight);
    // Same thumb-center track the position formula uses: the pointer rides the
    // pill's (and thumb's) center while scrubbing.
    c.scrollTop = dragToScrollTop(
      e.clientY,
      rect.top + thumbH / 2,
      c.clientHeight - thumbH,
      c.scrollHeight,
      c.clientHeight,
    );
  }

  function onPointerUp(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    restartHideTimer();
  }

  function onPointerEnter() {
    hovering = true;
    clearTimeout(hideTimer);
  }

  function onPointerLeave() {
    hovering = false;
    restartHideTimer();
  }
</script>

<!-- Supplementary pointer affordance only — the native scrollbar and wheel
     remain the accessible scrolling mechanisms, so this is hidden from AT
     and kept out of the tab order. -->
<div class="anchor" bind:this={anchorEl} aria-hidden="true">
  {#if container}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="bubble"
      class:visible={(visible || dragging) && label !== ""}
      class:dragging
      style="top: {topPx}px; right: {rightPx}px;"
      bind:this={bubbleEl}
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
      onpointerenter={onPointerEnter}
      onpointerleave={onPointerLeave}
    >
      {label}
    </div>
  {/if}
</div>

<style>
  /* Zero-height sticky wrapper: sticks to the scrollport top so the absolute
     bubble inside positions against the visible viewport, not the river. */
  .anchor {
    position: sticky;
    top: 0;
    height: 0;
    z-index: 3;
  }
  .bubble {
    position: absolute;
    padding: 3px 9px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface);
    color: var(--app-text-strong);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    box-shadow: var(--app-shadow-popover);
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
    touch-action: none;
    opacity: 0;
    pointer-events: none;
    transition:
      opacity 0.15s ease,
      background-color 0.12s ease,
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .bubble.visible {
    opacity: 1;
    pointer-events: auto;
  }
  /* :hover only lands while visible — pointer-events is none when hidden. */
  .bubble:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  .bubble.dragging {
    cursor: grabbing;
    background: var(--app-surface-active);
    border-color: var(--app-accent-border);
    box-shadow:
      var(--app-shadow-popover),
      0 0 0 3px var(--app-accent-glow);
  }
  @media (prefers-reduced-motion: reduce) {
    .bubble {
      transition: none;
    }
  }
</style>
