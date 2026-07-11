<script lang="ts">
  // ConclusionHero — the hero card atop ConclusionTimeline: status pills, the
  // full statement, Pin/Dismiss actions, and the confidence readout (big pct +
  // trend + inline area sparkline + trajectory summary). Split out of
  // ConclusionTimeline so each file stays a single responsibility (hero vs the
  // timeline track) and under the source-size ceiling. Ports the mockup's
  // .cd-card / .cd-left / .cd-right / .cd-spark CSS against the app tokens.

  import type { Conclusion, SubjectTrajectory } from "$lib/types/recording";
  import { humanizeHours } from "$lib/insights/activity-helpers";
  import IconPin from "~icons/lucide/pin";
  import IconDismiss from "~icons/lucide/x";
  import IconTrendUp from "~icons/lucide/trending-up";
  import IconTrendDown from "~icons/lucide/trending-down";
  import IconSteady from "~icons/lucide/minus";

  interface Props {
    conclusion: Conclusion;
    trajectory: SubjectTrajectory | undefined;
    actionId?: number | null;
    actionKind?: "pin" | "dismiss" | null;
    onTogglePin: (id: number, pinned: boolean) => void;
    onDismiss: (id: number) => void;
  }

  let {
    conclusion,
    trajectory,
    actionId = null,
    actionKind = null,
    onTogglePin,
    onDismiss,
  }: Props = $props();

  type Trend = "up" | "steady" | "down";

  function pct(confidence: number): number {
    return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
  }
  function clamp01(v: number): number {
    return Math.max(0, Math.min(1, v));
  }

  const isFaded = $derived(conclusion.status === "faded");

  // Header trend: faded reads as cooling; else first→last snapshot with a
  // ±0.04 dead-band (mirrors SubjectDetail's trendFor).
  const headerTrend = $derived.by<Trend>(() => {
    if (isFaded) return "down";
    const h = trajectory?.history ?? [];
    if (h.length >= 2) {
      const d = h[h.length - 1].confidence - h[0].confidence;
      if (d > 0.04) return "up";
      if (d < -0.04) return "down";
      return "steady";
    }
    return "steady";
  });
  const trendLabel = $derived(
    headerTrend === "up"
      ? "rising"
      : headerTrend === "down"
        ? "cooling"
        : "steady",
  );

  // Trajectory summary line: "N snapshots · rose/fell X→Y · over Zh".
  const trajNote = $derived.by(() => {
    const h = trajectory?.history ?? [];
    if (h.length === 0) return null;
    const n = h.length;
    const first = pct(h[0].confidence);
    const last = pct(h[h.length - 1].confidence);
    const span = h[h.length - 1].snapshotAtMs - h[0].snapshotAtMs;
    let prefix: string;
    let value: string;
    if (n < 2) {
      // Single snapshot: no movement to report — just where it sits.
      prefix = "at";
      value = `${last}%`;
    } else if (last > first) {
      prefix = "rose";
      value = `${first}→${last}`;
    } else if (last < first) {
      prefix = "fell";
      value = `${first}→${last}`;
    } else {
      prefix = "steady near";
      value = `${last}`;
    }
    return { n, prefix, value, span: span > 0 ? humanizeHours(span) : null };
  });

  // Inline area sparkline (mockup .cd-spark look — fill + line + end dot).
  // Ported rather than reusing charts/Sparkline: that component is line-only,
  // capped at 140px, and has no end-dot — this needs the full-width filled hero.
  const spark = $derived.by(() => {
    const h = trajectory?.history ?? [];
    // A single point is a meaningless floating dot — hide the sparkline until
    // there's actual movement to draw.
    if (h.length < 2) return null;
    // Hero chart: full-width, ~100px tall (see .cd-spark svg height). Wide
    // viewBox keeps the non-scaling stroke crisp and the end dot near-round
    // under preserveAspectRatio="none".
    const W = 320;
    const H = 110;
    const PAD = 8;
    const pts = h.map((s, i) => ({
      x: h.length <= 1 ? W - PAD : PAD + (i / (h.length - 1)) * (W - PAD * 2),
      y: PAD + (1 - clamp01(s.confidence)) * (H - PAD * 2),
    }));
    const coords = pts.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`);
    const last = pts[pts.length - 1];
    const fill = `M${coords.join("L")}L${last.x.toFixed(1)},${H}L${pts[0].x.toFixed(1)},${H}Z`;
    return { W, H, line: coords.join(" "), fill, last };
  });

  const busyPin = $derived(actionId === conclusion.id && actionKind === "pin");
  const busyDismiss = $derived(
    actionId === conclusion.id && actionKind === "dismiss",
  );
</script>

<div class="cd-card">
  <div class="cd-left">
    <div class="cd-left-top">
      <div class="cd-status">
        {#if conclusion.pinned}
          <span class="pill pill--pinned"
            ><IconPin aria-hidden="true" /> pinned</span
          >
        {/if}
        <span class="pill">{isFaded ? "faded" : "visible"}</span>
      </div>
      <!-- Full statement — NEVER truncated. -->
      <p class="cd-statement">{conclusion.statement}</p>
    </div>
    <div class="cd-actions">
      <button
        type="button"
        class="btn"
        class:btn--pinned={conclusion.pinned}
        class:btn--busy={busyPin}
        disabled={actionId !== null}
        onclick={() => onTogglePin(conclusion.id, !conclusion.pinned)}
      >
        {#if busyPin}
          <span class="btn-spinner" aria-hidden="true"></span>
          {conclusion.pinned ? "Unpinning…" : "Pinning…"}
        {:else if conclusion.pinned}
          <IconPin aria-hidden="true" /> Pinned — protected from decay
        {:else}
          <IconPin aria-hidden="true" /> Pin
        {/if}
      </button>
      <button
        type="button"
        class="btn btn--ghost"
        class:btn--busy={busyDismiss}
        disabled={actionId !== null}
        onclick={() => onDismiss(conclusion.id)}
      >
        {#if busyDismiss}
          <span class="btn-spinner" aria-hidden="true"></span>
          Dismissing…
        {:else}
          <IconDismiss aria-hidden="true" /> Dismiss
        {/if}
      </button>
    </div>
  </div>

  <div class="cd-right">
    <div class="cd-conf-row">
      <div class="cd-conf-big num" class:is-faded={isFaded}>
        {pct(conclusion.confidence)}%
      </div>
      <div class="cd-conf-meta">
        <span
          class="cd-conf-trend"
          class:up={headerTrend === "up"}
          class:steady={headerTrend === "steady"}
          class:down={headerTrend === "down"}
        >
          {#if headerTrend === "up"}
            <IconTrendUp aria-hidden="true" />
          {:else if headerTrend === "down"}
            <IconTrendDown aria-hidden="true" />
          {:else}
            <IconSteady aria-hidden="true" />
          {/if}
          {trendLabel}
        </span>
        <span class="cd-conf-cap">confidence</span>
      </div>
    </div>

    {#if spark}
      <div class="cd-spark">
        <svg
          viewBox="0 0 {spark.W} {spark.H}"
          preserveAspectRatio="none"
          role="img"
          aria-label="Confidence trajectory"
        >
          <path class="spark-fill" d={spark.fill} />
          <polyline class="spark-line" points={spark.line} />
          <circle
            class="spark-dot"
            cx={spark.last.x.toFixed(1)}
            cy={spark.last.y.toFixed(1)}
            r="3.2"
          />
        </svg>
      </div>
    {/if}

    {#if trajNote}
      <div class="cd-traj-note num">
        <span
          ><b>{trajNote.n}</b>
          {trajNote.n === 1 ? "snapshot" : "snapshots"}</span
        >
        <span class="sep">·</span>
        <span>{trajNote.prefix} <b>{trajNote.value}</b></span>
        {#if trajNote.span}
          <span class="sep">·</span>
          <span>over <b>{trajNote.span}</b></span>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .num {
    font-variant-numeric: tabular-nums;
  }
  /* Inline lucide icons: inherit color (currentColor), sit centered with text. */
  .cd-actions .btn :global(svg),
  .cd-conf-trend :global(svg) {
    width: 14px;
    height: 14px;
    flex: 0 0 auto;
  }
  .pill :global(svg) {
    width: 12px;
    height: 12px;
    flex: 0 0 auto;
  }
  .cd-card {
    display: grid;
    /* ~50/50 (a touch more to the statement) so the confidence panel gets real
       room for the hero chart. Columns stretch to the taller one (default
       align-items) — the left content then centers itself vertically, and a
       short right column (sparse history) just makes the whole card compact. */
    grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr);
    gap: 0;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    overflow: hidden;
    background: var(--app-surface);
    margin-bottom: 18px;
  }
  .cd-left {
    padding: 16px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 12px;
  }
  /* Top group (pills + statement) pins to the top; .cd-actions sits at the
     bottom via .cd-left's space-between. */
  .cd-left-top {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .cd-status {
    display: flex;
    gap: 6px;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 2px 9px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    color: var(--app-text-muted);
  }
  .pill--pinned {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .cd-statement {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.55;
    color: var(--app-text-strong);
    font-weight: 500;
  }
  .cd-actions {
    display: flex;
    gap: 8px;
  }
  .cd-actions .btn {
    flex: 1 1 0;
    justify-content: center;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: var(--text-base);
    padding: 6px 11px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .btn:hover:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .btn:not(:disabled):active {
    transform: translateY(1px);
  }
  .btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .btn--busy:disabled {
    opacity: 1;
    cursor: progress;
  }
  .btn--pinned {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .btn--pinned:hover:not(:disabled) {
    border-color: var(--app-accent);
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .btn--ghost {
    border-color: transparent;
    color: var(--app-text-muted);
  }
  .btn--ghost:hover:not(:disabled) {
    background: var(--app-surface-hover);
    color: var(--app-danger);
    border-color: transparent;
  }
  .btn-spinner {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--app-border-hover);
    border-top-color: var(--app-text-strong);
    animation: btn-spin 0.6s linear infinite;
    flex: 0 0 auto;
  }
  @keyframes btn-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .cd-right {
    padding: 16px;
    border-left: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 10px;
  }
  .cd-conf-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .cd-conf-big {
    font-size: var(--text-xl);
    line-height: 1;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-accent-strong);
  }
  .cd-conf-big.is-faded {
    color: var(--app-text-subtle);
  }
  .cd-conf-meta {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cd-conf-trend {
    font-size: var(--text-sm);
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .cd-conf-trend.up {
    color: var(--app-accent-strong);
  }
  .cd-conf-trend.steady {
    color: var(--app-text-muted);
  }
  .cd-conf-trend.down {
    color: var(--app-danger);
  }
  .cd-conf-cap {
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.13em;
    color: var(--app-text-subtle);
  }
  .cd-spark {
    width: 100%;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
  }
  .cd-spark svg {
    display: block;
    width: 100%;
    height: 100px;
  }
  .spark-fill {
    fill: var(--app-accent);
    opacity: 0.1;
  }
  .spark-line {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .spark-dot {
    fill: var(--app-accent);
    stroke: var(--app-surface);
    stroke-width: 1.5;
  }
  .cd-traj-note {
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .cd-traj-note b {
    color: var(--app-text-strong);
    font-weight: 600;
  }
  .cd-traj-note .sep {
    color: var(--app-text-faint);
  }

  @media (max-width: 820px) {
    .cd-card {
      grid-template-columns: 1fr;
    }
    .cd-right {
      border-left: none;
      border-top: 1px solid var(--app-border);
    }
    .cd-spark svg {
      height: 56px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .btn {
      transition: none;
    }
    .btn:not(:disabled):active {
      transform: none;
    }
    .btn-spinner {
      animation: none;
    }
  }
</style>
