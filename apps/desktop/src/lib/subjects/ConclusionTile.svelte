<script lang="ts">
  // CONCLUSION — the selected belief, and the two controls that change it.
  // Neither is a delete: Pin exempts the conclusion from decay, Dismiss resets
  // it behind a 2× resurface bar. Both are real writes, run by the shell.
  //
  // The area chart is the same trajectory the strip's card summarises, drawn
  // full-bleed so it clips on the tile radius; it renders only when there are at
  // least two snapshots — a single reading is a dot, not a journey.
  import type { Conclusion, SubjectTrajectory } from "$lib/types/recording";
  import { areaPaths, conf2, pct, spanLabel, sparkY } from "./subjects-format";
  import { DISPLAY_FLOOR } from "$lib/insights/subjectsTiers";

  interface Props {
    conclusion: Conclusion | null;
    trajectory: SubjectTrajectory | undefined;
    actionId: number | null;
    actionKind: "pin" | "dismiss" | null;
    onTogglePin: (id: number, pinned: boolean) => void;
    onDismiss: (id: number) => void;
  }
  let {
    conclusion,
    trajectory,
    actionId,
    actionKind,
    onTogglePin,
    onDismiss,
  }: Props = $props();

  const history = $derived(trajectory?.history ?? []);
  const faded = $derived(conclusion?.status === "faded");

  // The headline word under the figure: the belief's own first→last movement
  // with the same ±0.04 deadband every other surface uses.
  const direction = $derived.by<"rising" | "falling" | "steady">(() => {
    if (history.length < 2) return "steady";
    const d = history[history.length - 1].confidence - history[0].confidence;
    if (d > 0.04) return "rising";
    if (d < -0.04) return "falling";
    return "steady";
  });

  // "12 snapshots · rose 0.44 → 0.78 · over 21 days" — every clause a stored
  // value; a clause with nothing behind it is dropped, never guessed.
  const meta = $derived.by<string | null>(() => {
    if (!conclusion) return null;
    const n = history.length;
    if (n === 0) return null;
    const parts = [`${n} ${n === 1 ? "snapshot" : "snapshots"}`];
    const first = history[0].confidence;
    const last = history[n - 1].confidence;
    if (n < 2 || pct(first) === pct(last)) {
      parts.push(`at ${conf2(last)}`);
    } else {
      parts.push(`${last > first ? "rose" : "fell"} ${conf2(first)} → ${conf2(last)}`);
    }
    const span = spanLabel(history[n - 1].snapshotAtMs - history[0].snapshotAtMs);
    if (span) parts.push(span);
    return parts.join(" · ");
  });

  const W = 300;
  const H = 74;
  const area = $derived(areaPaths(history.map((h) => h.confidence), W, H));

  const busyPin = $derived(actionId === conclusion?.id && actionKind === "pin");
  const busyDismiss = $derived(
    actionId === conclusion?.id && actionKind === "dismiss",
  );
</script>

<div class="tile tile--w2 tile--static">
  <div class="tile__h">
    <span class="t-label">Conclusion</span>
    {#if conclusion}
      {#if conclusion.pinned}
        <span class="chip chip--verdict chip--ok pinchip">pinned</span>
      {/if}
      <span class="chip chip--verdict chip--flat" class:noleft={conclusion.pinned}>
        {faded ? "faded" : "visible"}
      </span>
    {/if}
  </div>

  {#if !conclusion}
    <div class="pay empty"><span class="t-meta">Select a conclusion above.</span></div>
  {:else}
    <div class="hero2">
      <div class="hero2__l">
        <p class="t-read stmt">{conclusion.statement}</p>
        <div class="acts">
          <button
            type="button"
            class="btn btn--sm"
            class:on={conclusion.pinned}
            disabled={actionId !== null}
            onclick={() => onTogglePin(conclusion.id, !conclusion.pinned)}
          >
            <svg class="pinico" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path
                d="M8.5 1.8 12.2 5.5 9.6 8.1 9.2 11 7.4 9.2 3.6 13 1 13 4.8 6.6 3 4.8z"
                stroke="currentColor"
                stroke-width="1.3"
                stroke-linejoin="round"
              />
            </svg>
            {#if busyPin}
              Saving…
            {:else if conclusion.pinned}
              Pinned — protected from decay
            {:else}
              Pin
            {/if}
          </button>
          <button
            type="button"
            class="btn btn--sm btn--ghost"
            disabled={actionId !== null}
            onclick={() => onDismiss(conclusion.id)}
          >
            {busyDismiss ? "Dismissing…" : "Dismiss"}
          </button>
        </div>
      </div>
      <div class="hero2__n">
        <b>{pct(conclusion.confidence)}%</b>
        <i class="dir dir--{direction}">{direction}</i>
        <u>confidence</u>
      </div>
    </div>

    {#if meta}<p class="meta">{meta}</p>{/if}

    {#if area}
      <div class="pay pay--bleed chart">
        <svg class="area" viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
          <line
            x1="0"
            y1={sparkY(DISPLAY_FLOOR, H)}
            x2={W}
            y2={sparkY(DISPLAY_FLOOR, H)}
          />
          <path class="f" d={area.fill} />
          <path class="l" d={area.line} />
        </svg>
      </div>
    {/if}
  {/if}
</div>

<style>
  .pinchip {
    margin-left: auto;
  }
  .chip--flat:not(.noleft) {
    margin-left: auto;
  }

  .hero2 {
    display: flex;
    align-items: flex-start;
    gap: var(--s-16);
  }
  .hero2__l {
    flex: 1 1 auto;
    min-width: 0;
  }
  .stmt {
    margin: 0;
  }
  .acts {
    display: flex;
    gap: var(--s-6);
    margin-top: var(--s-12);
    flex-wrap: wrap;
  }
  .btn.on {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }
  .pinico {
    flex: 0 0 auto;
    width: 12px;
    height: 12px;
  }

  .hero2__n {
    flex: 0 0 auto;
    text-align: right;
  }
  .hero2__n b {
    display: block;
    font: var(--w-semi) 30px / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
    color: var(--app-text-strong);
  }
  .hero2__n i {
    display: block;
    margin-top: 3px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-style: normal;
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
  }
  .dir--rising {
    color: var(--app-accent);
  }
  .dir--falling {
    color: var(--app-danger);
  }
  .dir--steady {
    color: var(--app-text-faint);
  }
  .hero2__n u {
    display: block;
    margin-top: 2px;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
    text-decoration: none;
  }

  .meta {
    margin: var(--s-12) 0 0;
    font: var(--w-regular) var(--t-label) / 1.3 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }

  .chart {
    margin-top: var(--s-8);
    min-height: 48px;
  }
  .area {
    display: block;
    width: 100%;
    height: 100%;
  }
  .area line {
    stroke: var(--app-border-hover);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    vector-effect: non-scaling-stroke;
  }
  .area path.f {
    fill: var(--app-accent);
    opacity: 0.16;
  }
  .area path.l {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 1.6;
    vector-effect: non-scaling-stroke;
  }

  .empty {
    display: flex;
    align-items: center;
  }
</style>
