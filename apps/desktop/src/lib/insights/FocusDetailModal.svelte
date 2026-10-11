<script lang="ts">
  import { formatClock } from "$lib/time-format";
  // FocusDetailModal — "Focus detail" behind Overview's focus column, in the
  // insights.html `.in-modal` chrome: the Deep/Mixed/Scattered split (by
  // activity count), the focus heat at modal width, and the longest sessions.
  // Escape / backdrop close, Tab trapped, focus handed in and back (WebKit).

  import { tick } from "svelte";
  import type { Activity } from "$lib/types/recording";
  import { trapTabKey } from "$lib/keyboard";
  import { focusHint } from "$lib/insights/activity-helpers";
  import { fmtDuration, focusCounts, type HeatRow } from "$lib/insights/lede-stats";
  import Heatmap from "$lib/insights/charts/Heatmap.svelte";
  import FocusSplit from "$lib/insights/FocusSplit.svelte";
  import IconX from "~icons/lucide/x";

  interface Props {
    open: boolean;
    activities: Activity[];
    focusRows: HeatRow[];
    rangeLabel: string;
    onClose: () => void;
  }

  let { open, activities, focusRows, rangeLabel, onClose }: Props = $props();

  // "Tue 09:30" / "Tue 9:30 AM".
  const clockTime = (ms: number): string =>
    `${new Date(ms).toLocaleDateString("en-US", { weekday: "short" })} ${formatClock(ms)}`;

  type FocusLevel = "deep" | "mixed" | "distracted";
  const VAR: Record<FocusLevel, string> = { deep: "--focus-deep", mixed: "--focus-mid", distracted: "--focus-distracted" };

  // The split is by activity COUNT — the same computation as Overview's
  // "Deep focus" stat (ruled 2026-10-10), never span minutes.
  const counts = $derived(focusCounts(activities));

  // The longest individual focused sessions, by duration, capped.
  const sessions = $derived(
    activities
      .filter((a): a is Activity & { focus: FocusLevel } => a.focus != null)
      .map((a) => ({ id: a.id, title: a.title, focus: a.focus, startedAtMs: a.startedAtMs, durationMs: Math.max(0, a.endedAtMs - a.startedAtMs) }))
      .sort((a, b) => b.durationMs - a.durationMs)
      .slice(0, 12),
  );

  // Backdrop click → close. Guarded so a drag that started inside the panel
  // doesn't dismiss it.
  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return;
    onClose();
  }

  // Move keyboard focus into the dialog when it opens, so Escape/Tab act on
  // the modal immediately (WebKit gives the opener no focus handoff). On open we
  // also capture the opener so focus can return to it on close (else it falls to
  // <body>); mirrors ModelPickerMenu's trigger-focus return.
  let panelEl = $state<HTMLDivElement | null>(null);
  let opener: HTMLElement | null = null;
  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      opener = document.activeElement as HTMLElement | null;
      panelEl?.focus();
    } else if (!open && wasOpen) {
      const trigger = opener;
      opener = null;
      void tick().then(() => trigger?.focus());
    }
    wasOpen = open;
  });
</script>

<!-- Escape closes the modal. No-ops while closed so it never fights other
     surfaces; the tag must stay at the top level (Svelte forbids it in a block). -->
<svelte:window
  onkeydown={(e) => {
    if (!open) return;
    // Keep Tab inside the dialog so focus never escapes behind the backdrop.
    if (trapTabKey(e, panelEl)) return;
    if (e.key !== "Escape") return;
    onClose();
  }}
/>

{#if open}
  <div class="in-modal" role="presentation" onpointerdown={onBackdropPointerDown}>
    <div
      bind:this={panelEl}
      class="in-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="focus-modal-title"
      tabindex="-1"
    >
      <div class="in-modal__head">
        <div>
          <span class="mx-kicker">Focus · {rangeLabel}</span>
          <h2 class="mx-h2" id="focus-modal-title">Focus detail</h2>
          <p class="mx-label">
            <span class="num strong">{counts.n}</span> activit{counts.n === 1 ? "y" : "ies"} with a focus read
          </p>
        </div>
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Close" onclick={onClose}>
          <IconX width="15" height="15" aria-hidden="true" />
        </button>
      </div>
      <div class="in-modal__body">
        <p class="in-group">Split</p>
        {#if counts.n}
          <FocusSplit {counts} />
        {:else}
          <p class="mx-label">No focus signal in this range.</p>
        {/if}
        <p class="in-group">By time of day</p>
        <Heatmap rows={focusRows} />
        <p class="in-group">Longest sessions</p>
        {#if sessions.length === 0}
          <p class="mx-label">No focused sessions in this range.</p>
        {:else}
          {#each sessions as s (s.id)}
            <div class="session">
              <span class="sw" style="--c:var({VAR[s.focus]})" aria-hidden="true"></span>
              <span class="title">{s.title}</span>
              <span class="num meta">{clockTime(s.startedAtMs)} · {focusHint(s.focus)}</span>
              <span class="num dur">{fmtDuration(s.durationMs)}</span>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* insights.html `.in-modal` */
  .in-modal {
    position: fixed;
    inset: 0;
    z-index: 2000; /* above the shell, like the sibling modals */
    display: grid;
    place-items: center;
    padding: var(--s-6);
    background: color-mix(in srgb, var(--app-bg) 62%, transparent);
    -webkit-backdrop-filter: blur(4px);
    backdrop-filter: blur(4px);
    animation: fade var(--t-med) var(--ease-quart);
  }
  .in-modal__panel {
    width: min(640px, 100%);
    max-height: min(720px, 100%);
    overflow: auto;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: var(--r-xl);
    box-shadow: var(--app-shadow-popover);
    outline: none;
  }
  .in-modal__head {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--s-2) var(--s-4);
    padding: var(--s-5) var(--s-5) var(--s-4);
    border-bottom: 1px solid var(--mx-hairline);
  }
  .in-modal__head .mx-h2 {
    margin-top: 8px;
  }
  .in-modal__head .mx-label {
    margin: 8px 0 0;
  }
  .strong {
    color: var(--app-text-strong);
  }
  .in-modal__body {
    padding: 0 var(--s-5) var(--s-5);
  }
  .in-group {
    margin: var(--s-5) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .in-group::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--mx-hairline);
  }
  .session {
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) auto 64px;
    gap: var(--s-3);
    align-items: center;
    padding: 9px 0;
    border-top: 1px solid var(--mx-hairline);
    font: 400 var(--text-base) / 1.4 var(--font-sans);
  }
  .in-group + .session {
    border-top: 0;
  }
  .session .title {
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .session .meta {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
  }
  .session .dur {
    font-size: var(--text-sm);
    color: var(--app-text-strong);
    text-align: right;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>
