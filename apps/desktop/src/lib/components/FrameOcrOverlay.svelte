<script lang="ts">
  // Presentational OCR surface for FrameDetailModal (slice 3). Purely renders
  // what the parent measured + loaded — no tauri, no state, no geometry. The
  // parent owns lazy loading, polling, and the contained-image rect; this
  // component just paints boxes/chips over that rect and the non-success
  // status pill. Visuals mirror the Timeline's overlay and the mockup, keyed
  // off the global `--app-ocr-*` tokens.

  import type { OcrStatus } from "$lib/frame-ocr";
  import type { OcrObservation } from "$lib/types/app-infra";

  type Rect = { left: number; top: number; width: number; height: number };

  interface Props {
    status: OcrStatus;
    error: string | null;
    observations: OcrObservation[];
    rect: Rect;
    // Per-observation box style (position + --ocr-font-size), from ocrBoxStyle.
    boxStyle: (obs: OcrObservation) => string;
  }

  let { status, error, observations, rect, boxStyle }: Props = $props();

  const positioned = $derived(
    status === "success" && observations.length > 0 && rect.width > 0 && rect.height > 0,
  );
  // Success + text, but the rect isn't measurable (img not laid out yet) — fall
  // back to a note so the recognized text isn't silently unreachable.
  const unpositionable = $derived(
    status === "success" && observations.length > 0 && (rect.width <= 0 || rect.height <= 0),
  );
</script>

<!-- OCR overlay: boxes anchored to the measured contained-image rect. Text is
     exposed to AT via list/listitem + aria-label; no per-box tabindex (dozens
     of tab stops would trip a11y). Chip reveals on hover for pointer users.
     Pointer-events off on the wrapper so the overlay never blocks the image;
     boxes re-enable them. -->
{#if positioned}
  <div
    class="frame-ocr-overlay"
    role="list"
    aria-label="Recognized on-screen text"
    style={`left: ${rect.left}px; top: ${rect.top}px; width: ${rect.width}px; height: ${rect.height}px;`}
  >
    {#each observations as obs, i (i)}
      <div
        class="frame-ocr-box"
        role="listitem"
        style={boxStyle(obs)}
        aria-label={`${obs.text} (${(obs.confidence * 100).toFixed(0)}% confidence)`}
      >
        <span class="frame-ocr-text">{obs.text}</span>
      </div>
    {/each}
    <span class="frame-ocr-hint" aria-hidden="true">hover to read</span>
  </div>
{/if}

{#if status !== "idle" && status !== "success"}
  <div class="frame-ocr-status frame-ocr-status--{status}" role="status" aria-live="polite">
    {#if status === "running"}
      <span class="frame-ocr-spinner" aria-hidden="true"></span>
      <span>loading OCR…</span>
    {:else if status === "empty"}
      <span class="frame-ocr-status-glyph" aria-hidden="true">∅</span>
      <span>no text detected</span>
    {:else if status === "missing"}
      <span class="frame-ocr-status-glyph" aria-hidden="true">∅</span>
      <span>no OCR data for this frame</span>
    {:else if status === "error"}
      <span class="frame-ocr-status-glyph" aria-hidden="true">!</span>
      <span class="frame-ocr-status-msg">{error ?? "OCR failed"}</span>
    {/if}
  </div>
{:else if unpositionable}
  <div class="frame-ocr-status frame-ocr-status--empty" role="status" aria-live="polite">
    <span class="frame-ocr-status-glyph" aria-hidden="true">⌶</span>
    <span>text detected but couldn't be positioned</span>
  </div>
{/if}

<style>
  .frame-ocr-overlay {
    position: absolute;
    overflow: hidden;
    pointer-events: none;
  }
  .frame-ocr-box {
    position: absolute;
    border: 1px solid var(--app-ocr-box);
    background: color-mix(in srgb, var(--app-ocr-box-fill) 18%, transparent);
    border-radius: 2px;
    min-width: 1px;
    min-height: 1px;
    pointer-events: auto;
    cursor: text;
    transition:
      border-color 120ms ease,
      background 120ms ease;
  }
  .frame-ocr-box:hover,
  .frame-ocr-box:focus-within {
    border-color: var(--app-ocr-box-hover);
    background: var(--app-ocr-box-fill);
    box-shadow:
      0 0 0 1px var(--app-ocr-hover-shadow),
      inset 0 0 0 1px var(--app-ocr-hover-inset);
    z-index: 2;
  }
  /* Text hidden until hover: boxes are a quiet scan layer; the revealed chip is
     opaque and sits flush with the bbox so it replaces the glyphs beneath.
     --ocr-font-size comes from ocrBoxStyle so chip text matches the glyph row. */
  .frame-ocr-text {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    padding: 0 4px;
    background: var(--app-ocr-chip-bg);
    color: var(--app-ocr-chip-text);
    text-shadow: var(--app-ocr-chip-text-shadow);
    font-family: var(--app-font-mono);
    font-size: var(--ocr-font-size, 11px);
    line-height: 1;
    letter-spacing: -0.01em;
    white-space: nowrap;
    width: max-content;
    min-width: 100%;
    max-width: none;
    border: 1px solid var(--app-ocr-chip-border);
    border-radius: 2px;
    pointer-events: none;
    user-select: text;
    opacity: 0;
    transition: opacity 80ms ease;
  }
  .frame-ocr-box:hover .frame-ocr-text,
  .frame-ocr-box:focus-within .frame-ocr-text {
    opacity: 1;
    pointer-events: auto;
  }
  .frame-ocr-hint {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 2px 6px;
    border-radius: 3px;
    background: var(--app-ocr-chip-bg);
    color: var(--app-ocr-chip-text);
    border: 1px solid var(--app-ocr-chip-border);
    font-family: var(--app-font-mono);
    font-size: 10px;
    letter-spacing: 0.02em;
    line-height: 1.2;
    white-space: nowrap;
    opacity: 0.72;
    pointer-events: none;
  }

  /* Non-success status pill, pinned bottom-left of the hero. */
  .frame-ocr-status {
    position: absolute;
    left: 8px;
    bottom: 8px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    background: var(--app-overlay-bg);
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    backdrop-filter: blur(4px);
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    max-width: calc(100% - 16px);
  }
  .frame-ocr-status--running {
    color: var(--app-warn);
  }
  .frame-ocr-status--error {
    color: var(--app-danger);
  }
  .frame-ocr-status-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid currentColor;
    font-size: 9px;
    font-weight: 700;
  }
  .frame-ocr-status-msg {
    text-transform: none;
    letter-spacing: 0;
    font-family: var(--app-font-mono);
    word-break: break-word;
    max-width: 320px;
  }
  .frame-ocr-spinner {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid color-mix(in srgb, var(--app-warn) 30%, transparent);
    border-top-color: var(--app-warn);
    animation: frame-ocr-spin 0.9s linear infinite;
  }
  @keyframes frame-ocr-spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .frame-ocr-box,
    .frame-ocr-text {
      transition: none;
    }
    .frame-ocr-spinner {
      animation: none;
    }
  }
</style>
