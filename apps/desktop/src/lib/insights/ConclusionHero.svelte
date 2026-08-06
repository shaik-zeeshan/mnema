<script lang="ts">
  // ConclusionHero — the PINNED-BELIEF hero atop ConclusionTimeline (page 09's
  // `.chero`): pinned/visible chips, the full statement, the only two correction
  // affordances that exist (Pin, Dismiss), the snapshot note, and the page's
  // single display-size number with its area sparkline. Split out of
  // ConclusionTimeline so each file stays a single responsibility (hero vs the
  // trajectory track) and under the source-size ceiling.

  import type { Conclusion, SubjectTrajectory } from "$lib/types/recording";
  import { humanizeHours } from "$lib/insights/activity-helpers";

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

  // Trajectory summary sentence: "9 snapshots · rose 42% → 72% · over 21h".
  // Assembled in JS so the separators keep their spacing (template whitespace
  // around a block gets trimmed) and so every clause is optional and honest —
  // no span claim when the snapshots share a timestamp.
  const trajNote = $derived.by<string | null>(() => {
    const h = trajectory?.history ?? [];
    if (h.length === 0) return null;
    const n = h.length;
    const first = pct(h[0].confidence);
    const last = pct(h[h.length - 1].confidence);
    const span = h[h.length - 1].snapshotAtMs - h[0].snapshotAtMs;
    const parts = [`${n} ${n === 1 ? "snapshot" : "snapshots"}`];
    if (n < 2) {
      // Single snapshot: no movement to report — just where it sits.
      parts.push(`at ${last}%`);
    } else if (last > first) {
      parts.push(`rose ${first}% → ${last}%`);
    } else if (last < first) {
      parts.push(`fell ${first}% → ${last}%`);
    } else {
      parts.push(`steady near ${last}%`);
    }
    if (span > 0) parts.push(`over ${humanizeHours(span)}`);
    return parts.join(" · ");
  });

  // The hero's AREA sparkline (09's .areaspark — 168 x 44, filled). Ported
  // rather than reusing charts/Sparkline, which is line-only and unfilled: the
  // area is what makes this read as the belief's arc rather than a row glyph.
  // Points are evenly spaced by INDEX — the backend stores a confidence
  // history, not a time series, so there is no time axis and never a date.
  const spark = $derived.by(() => {
    const h = trajectory?.history ?? [];
    // A single point is a meaningless floating dot — hide the sparkline until
    // there's actual movement to draw.
    if (h.length < 2) return null;
    const W = 168;
    const H = 44;
    const PAD = 3;
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

<div class="plate chero">
  <div class="chero__l">
    <div class="chero__chips">
      {#if conclusion.pinned}
        <span class="chip chip--on">pinned</span>
      {/if}
      <span class="chip">{isFaded ? "faded" : "visible"}</span>
    </div>
    <!-- Full statement — NEVER truncated. A conclusion's wording is frozen at
         formation; a wrong one is superseded, never revised, so there is no
         edit affordance anywhere on this page. -->
    <h2 class="chero__stmt">{conclusion.statement}</h2>
    <!-- The only two correction affordances that exist: set_pinned and
         dismiss_conclusion. Dismiss is not delete — it raises the resurface bar
         to 2x the evidence that formed the belief, which can re-form later. -->
    <div class="chero__actions">
      <button
        type="button"
        class="btn btn--sm"
        class:btn--accent={conclusion.pinned}
        class:btn--busy={busyPin}
        disabled={actionId !== null}
        onclick={() => onTogglePin(conclusion.id, !conclusion.pinned)}
      >
        {#if busyPin}
          <span class="btn-spinner" aria-hidden="true"></span>
          {conclusion.pinned ? "Unpinning…" : "Pinning…"}
        {:else if conclusion.pinned}
          <span class="star" aria-hidden="true">★</span> Pinned — protected from decay
        {:else}
          <span class="star" aria-hidden="true">★</span> Pin
        {/if}
      </button>
      <button
        type="button"
        class="btn btn--ghost btn--sm"
        class:btn--busy={busyDismiss}
        disabled={actionId !== null}
        onclick={() => onDismiss(conclusion.id)}
      >
        {#if busyDismiss}
          <span class="btn-spinner" aria-hidden="true"></span>
          Dismissing…
        {:else}
          Dismiss
        {/if}
      </button>
    </div>
    {#if trajNote}
      <p class="chero__note num">{trajNote}</p>
    {/if}
  </div>

  <!-- The one display-size number on the page. -->
  <div class="chero__r">
    <span class="chero__big num" class:is-faded={isFaded}
      >{pct(conclusion.confidence)}%</span
    >
    <span
      class="chero__trend"
      class:up={headerTrend === "up"}
      class:down={headerTrend === "down"}
    >{trendLabel}</span>
    <span class="chero__cap">confidence</span>

    {#if spark}
      <svg
        class="areaspark"
        viewBox="0 0 {spark.W} {spark.H}"
        preserveAspectRatio="none"
        role="img"
        aria-label="Confidence trajectory"
      >
        <defs>
          <linearGradient id="chero-area" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="var(--app-accent)" stop-opacity="0.34" />
            <stop offset="100%" stop-color="var(--app-accent)" stop-opacity="0" />
          </linearGradient>
        </defs>
        <path class="spark-fill" fill="url(#chero-area)" d={spark.fill} />
        <polyline class="spark-line" points={spark.line} />
      </svg>
    {/if}
  </div>
</div>

<style>
  .num {
    font-variant-numeric: tabular-nums;
  }
  .star {
    color: var(--app-accent);
    line-height: 1;
  }

  /* The selected conclusion's own plate — the one place a number on this page
     is display-size. */
  .chero {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 168px;
    gap: var(--s-16);
    padding: 14px var(--s-16);
    margin-bottom: 14px;
  }
  .chero__l {
    min-width: 0;
  }
  .chero__chips {
    display: flex;
    gap: var(--s-6);
    margin-bottom: var(--s-8);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--r-pill);
    background: var(--glass-tint);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
  }
  .chip--on {
    background: var(--app-accent);
    box-shadow: none;
    color: var(--app-accent-contrast);
  }
  .chero__stmt {
    margin: 0 0 var(--s-8);
    max-width: 62ch;
    font: var(--w-medium) var(--t-read) / 1.45 var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text-strong);
  }
  .chero__actions {
    display: flex;
    gap: var(--s-8);
  }
  /* Base `.btn` / `--accent` / `--ghost` come from the shell (system.css §6).
     Only the modifiers this hero invents live here. */
  .btn--busy:disabled {
    opacity: 1;
    cursor: progress;
  }
  /* Dismiss is the one destructive action in the hero, so its ghost hover goes
     danger-red rather than the shared strong-text. */
  .btn--ghost:hover {
    color: var(--app-danger);
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
  .chero__note {
    margin: 9px 0 0;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-subtle);
  }

  .chero__r {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--s-2);
  }
  .chero__big {
    font: var(--w-semi) 34px / 1 var(--app-font-sans);
    letter-spacing: -0.02em;
    color: var(--app-text-strong);
  }
  .chero__big.is-faded {
    color: var(--app-text-subtle);
  }
  .chero__trend {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .chero__trend.up {
    color: var(--app-accent);
  }
  .chero__trend.down {
    color: var(--app-text-faint);
  }
  .chero__cap {
    font: var(--w-medium) var(--t-label) / var(--lh-label) var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .areaspark {
    display: block;
    width: 168px;
    height: 44px;
    margin-top: var(--s-6);
  }
  .spark-fill {
    stroke: none;
  }
  .spark-line {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 1.8;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }

  /* Narrow window: the readout stacks under the statement and goes full width. */
  @media (max-width: 760px) {
    .chero {
      grid-template-columns: 1fr;
    }
    .chero__r {
      align-items: flex-start;
    }
    .areaspark {
      width: 100%;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .btn-spinner {
      animation: none;
    }
  }
</style>
