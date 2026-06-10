<script lang="ts">
  // CategoryDetailModal — the per-category activity-thread breakdown lifted out
  // of the Overview feed (Insights redesign, Phase 2). Opened from the
  // "Categories" glance tile; renders the same "what you worked on" thread list
  // (colored dot + label + stats + chevron, expandable to per-activity rows with
  // an "adjust" popover) as a centered overlay.
  //
  // OWNS locally: expanded-thread state, the per-row "adjust" popover state
  // (editingActivity), and a window handler for Escape + outside-click that
  // closes the popover first, then defers to onClose. Corrections delegate to
  // the parent via the onCorrectCategory / onCorrectFocus callbacks.

  import type { Activity } from "$lib/types/recording";
  import {
    CATEGORY_OPTIONS,
    FOCUS_OPTIONS,
    threadStats,
    focusHint,
    humanizeMs,
    type ActivityThread,
    type ActivityCategory,
  } from "$lib/insights/activity-helpers";

  interface Props {
    open: boolean;
    threads: ActivityThread[];
    rangeMode: "day" | "week" | "month";
    rangeLabel: string;
    correctingActivity: Set<number>;
    onClose: () => void;
    onCorrectCategory: (a: Activity, cat: ActivityCategory | null) => void;
    onCorrectFocus: (
      a: Activity,
      focus: "deep" | "mixed" | "distracted" | null,
    ) => void;
  }

  let {
    open,
    threads,
    rangeMode,
    rangeLabel,
    correctingActivity,
    onClose,
    onCorrectCategory,
    onCorrectFocus,
  }: Props = $props();

  function clockTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

  // Expanded threads, keyed by thread key. All collapsed by default.
  let expandedThreads = $state<Set<string>>(new Set());
  function toggleThread(key: string): void {
    const set = new Set(expandedThreads);
    if (set.has(key)) set.delete(key);
    else set.add(key);
    expandedThreads = set;
  }

  // Rows in "adjust" mode, keyed by activity id. At most one popover open at a
  // time — opening one closes any other; an outside click / Escape closes it.
  let editingActivity = $state<Set<number>>(new Set());
  function toggleActivityEdit(id: number): void {
    editingActivity = editingActivity.has(id) ? new Set() : new Set([id]);
  }
  function closeActivityEdit(): void {
    if (editingActivity.size > 0) editingActivity = new Set();
  }

  // Backdrop click → close. Guarded so a click that started inside the panel
  // (e.g. dragging a selection out) doesn't close.
  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target === e.currentTarget) onClose();
  }
</script>

<!-- Escape closes the popover first if one is open, else the modal. An outside
     click closes only the popover (the backdrop owns modal-close). The handlers
     no-op while closed so they never fight other surfaces; the tag itself must
     stay at the top level (Svelte forbids it inside a block). -->
<svelte:window
  onpointerdown={(e) => {
    if (!open) return;
    const target = e.target;
    if (!(target instanceof Element) || !target.closest(".act-adjust")) {
      closeActivityEdit();
    }
  }}
  onkeydown={(e) => {
    if (!open || e.key !== "Escape") return;
    if (editingActivity.size > 0) {
      e.stopPropagation();
      closeActivityEdit();
    } else {
      onClose();
    }
  }}
/>

{#if open}
  <div
    class="cat-modal"
    role="presentation"
    onpointerdown={onBackdropPointerDown}
  >
    <div
      class="cat-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="cat-modal-title"
      tabindex="-1"
    >
      <header class="cat-modal__header">
        <div>
          <p class="cat-modal__eyebrow">{rangeLabel}</p>
          <h2 id="cat-modal-title">What you worked on</h2>
        </div>
        <button
          type="button"
          class="cat-modal__close"
          aria-label="Close breakdown"
          onclick={onClose}>×</button
        >
      </header>

      <div class="cat-modal__body">
        {#if threads.length === 0}
          <p class="cat-modal__empty">No categorized activity in this range.</p>
        {:else}
          <div class="thread-list">
            {#each threads as t (t.key)}
              <div class="thread">
                <button
                  type="button"
                  class="thread-head"
                  aria-expanded={expandedThreads.has(t.key)}
                  onclick={() => toggleThread(t.key)}
                >
                  <span
                    class="thread-dot"
                    style="background:var({t.colorVar});"
                    aria-hidden="true"
                  ></span>
                  <span class="thread-label">{t.label}</span>
                  <span class="thread-stats">{threadStats(t, rangeMode)}</span>
                  <span
                    class="thread-chevron"
                    class:open={expandedThreads.has(t.key)}
                    aria-hidden="true">›</span
                  >
                </button>
                {#if expandedThreads.has(t.key)}
                  <div class="act-list">
                    {#each t.activities as a (a.id)}
                      <div
                        class="act-row"
                        class:act-row--busy={correctingActivity.has(a.id)}
                      >
                        <div class="act-line">
                          <div class="act-main">
                            <span class="act-title">{a.title}</span>
                            <span class="act-time"
                              >{clockTime(a.startedAtMs)} · {humanizeMs(
                                Math.max(0, a.endedAtMs - a.startedAtMs),
                              )}</span
                            >
                          </div>
                          {#if a.focus != null}
                            <span class="act-focus-hint">{focusHint(a.focus)}</span>
                          {/if}
                          <!-- "adjust" opens a popover anchored here. -->
                          <div class="act-adjust">
                            <button
                              type="button"
                              class="evidence-link act-adjust-btn"
                              aria-haspopup="true"
                              aria-expanded={editingActivity.has(a.id)}
                              onclick={() => toggleActivityEdit(a.id)}
                            >
                              adjust
                              <span
                                class="act-adjust-caret"
                                class:open={editingActivity.has(a.id)}
                                aria-hidden="true">▾</span
                              >
                            </button>
                            {#if editingActivity.has(a.id)}
                              <!-- Category moves the row to another thread on the
                                   next recompute — expected, not fought. -->
                              <div
                                class="adjust-pop"
                                role="group"
                                aria-label="Adjust activity"
                              >
                                <label class="corr">
                                  <span class="corr-label">Category</span>
                                  <select
                                    class="corr-select"
                                    value={a.category ?? ""}
                                    disabled={correctingActivity.has(a.id)}
                                    onchange={(e) =>
                                      onCorrectCategory(
                                        a,
                                        (e.currentTarget.value || null) as
                                          | ActivityCategory
                                          | null,
                                      )}
                                  >
                                    {#each CATEGORY_OPTIONS as opt (opt.value)}
                                      <option value={opt.value}>{opt.label}</option>
                                    {/each}
                                  </select>
                                </label>
                                <label class="corr">
                                  <span class="corr-label">Focus</span>
                                  <select
                                    class="corr-select"
                                    value={a.focus ?? ""}
                                    disabled={correctingActivity.has(a.id)}
                                    onchange={(e) =>
                                      onCorrectFocus(
                                        a,
                                        (e.currentTarget.value || null) as
                                          | "deep"
                                          | "mixed"
                                          | "distracted"
                                          | null,
                                      )}
                                  >
                                    {#each FOCUS_OPTIONS as opt (opt.value)}
                                      <option value={opt.value}>{opt.label}</option>
                                    {/each}
                                  </select>
                                </label>
                              </div>
                            {/if}
                          </div>
                        </div>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay + panel (mirrors +layout.svelte .shortcut-help) ---- */
  .cat-modal {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.42);
    backdrop-filter: blur(10px);
  }
  .cat-modal__panel {
    width: min(640px, 100%);
    max-height: min(720px, calc(100vh - 48px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface);
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.42);
  }
  .cat-modal__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 18px 12px;
  }
  .cat-modal__eyebrow {
    margin: 0 0 2px;
    font-size: 10.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .cat-modal__header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .cat-modal__close {
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
  .cat-modal__close:hover,
  .cat-modal__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    outline: none;
  }
  .cat-modal__body {
    overflow-y: auto;
    padding: 0 18px 18px;
  }
  .cat-modal__empty {
    margin: 0;
    padding: 8px 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }

  /* ---- Activity threads / corrections (ported from Overview) ---- */
  .evidence-link {
    font: inherit;
    font-size: 11px;
    color: var(--app-text-muted);
    background: transparent;
    border: none;
    border-bottom: 1px dotted var(--app-border-strong);
    padding: 0 0 1px;
    cursor: pointer;
    white-space: nowrap;
    transition:
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .evidence-link:hover {
    color: var(--app-text-strong);
    border-bottom-color: var(--app-border-hover);
  }

  .thread-list {
    display: flex;
    flex-direction: column;
  }
  .thread + .thread {
    border-top: 1px dashed var(--app-border);
  }
  .thread-head {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 10px 0;
    font: inherit;
    text-align: left;
    background: transparent;
    border: none;
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .thread-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .thread-label {
    flex: 0 0 auto;
    font-size: 12.5px;
    color: var(--app-text-strong);
  }
  .thread-stats {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .thread-chevron {
    flex: 0 0 auto;
    font-size: 13px;
    line-height: 1;
    color: var(--app-text-faint);
    transition:
      transform 0.12s ease,
      color 0.12s ease;
  }
  .thread-chevron.open {
    transform: rotate(90deg);
  }
  .thread-head:hover .thread-chevron {
    color: var(--app-text-strong);
  }
  .act-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 0 8px 17px; /* indent under the dot+gap of the thread header */
  }
  .act-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 9px 0;
    transition: opacity 0.12s ease;
  }
  .act-row + .act-row {
    border-top: 1px dashed var(--app-border);
  }
  .act-row--busy {
    opacity: 0.5;
  }
  .act-line {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .act-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .act-title {
    font-size: 12.5px;
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .act-time {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  /* Quiet read-only focus hint — metadata, not a control. */
  .act-focus-hint {
    flex: 0 0 auto;
    font-size: 9.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  /* "adjust" button + its anchored popover. */
  .act-adjust {
    position: relative;
    flex: 0 0 auto;
  }
  .act-adjust-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .act-adjust-caret {
    font-size: 8px;
    line-height: 1;
    transition: transform 0.12s ease;
  }
  .act-adjust-caret.open {
    transform: rotate(180deg);
  }
  /* Popover panel — anchored to the button's right edge so it never clips the
     panel's right wall. */
  .adjust-pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    min-width: 220px;
    display: flex;
    flex-direction: column;
    gap: 11px;
    padding: 11px 12px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 8px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
  }
  .adjust-pop .corr {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }
  .adjust-pop .corr-select {
    flex: 0 0 auto;
    min-width: 120px;
  }
  .corr {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .corr-label {
    font-size: 9px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .corr-select {
    font: inherit;
    font-size: 11px;
    padding: 3px 6px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text);
    cursor: pointer;
    transition: border-color 0.12s ease;
  }
  .corr-select:hover:not(:disabled) {
    border-color: var(--app-border-hover);
  }
  .corr-select:focus {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .corr-select:disabled {
    opacity: 0.6;
    cursor: default;
  }

  @media (prefers-reduced-motion: reduce) {
    .cat-modal__close,
    .thread-head,
    .thread-chevron,
    .act-row,
    .act-adjust-caret,
    .corr-select,
    .evidence-link {
      transition: none;
    }
  }
</style>
