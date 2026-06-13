<script lang="ts">
  // FocusDetailModal — the click-to-expand breakdown behind the dashboard's
  // small "Focus" heatmap card. Shows the full focus story the card can't fit:
  // a Deep/Mixed/Scattered time distribution, the same day×time-of-day heatmap
  // at full modal width, and the longest individual focused sessions.
  //
  // Structure/behaviour mirror CategoryDetailModal exactly (Escape-to-close,
  // backdrop pointerdown-to-close, panel focus handoff for WebKit, overlay/
  // panel/header chrome) for visual + interaction consistency.

  import type { Activity } from "$lib/types/recording";
  import { humanizeMs, focusHint } from "$lib/insights/activity-helpers";
  import Heatmap from "$lib/insights/charts/Heatmap.svelte";

  interface FocusRow {
    label: string;
    cells: number[];
  }

  interface Props {
    open: boolean;
    activities: Activity[];
    focusRows: FocusRow[];
    rangeMode: "day" | "week" | "month";
    rangeLabel: string;
    onClose: () => void;
  }

  let { open, activities, focusRows, rangeMode, rangeLabel, onClose }: Props =
    $props();

  // Local clock formatter (identical to CategoryDetailModal's helper).
  function clockTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

  type FocusLevel = "deep" | "mixed" | "distracted";

  // Stable presentation order + tokens/labels for the distribution rows.
  const FOCUS_VIEW: { level: FocusLevel; colorVar: string }[] = [
    { level: "deep", colorVar: "--focus-deep" },
    { level: "mixed", colorVar: "--focus-mid" },
    { level: "distracted", colorVar: "--focus-distracted" },
  ];

  // The headline metric: total focused time grouped by focus level (null
  // focus ignored), with each level's share of the focused total.
  interface FocusSlice {
    level: FocusLevel;
    colorVar: string;
    label: string;
    totalMs: number;
    pct: number; // 0..100, of the focused total
  }
  const distribution = $derived.by<{ slices: FocusSlice[]; totalMs: number }>(
    () => {
      const totals: Record<FocusLevel, number> = {
        deep: 0,
        mixed: 0,
        distracted: 0,
      };
      for (const a of activities) {
        if (a.focus == null) continue;
        totals[a.focus] += Math.max(0, a.endedAtMs - a.startedAtMs);
      }
      const totalMs = totals.deep + totals.mixed + totals.distracted;
      const slices = FOCUS_VIEW.map(({ level, colorVar }) => ({
        level,
        colorVar,
        label: focusHint(level),
        totalMs: totals[level],
        pct: totalMs > 0 ? (totals[level] / totalMs) * 100 : 0,
      }));
      return { slices, totalMs };
    },
  );

  // The longest individual focused sessions, newest-irrelevant — sorted by
  // duration descending and capped so the list stays scannable.
  interface FocusSession {
    id: number;
    title: string;
    focus: FocusLevel;
    colorVar: string;
    startedAtMs: number;
    durationMs: number;
  }
  const sessions = $derived.by<FocusSession[]>(() => {
    const colorFor: Record<FocusLevel, string> = {
      deep: "--focus-deep",
      mixed: "--focus-mid",
      distracted: "--focus-distracted",
    };
    return activities
      .filter((a): a is Activity & { focus: FocusLevel } => a.focus != null)
      .map((a) => ({
        id: a.id,
        title: a.title,
        focus: a.focus,
        colorVar: colorFor[a.focus],
        startedAtMs: a.startedAtMs,
        durationMs: Math.max(0, a.endedAtMs - a.startedAtMs),
      }))
      .sort((a, b) => b.durationMs - a.durationMs)
      .slice(0, 12);
  });

  // Backdrop click → close. Guarded so a drag that started inside the panel
  // doesn't dismiss it.
  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return;
    onClose();
  }

  // Move keyboard focus into the dialog when it opens, so Escape/Tab act on
  // the modal immediately (WebKit gives the opener no focus handoff).
  let panelEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (open) panelEl?.focus();
  });
</script>

<!-- Escape closes the modal. No-ops while closed so it never fights other
     surfaces; the tag must stay at the top level (Svelte forbids it in a block). -->
<svelte:window
  onkeydown={(e) => {
    if (!open || e.key !== "Escape") return;
    onClose();
  }}
/>

{#if open}
  <div
    class="focus-modal"
    role="presentation"
    onpointerdown={onBackdropPointerDown}
  >
    <div
      bind:this={panelEl}
      class="focus-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="focus-modal-title"
      tabindex="-1"
    >
      <header class="focus-modal__header">
        <div>
          <p class="focus-modal__eyebrow">{rangeLabel}</p>
          <h2 id="focus-modal-title">Focus breakdown</h2>
        </div>
        <button
          type="button"
          class="focus-modal__close"
          aria-label="Close breakdown"
          onclick={onClose}>×</button
        >
      </header>

      <div class="focus-modal__body">
        <!-- 1. Focus distribution summary (the headline metric). -->
        <section class="section">
          <h3 class="section__title">Focus distribution</h3>
          {#if distribution.totalMs > 0}
            <div class="dist">
              {#each distribution.slices as s (s.level)}
                <div class="dist-row">
                  <span
                    class="dist-swatch"
                    style="background:var({s.colorVar});"
                    aria-hidden="true"
                  ></span>
                  <span class="dist-label">{s.label}</span>
                  <span class="dist-bar" aria-hidden="true">
                    <span
                      class="dist-bar-fill"
                      style="width:{s.pct}%; background:var({s.colorVar});"
                    ></span>
                  </span>
                  <span class="dist-time">{humanizeMs(s.totalMs)}</span>
                  <span class="dist-pct">{Math.round(s.pct)}%</span>
                </div>
              {/each}
            </div>
          {:else}
            <p class="empty-note">No focus signal in this range.</p>
          {/if}
        </section>

        <!-- 2. The full heatmap at modal width. -->
        <section class="section">
          <h3 class="section__title">By time of day</h3>
          <Heatmap
            rows={focusRows}
            colorMode="focus"
            legend="deep · mixed · scattered"
          />
        </section>

        <!-- 3. Longest focused sessions. -->
        <section class="section">
          <h3 class="section__title">Top focus sessions</h3>
          {#if sessions.length === 0}
            <p class="empty-note">No focused sessions in this range.</p>
          {:else}
            <div class="sessions">
              {#each sessions as s (s.id)}
                <div class="session-row">
                  <span
                    class="session-dot"
                    style="background:var({s.colorVar});"
                    aria-hidden="true"
                  ></span>
                  <div class="session-main">
                    <span class="session-title">{s.title}</span>
                    <span class="session-meta"
                      >{clockTime(s.startedAtMs)} · {humanizeMs(
                        s.durationMs,
                      )}</span
                    >
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </section>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay + panel (mirrors CategoryDetailModal) ---- */
  .focus-modal {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.42);
    backdrop-filter: blur(10px);
  }
  .focus-modal__panel {
    width: min(640px, 100%);
    max-height: min(720px, calc(100vh - 48px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface);
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.42);
  }
  .focus-modal__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 18px 12px;
  }
  .focus-modal__eyebrow {
    margin: 0 0 2px;
    font-size: 10.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .focus-modal__header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .focus-modal__close {
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
  .focus-modal__close:hover,
  .focus-modal__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    outline: none;
  }
  .focus-modal__body {
    overflow-y: auto;
    padding: 0 18px 18px;
  }

  /* ---- Sections ---- */
  .section + .section {
    margin-top: 22px;
  }
  .section__title {
    margin: 0 0 10px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .empty-note {
    margin: 0;
    padding: 4px 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }

  /* ---- Focus distribution ---- */
  .dist {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .dist-row {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .dist-swatch {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dist-label {
    flex: 0 0 auto;
    width: 64px;
    font-size: 12.5px;
    color: var(--app-text-strong);
  }
  .dist-bar {
    flex: 1 1 auto;
    min-width: 0;
    height: 6px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }
  .dist-bar-fill {
    display: block;
    height: 100%;
    border-radius: 3px;
  }
  .dist-time {
    flex: 0 0 auto;
    font-size: 11px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    text-align: right;
    min-width: 56px;
  }
  .dist-pct {
    flex: 0 0 auto;
    width: 38px;
    text-align: right;
    font-size: 11px;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  /* ---- Top focus sessions ---- */
  .sessions {
    display: flex;
    flex-direction: column;
  }
  .session-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 0;
  }
  .session-row + .session-row {
    border-top: 1px dashed var(--app-border);
  }
  .session-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .session-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .session-title {
    font-size: 12.5px;
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .session-meta {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }

  @media (prefers-reduced-motion: reduce) {
    .focus-modal__close {
      transition: none;
    }
  }
</style>
