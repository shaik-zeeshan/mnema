<script lang="ts">
  // AppDetailModal — the full app-usage breakdown lifted out of the dashboard
  // "Time" glance card (Insights redesign). The small card only fits the top 5
  // apps as mini bars; this centered overlay shows ALL tracked apps as a denser
  // ranked list (icon + name + proportional bar + time/share/frame readout).
  //
  // Mirrors CategoryDetailModal exactly for visual + behavioral consistency:
  // same overlay/panel chrome, Escape-to-close, backdrop pointerdown-to-close,
  // and the panel focus handoff (WebKit gives the opener no focus). Read-only —
  // it owns no correction state, just renders the rows the parent passes in.

  import { humanizeMs } from "$lib/insights/activity-helpers";

  interface AppRow {
    app: string; // app display name
    activeMs: number; // active time in ms
    frameCount: number; // number of captured frames
    iconSrc: string | null; // resolved icon image src, or null
    fallback: string; // single-letter fallback when no icon
  }
  interface Props {
    open: boolean;
    apps: AppRow[]; // ALL apps, already sorted descending by activeMs
    rangeLabel: string; // e.g. "Jun 2 – 8"
    onClose: () => void;
  }

  let { open, apps, rangeLabel, onClose }: Props = $props();

  // Largest active time → 100% bar width. Guarded so an empty / all-zero list
  // never divides by zero (bars just render empty).
  const maxActiveMs = $derived(
    apps.reduce((m, a) => (a.activeMs > m ? a.activeMs : m), 0),
  );
  // Sum of active time → per-app share %. Guarded the same way.
  const totalActiveMs = $derived(
    apps.reduce((s, a) => s + Math.max(0, a.activeMs), 0),
  );

  function barWidth(ms: number): string {
    if (maxActiveMs <= 0) return "0%";
    return `${Math.max(0, Math.min(100, (ms / maxActiveMs) * 100))}%`;
  }
  function sharePct(ms: number): string {
    if (totalActiveMs <= 0) return "0%";
    return `${Math.round((ms / totalActiveMs) * 100)}%`;
  }
  function framesLabel(count: number): string {
    return `${count.toLocaleString()} ${count === 1 ? "frame" : "frames"}`;
  }

  // Backdrop click → close. Guarded so a click that started inside the panel
  // (e.g. dragging out a selection) doesn't dismiss the modal.
  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return;
    onClose();
  }

  // Move keyboard focus into the dialog when it opens, so Escape/Tab act on the
  // modal immediately (WebKit gives the opener no focus handoff).
  let panelEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (open) panelEl?.focus();
  });
</script>

<!-- Escape closes the modal. The handler no-ops while closed so it never fights
     other surfaces; the tag must stay top level (Svelte forbids it in a block). -->
<svelte:window
  onkeydown={(e) => {
    if (!open || e.key !== "Escape") return;
    onClose();
  }}
/>

{#if open}
  <div class="app-modal" role="presentation" onpointerdown={onBackdropPointerDown}>
    <div
      bind:this={panelEl}
      class="app-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="app-modal-title"
      tabindex="-1"
    >
      <header class="app-modal__header">
        <div>
          <p class="app-modal__eyebrow">{rangeLabel}</p>
          <h2 id="app-modal-title">App usage</h2>
        </div>
        <button
          type="button"
          class="app-modal__close"
          aria-label="Close breakdown"
          onclick={onClose}>×</button
        >
      </header>

      <div class="app-modal__body">
        {#if apps.length === 0}
          <p class="app-modal__empty">No tracked app time in this range.</p>
        {:else}
          <div class="app-list">
            {#each apps as a (a.app)}
              <div class="app-row">
                <div class="app-avatar" aria-hidden="true">
                  {#if a.iconSrc}
                    <img src={a.iconSrc} alt="" />
                  {:else}
                    <span class="app-avatar__fallback">{a.fallback.toUpperCase()}</span>
                  {/if}
                </div>
                <div class="app-main">
                  <div class="app-name-line">
                    <span class="app-name">{a.app}</span>
                    <span class="app-share">{sharePct(a.activeMs)}</span>
                  </div>
                  <div class="app-bar" aria-hidden="true">
                    <span class="app-bar__fill" style="width:{barWidth(a.activeMs)};"></span>
                  </div>
                </div>
                <div class="app-readout">
                  <span class="app-time">{humanizeMs(a.activeMs)}</span>
                  <span class="app-frames">{framesLabel(a.frameCount)}</span>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay + panel (mirrors CategoryDetailModal / .shortcut-help) ---- */
  .app-modal {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.42);
    backdrop-filter: blur(10px);
  }
  .app-modal__panel {
    width: min(640px, 100%);
    max-height: min(720px, calc(100vh - 48px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface);
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.42);
  }
  .app-modal__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 18px 12px;
  }
  .app-modal__eyebrow {
    margin: 0 0 2px;
    font-size: 10.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .app-modal__header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .app-modal__close {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    line-height: 1;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .app-modal__close:hover,
  .app-modal__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    outline: none;
  }
  .app-modal__body {
    overflow-y: auto;
    padding: 0 18px 18px;
  }
  .app-modal__empty {
    margin: 0;
    padding: 8px 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }

  /* ---- App rows — a denser version of the card's mini bars ---- */
  .app-list {
    display: flex;
    flex-direction: column;
  }
  .app-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 0;
  }
  .app-row + .app-row {
    border-top: 1px dashed var(--app-border);
  }
  .app-avatar {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--app-surface-hover);
  }
  .app-avatar img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .app-avatar__fallback {
    font-size: 11px;
    font-weight: 600;
    color: var(--app-text-muted);
    text-transform: uppercase;
    line-height: 1;
  }
  .app-main {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .app-name-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .app-name {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 12.5px;
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .app-share {
    flex: 0 0 auto;
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .app-bar {
    height: 4px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }
  .app-bar__fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--app-accent);
    transition: width 0.18s ease;
  }
  .app-readout {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
    text-align: right;
  }
  .app-time {
    font-size: 12px;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .app-frames {
    font-size: 10.5px;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  @media (prefers-reduced-motion: reduce) {
    .app-modal__close,
    .app-bar__fill {
      transition: none;
    }
  }
</style>
