<script lang="ts">
  // ConclusionTimeline — the selected Conclusion's detail (page 09): the
  // pinned-belief hero (ConclusionHero) above "The story over time", the
  // trajectory track.
  //
  // THE SPINE IS NOT DECORATION: a node's horizontal position IS the belief's
  // confidence at that moment, so the line leaning right is literally the
  // belief getting firmer. Five event kinds thread onto it, newest at the top,
  // `formed` always last: a confidence step, an evidence event, a replaced
  // earlier take, a contradiction, and the origin. Every card is an opaque
  // plate; the spine is drawn on the pane behind them, one layer down.
  //
  // Node X comes from `confidenceToX(confidenceAt)` (pure + tested in
  // subjectTimeline.ts); node Y and the track height are MEASURED from the real
  // DOM after render (rows size to their content — no assumed heights), kept
  // fresh by a ResizeObserver on the track.
  //
  // The shell owns everything else: it builds & orders `events` (newest-first,
  // `formed` last), lazy-loads `thumbnails`, and runs the pin/dismiss commands
  // (incl. any confirmation dialogs). This component only renders + calls back.

  import { untrack } from "svelte";
  import type {
    ActivityCategory,
    Conclusion,
    SubjectTrajectory,
  } from "$lib/types/recording";
  import {
    confidenceToX,
    type TimelineEvent,
  } from "$lib/insights/subjectTimeline";
  import { CATEGORY_COLOR, categoryLabel } from "$lib/insights/activity-helpers";
  import { invoke } from "@tauri-apps/api/core";
  import ConclusionHero from "$lib/insights/ConclusionHero.svelte";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";

  interface Props {
    events: TimelineEvent[];
    conclusion: Conclusion;
    trajectory: SubjectTrajectory | undefined;
    thumbnails: Map<number, string>;
    actionId?: number | null;
    actionKind?: "pin" | "dismiss" | null;
    onTogglePin: (id: number, pinned: boolean) => void;
    onDismiss: (id: number) => void;
    onViewInTimeline: (activityId: number) => void;
  }

  let {
    events,
    conclusion,
    trajectory,
    thumbnails,
    actionId = null,
    actionKind = null,
    onTogglePin,
    onDismiss,
    onViewInTimeline,
  }: Props = $props();

  const isFaded = $derived(conclusion.status === "faded");

  // In-place frame peek (FrameDetailModal). An evidence row that carries a frame
  // opens the modal instead of hopping to the raw Timeline window; the old
  // hand-off (the `onViewInTimeline` prop) survives only as the modal's escape
  // hatch and as the fallback for rows with no frame (audio evidence, contradict
  // rows whose frame the parent resolves).
  let frameModalOpen = $state(false);
  let frameModalId = $state<number | null>(null);
  let frameModalOpenInTimeline = $state<(() => void) | null>(null);

  function openEvidence(frameId: number | null, activityId: number): void {
    if (frameId != null) {
      const fid = frameId;
      frameModalId = fid;
      // Escape hatch = hand THIS frame to the raw Timeline directly. It must NOT
      // route back through `onViewInTimeline`, which re-branches a frame ref into
      // the parent's own peek modal — that reopened a modal instead of navigating
      // (the "flicker + needs a second click" bug).
      frameModalOpenInTimeline = () => void openFrameInTimeline(fid, activityId);
      frameModalOpen = true;
      return;
    }
    onViewInTimeline(activityId);
  }

  // Raw-Timeline hand-off for a specific frame (the modal's escape hatch). On
  // failure, fall back to the parent's activity-span navigation.
  async function openFrameInTimeline(frameId: number, activityId: number): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId,
        audioSegmentId: null,
      });
    } catch {
      onViewInTimeline(activityId);
    }
  }

  function pct(confidence: number): number {
    return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
  }

  // Spine geometry: X is pure (confidence), Y + height are MEASURED. The gutter
  // cell of each row is a zero-size anchor centred vertically on its card; after
  // render we measure each anchor's centre Y relative to `.tl-track` and drive
  // the absolutely-positioned SVG from those. A ResizeObserver on the track
  // re-measures on reflow (thumbnail load, window resize, theme change); the
  // effect also re-runs when `events` changes.
  let track = $state<HTMLElement>();
  let anchors = $state<(HTMLElement | undefined)[]>([]);
  let centers = $state<number[]>([]);
  let trackHeight = $state(0);

  function eq(a: number[], b: number[]): boolean {
    return a.length === b.length && a.every((v, i) => v === b[i]);
  }

  function measure() {
    const el = track;
    if (!el) return;
    // untrack: the reads below (state + DOM) must not become effect deps, or the
    // measure→setState→measure loop thrashes. Writes still update state.
    untrack(() => {
      const trackTop = el.getBoundingClientRect().top;
      const next = anchors.map((a) => {
        if (!a) return 0;
        const r = a.getBoundingClientRect();
        return r.top - trackTop + r.height / 2;
      });
      const h = el.scrollHeight;
      if (h !== trackHeight) trackHeight = h;
      if (!eq(next, centers)) centers = next;
    });
  }

  $effect(() => {
    events; // re-measure when the event stream changes
    const el = track;
    if (!el) return;
    measure();
    const ro = new ResizeObserver(() => measure());
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Node X from confidence, Y from measured centre. Index-aligned with `events`.
  // All vertical extents (SVG height + dashed axis) are derived from the NODES,
  // never from the container height: the axis spans the first node → the last
  // (origin) node, and the SVG is no taller than the last node. So even if a
  // parent stretches `.tl-track` past its content, nothing can draw into the
  // empty space below the "formed" card. Guards cover the 0/1-node cases.
  const geom = $derived.by(() => {
    const nodes = events.map((ev, i) => ({
      x: confidenceToX(ev.confidenceAt),
      y: centers[i] ?? 0,
      cls: nodeClass(ev),
      r: 4,
    }));
    const n = nodes.length;
    const lastY = n ? nodes[n - 1].y : 0;
    return {
      // Node-driven; fall back to the measured track only before first measure.
      height: lastY > 0 ? lastY + 8 : trackHeight,
      points: nodes.map((p) => `${p.x},${p.y}`).join(" "),
      nodes,
    };
  });

  // Node vocabulary (09's spine): the trajectory's OWN points — a confidence
  // step and the origin — are filled accent; everything else is a hollow node
  // on the line, and a cooling/contradicting one loses its grey for faint.
  function nodeClass(ev: TimelineEvent): string {
    if (ev.kind === "formed") return "origin";
    if (ev.kind === "marker") {
      return ev.direction === "reinforced" ? "step" : "down";
    }
    if (ev.kind === "contradict") return "down";
    return isFaded ? "down" : "";
  }

  function catColorVar(category: string | null): string {
    if (!category) return "--app-text-muted";
    return CATEGORY_COLOR[category as ActivityCategory] ?? "--app-text-muted";
  }
  function catLabel(category: string | null): string {
    if (!category) return "";
    return categoryLabel(category as ActivityCategory);
  }

  // Relative timestamp — same shape as SubjectDetail.svelte `relativeTime`.
  function relativeTime(ms: number | null): string {
    if (ms === null || !Number.isFinite(ms) || ms <= 0) return "—";
    const diff = Date.now() - ms;
    if (diff < 0) return "just now";
    const min = Math.floor(diff / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    const day = Math.floor(hr / 24);
    if (day < 7) return `${day}d ago`;
    const wk = Math.floor(day / 7);
    if (wk < 5) return `${wk}w ago`;
    const mo = Math.floor(day / 30);
    if (mo < 12) return `${mo}mo ago`;
    const yr = Math.floor(day / 365);
    return `${yr}y ago`;
  }
  // Wall-clock HH:MM for the timeline's second time line.
  function clockTime(ms: number | null): string {
    if (ms === null || !Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      hour12: true,
    });
  }
  /** The card's one timestamp: "4h ago · 1:46 PM" (clock dropped when absent). */
  function when(ms: number | null): string {
    const clock = clockTime(ms);
    return clock ? `${relativeTime(ms)} · ${clock}` : relativeTime(ms);
  }
</script>

<section class="conclusion-timeline">
  <!-- ============================== THE HERO ============================== -->
  <ConclusionHero
    {conclusion}
    {trajectory}
    {actionId}
    {actionKind}
    {onTogglePin}
    {onDismiss}
  />

  <!-- ============================ STORY FRAMING ============================ -->
  <div class="story">
    <span class="story__t">The story over time</span>
    <span class="story__s"
      >Most recent at top. The line is this belief's confidence journey —
      evidence events feed it, quiet stretches let it decay.</span
    >
  </div>
  <div class="legend">
    <span><i class="lg-traj"></i>confidence trajectory</span>
    <span><i class="lg-ev"></i>evidence event</span>
    <span><i class="lg-up"></i>↑ reinforced</span>
    <span><i class="lg-dn"></i>↓ decayed / contradicted</span>
  </div>

  <!-- ========================= THE TRAJECTORY TRACK =========================
       The spine is NOT decoration: each node's horizontal position IS the
       belief's confidence at that moment, so the line leaning right is
       literally the belief getting firmer. Every card is an opaque plate; the
       spine is drawn on the pane BEHIND them, one layer down. Node X is pure
       (`confidenceToX`); node Y is MEASURED from the real DOM, because rows
       size to their content and no row height may be assumed. -->
  <div class="tline" bind:this={track}>
    <svg
      class="spine"
      height={geom.height}
      viewBox="0 0 72 {geom.height}"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <polyline class="spine__line" points={geom.points} />
      {#each geom.nodes as n, i (i)}
        <circle class="spine__node {n.cls}" cx={n.x} cy={n.y} r={n.r} />
      {/each}
    </svg>

    <!-- Positional key: TimelineEvent has no unique id (activityId repeats
         across support/contradict rows), so `i` mirrors the built order. -->
    {#each events as ev, i (i)}
      <div class="tev">
        <span class="tev__t is-num">{pct(ev.confidenceAt)}%</span>
        <span class="tev__a" bind:this={anchors[i]}></span>

        {#if ev.kind === "evidence" || ev.kind === "contradict"}
          {@const isContra = ev.kind === "contradict"}
          {@const thumbUrl =
            ev.sourceType === "screen" && ev.frameId != null
              ? (thumbnails.get(ev.frameId) ?? null)
              : null}
          <button
            type="button"
            class="plate tev__c tev__c--act"
            onclick={() => openEvidence(ev.frameId, ev.activityId)}
          >
            <span class="tev__h">
              {#if ev.sourceType}
                <span class="badge">{ev.sourceType === "audio" ? "mic" : "scr"}</span>
              {/if}
              {#if ev.category}
                <span class="cchip">
                  <em style="background:var({catColorVar(ev.category)})"></em>
                  {catLabel(ev.category)}
                </span>
              {/if}
              <span class="tev__when is-num">{when(ev.atMs)}</span>
            </span>
            <span class="tev__b">
              {#if ev.sourceType}
                <span
                  class="th"
                  class:th--audio={ev.sourceType === "audio"}
                  aria-hidden="true"
                >
                  {#if thumbUrl}<img src={thumbUrl} alt="" />{/if}
                </span>
              {/if}
              <span class="tev__title">{ev.title}</span>
            </span>
            <span class="tev__f">
              <span class="stance" class:stance--contra={isContra}>
                {isContra ? "contradicts" : "supports"}
              </span>
              <span class="tev__link"
                >{ev.frameId != null ? "view frame ›" : "view in Timeline ›"}</span
              >
            </span>
          </button>
        {:else if ev.kind === "marker"}
          {@const up = ev.direction === "reinforced"}
          <div class="plate tev__c">
            <div class="tev__h">
              <span class="trend" class:tr-up={up} class:tr-dn={!up}
                >{up ? "↑" : "↓"}</span
              >
              <span class="tev__lead is-num"
                >confidence {pct(ev.from)}% → {pct(ev.to)}%</span
              >
              <span class="tev__chip">{up ? "reinforced" : "decayed"}</span>
              <span class="tev__when is-num">{when(ev.atMs)}</span>
            </div>
            <p class="tev__p">
              {up
                ? "A fresh supporting activity re-formed the belief and snapshotted the up-step."
                : "No fresh evidence in this stretch, so the belief cooled on its own."}
            </p>
          </div>
        {:else if ev.kind === "replaced"}
          <!-- ADR 0046 audit event: a belief is superseded, never revised. -->
          <div class="plate tev__c">
            <div class="tev__h">
              <svg
                class="tev__glyph"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M9 14.5 3.8 9.4 9 4.3" />
                <path d="M3.8 9.4h11.4a5.3 5.3 0 1 1 0 10.6h-4.4" />
              </svg>
              <span class="tev__lead">Replaced an earlier take</span>
              <span class="tev__when is-num">{when(ev.atMs)}</span>
            </div>
            <p class="tev__p">“{ev.statement}” — retired, not revised.</p>
          </div>
        {:else if ev.kind === "formed"}
          <div class="plate tev__c">
            <div class="tev__h">
              <span class="formed" aria-hidden="true">✦</span>
              <span class="formed__l">formed</span>
              <span class="tev__when is-num">{when(ev.atMs)}</span>
            </div>
            <div class="tev__b"><span class="tev__title">Conclusion first formed</span></div>
            <p class="tev__p is-num">Started at {pct(ev.confidence)}%.</p>
          </div>
        {/if}
      </div>
    {/each}
  </div>
</section>

<!-- In-place frame peek for an evidence row. Its "open full timeline →" escape
     hatch replays the parent's raw-Timeline hand-off (onViewInTimeline). -->
<FrameDetailModal
  open={frameModalOpen}
  frameId={frameModalId}
  onClose={() => (frameModalOpen = false)}
  onOpenInTimeline={frameModalOpenInTimeline ?? undefined}
/>

<style>
  .is-num {
    font-variant-numeric: tabular-nums;
  }

  /* ============================ STORY FRAMING ============================ */
  .story {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--s-12);
    margin: var(--s-4) 0 var(--s-8);
    padding: 0 var(--s-4);
  }
  .story__t {
    font: var(--w-semi) var(--t-title) / var(--lh-title) var(--app-font-sans);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }
  .story__s {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-12);
    padding: 0 var(--s-4);
    margin-bottom: var(--s-12);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: var(--s-6);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .legend i {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    display: block;
  }
  .lg-traj {
    background: var(--app-accent);
  }
  .lg-ev {
    background: var(--chart-grey-3);
  }
  .lg-up {
    background: var(--app-accent);
    opacity: 0.6;
  }
  .lg-dn {
    background: var(--app-text-faint);
  }

  /* ========================= THE TRAJECTORY TRACK ========================= */
  .tline {
    position: relative;
  }
  /* The spine lives on the PANE, behind every plate — one layer down. */
  .spine {
    position: absolute;
    left: 34px;
    top: 0;
    width: 72px;
    pointer-events: none;
  }
  .spine__line {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 1.6;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .spine__node {
    fill: var(--app-surface);
    stroke: var(--chart-grey-3);
    stroke-width: 2;
  }
  /* A confidence step and the origin are the trajectory's OWN points, so they
     are filled accent; evidence and audit events are hollow. */
  .spine__node.step,
  .spine__node.origin {
    fill: var(--app-accent);
    stroke: none;
  }
  .spine__node.down {
    stroke: var(--app-text-faint);
  }

  /* One row: [34px confidence label][72px spine gutter][the card]. The label
     and the node share the row's centre line, so the number reads as the
     node's value — the y-axis tick for that moment. */
  .tev {
    display: grid;
    grid-template-columns: 34px 72px minmax(0, 1fr);
    align-items: center;
    padding-bottom: 10px;
  }
  .tev__t {
    text-align: right;
    padding-right: var(--s-6);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    color: var(--app-text-faint);
  }
  /* Zero-size spine anchor — measured for each node's Y (see measure()). */
  .tev__a {
    width: 0;
    height: 0;
    justify-self: center;
  }

  /* Every event is an opaque plate. */
  .tev__c {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 9px 12px 10px;
    min-width: 0;
    text-align: left;
  }
  .tev__c--act {
    border: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition: box-shadow var(--dur-quick) var(--ease);
  }
  .tev__c--act:hover {
    box-shadow: var(--sh-tile), inset 0 0 0 var(--hairline) var(--app-border-hover);
  }
  .tev__c--act:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  .tev__h {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    min-width: 0;
  }
  .tev__lead {
    font: var(--w-medium) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .tev__when {
    margin-left: auto;
    padding-left: var(--s-8);
    white-space: nowrap;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .tev__glyph {
    width: 12px;
    height: 12px;
    flex: 0 0 auto;
    color: var(--app-text-subtle);
  }
  .tev__p {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .tev__b {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .tev__title {
    font: var(--w-medium) var(--t-ui) / 1.35 var(--app-font-sans);
    color: var(--app-text-strong);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tev__f {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }
  .tev__link {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .tev__c--act:hover .tev__link {
    color: var(--app-accent);
  }

  /* The frame itself — a small opaque thumbnail, never a decoration. */
  .th {
    width: 56px;
    height: 34px;
    border-radius: 5px;
    overflow: hidden;
    flex: 0 0 auto;
    position: relative;
    background: var(--app-source-screen-bg);
  }
  .th--audio {
    background: var(--app-source-mic-bg);
  }
  .th img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  /* The machine's own labels: mono, uppercase, on the material's tint. */
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 16px;
    flex: 0 0 auto;
    border-radius: 3px;
    background: var(--glass-tint);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
    color: var(--app-text-muted);
    font: var(--w-medium) 8px / 1 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .cchip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text);
  }
  .cchip em {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: block;
    flex: 0 0 auto;
  }
  .stance {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-accent);
  }
  .stance--contra {
    color: var(--app-danger);
  }
  .trend {
    width: 12px;
    flex: 0 0 auto;
    text-align: center;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
  }
  .tr-up {
    color: var(--app-accent);
  }
  .tr-dn {
    color: var(--app-text-faint);
  }
  .tev__chip {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .formed {
    color: var(--app-accent);
    line-height: 1;
  }
  .formed__l {
    font: var(--w-medium) var(--t-label) / var(--lh-label) var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-accent);
  }

  @media (prefers-reduced-motion: reduce) {
    .tev__c--act {
      transition: none;
    }
  }
</style>
