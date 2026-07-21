<script lang="ts">
  // RailResizer — the draggable boundary between the Insights left rail and the
  // active sub-surface. It is the visible 1px divider (the rail dropped its own
  // border-right) AND the grab strip: a 7px hit area painted with the rail's own
  // surface, with the hairline flush to its RIGHT (main-facing) edge. The strip
  // must carry the rail surface, not stay transparent: as a flex sibling it adds
  // 7px between the rail and main, and a transparent strip lets the window
  // background show through — which on the Chat tab (white main pane) reads as the
  // rail's cream bleeding past the divider. Painting it as the rail and seating
  // the hairline at the cream→main seam keeps the boundary clean in every theme.
  // Drag (pointer), arrow-key nudges, and double-click-to-reset all feed
  // the shell's persisted `railWidth`; the shell owns clamping + storage. Follows
  // the WAI-ARIA window-splitter pattern (focusable `role="separator"` carrying
  // aria-valuenow/min/max). Visual cues — hairline-as-handle, a grip that surfaces
  // on hover/focus, a focus ring — are borrowed from shadcn-svelte's Resizable.
  interface Props {
    width: number;
    min: number;
    max: number;
    // Report a new desired width (already in pixels; the shell re-clamps + persists).
    onWidth: (width: number) => void;
    // Double-click → snap back to the default width.
    onReset: () => void;
  }

  let { width, min, max, onWidth, onReset }: Props = $props();

  // Keyboard nudge step. Coarser than 1px so arrow-resizing is usable; Home/End
  // jump to the clamps.
  const STEP = 16;

  let dragging = $state(false);

  function onPointerDown(event: PointerEvent): void {
    // Primary button only — let context-menu / middle-click fall through.
    if (event.button !== 0) return;
    event.preventDefault();

    const startX = event.clientX;
    const startWidth = width;
    const handle = event.currentTarget as HTMLElement;
    const pointerId = event.pointerId;

    dragging = true;
    document.body.classList.add("rail-resizing");
    // Capture so the drag survives the pointer leaving the thin strip (or the
    // window) — moves keep landing on the handle until release.
    handle.setPointerCapture(pointerId);

    const move = (e: PointerEvent) => onWidth(startWidth + (e.clientX - startX));
    const up = () => {
      dragging = false;
      document.body.classList.remove("rail-resizing");
      handle.releasePointerCapture(pointerId);
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
    };

    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  }

  function onKeyDown(event: KeyboardEvent): void {
    switch (event.key) {
      case "ArrowLeft":
        event.preventDefault();
        onWidth(width - STEP);
        break;
      case "ArrowRight":
        event.preventDefault();
        onWidth(width + STEP);
        break;
      case "Home":
        event.preventDefault();
        onWidth(min);
        break;
      case "End":
        event.preventDefault();
        onWidth(max);
        break;
    }
  }
</script>

<!-- A focusable, keyboard-operable separator is the WAI-ARIA window-splitter
     pattern; Svelte's a11y lint treats `separator` as non-interactive, so the
     tabindex + listeners trip false positives. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="rail-resizer"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label="Resize sidebar"
  aria-valuenow={width}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  onpointerdown={onPointerDown}
  ondblclick={onReset}
  onkeydown={onKeyDown}
>
  <span class="line" aria-hidden="true"></span>
  <span class="grip" aria-hidden="true"></span>
</div>

<style>
  /* The grab strip that owns the rail/main divider. It takes NO layout width:
     the negative margin folds the 7px hit area back over the rail's right-edge
     gutter (it is a later sibling, so it paints/hits above the rail), and the
     hairline sits flush at the strip's right edge — exactly the rail→main seam.
     When the strip occupied its own 7px of flex space it widened the rail's
     visual right gutter to 23px vs the 16px left one, which read as dead space. */
  .rail-resizer {
    position: relative;
    flex: 0 0 auto;
    width: 7px;
    margin-left: -7px;
    align-self: stretch;
    cursor: col-resize;
    /* Don't let a quick drag select sub-surface text. */
    touch-action: none;
    -webkit-user-select: none;
    user-select: none;
  }

  /* The hairline divider — flush to the strip's RIGHT (main-facing) edge so it
     sits on the cream→main seam, full height. Quiet by default, accent on
     hover / drag / keyboard focus. */
  .rail-resizer .line {
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    width: 1px;
    background: var(--app-border);
    transition: background 0.12s ease;
  }
  .rail-resizer:hover .line,
  .rail-resizer.dragging .line {
    background: var(--app-accent-border);
  }

  /* A small grip pill centered on the hairline — hidden until the strip is
     hovered/focused/dragged so the resting state stays a clean 1px line. */
  .rail-resizer .grip {
    position: absolute;
    top: 50%;
    right: -1px;
    transform: translateY(-50%);
    width: 3px;
    height: 26px;
    border-radius: 2px;
    background: var(--app-accent-border);
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .rail-resizer:hover .grip,
  .rail-resizer.dragging .grip {
    opacity: 1;
  }

  /* Keyboard focus — surface the grip and ring the hairline with the accent so
     the (otherwise invisible) separator is clearly the focused control. */
  .rail-resizer:focus-visible {
    outline: none;
  }
  .rail-resizer:focus-visible .line {
    background: var(--app-accent);
    box-shadow: 0 0 0 1px var(--app-accent-glow);
  }
  .rail-resizer:focus-visible .grip {
    opacity: 1;
    background: var(--app-accent);
  }

  /* While dragging, hold a global resize cursor + suppress selection everywhere
     so the cursor doesn't flicker as the pointer crosses sub-surface content. */
  :global(body.rail-resizing) {
    cursor: col-resize;
    -webkit-user-select: none;
    user-select: none;
  }
</style>
