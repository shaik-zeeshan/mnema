<script lang="ts">
  // ConclusionTimeline — the selected Conclusion's detail: a hero card
  // (ConclusionHero) above a story-framed UNIFIED timeline. The timeline threads
  // an SVG confidence trajectory through content-sized rows: higher confidence
  // sits further right, one node per event, evidence/markers/origin interleaved
  // newest-first.
  //
  // Faithful to docs/user-context/mockups/subject-redesign/unified-timeline.html
  // — the mockup's CSS was authored against the real app tokens, so it ports
  // near-verbatim. Node X comes from `confidenceToX(confidenceAt)` (pure +
  // tested in subjectTimeline.ts); node Y and the track height are MEASURED from
  // the real DOM after render (rows size to their content — no assumed heights),
  // kept fresh by a ResizeObserver on the track.
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
      r: nodeR(ev),
    }));
    const n = nodes.length;
    const firstY = n ? nodes[0].y : 0;
    const lastY = n ? nodes[n - 1].y : 0;
    return {
      // Node-driven; fall back to the measured track only before first measure.
      height: lastY > 0 ? lastY + 8 : trackHeight,
      axisY1: n ? Math.max(0, firstY - 6) : 8,
      axisY2: n ? lastY : Math.max(8, trackHeight - 8),
      points: nodes.map((p) => `${p.x},${p.y}`).join(" "),
      nodes,
    };
  });

  function nodeClass(ev: TimelineEvent): string {
    if (ev.kind === "formed") return "origin";
    if (
      ev.kind === "contradict" ||
      (ev.kind === "marker" && ev.direction === "decayed")
    ) {
      return "bad";
    }
    return isFaded ? "dim" : "";
  }
  function nodeR(ev: TimelineEvent): number {
    if (ev.kind === "formed") return 5;
    if (ev.kind === "marker") return 3.6;
    return 4.5;
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
      hour12: false,
    });
  }
</script>

<section class="conclusion-timeline">
  <!-- ============================== HERO CARD ============================== -->
  <ConclusionHero
    {conclusion}
    {trajectory}
    {actionId}
    {actionKind}
    {onTogglePin}
    {onDismiss}
  />

  <!-- ============================== STORY FRAMING ============================== -->
  <div class="story-head"><span class="story-title">The story over time</span></div>
  <p class="story-sub">
    Most recent at top. The green line is this belief's confidence journey —
    evidence events feed it, quiet stretches let it decay.
  </p>
  <div class="story-legend">
    <span class="li"><span class="lg-line"></span> confidence trajectory</span>
    <span class="li"><span class="lg-node"></span> evidence event</span>
    <span class="li"><span class="lg-mk">↑</span> reinforced</span>
    <span class="li"><span class="lg-mk down">↓</span> decayed / contradicted</span>
  </div>

  <!-- ============================== TIMELINE ============================== -->
  <!-- Rows size to their content; the absolutely-positioned SVG trajectory is
       driven by measured anchor Ys (see `measure()`) so every node lands on its
       card's vertical centre at its confidence x — nothing gets clipped. -->
  <div class="tl-track" bind:this={track}>
    <svg
      class="tl-spine-svg"
      height={geom.height}
      viewBox="0 0 72 {geom.height}"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <line class="axis" x1="36" y1={geom.axisY1} x2="36" y2={geom.axisY2} />
      <polyline class="glow" points={geom.points} />
      <polyline class="line" points={geom.points} />
      {#each geom.nodes as n, i (i)}
        <circle class="node {n.cls}" cx={n.x} cy={n.y} r={n.r} />
      {/each}
    </svg>

    <!-- Positional key: TimelineEvent has no unique id (activityId repeats
         across support/contradict rows), so `i` mirrors SubjectDetail's list. -->
    {#each events as ev, i (i)}
      <div class="tl-row">
        {#if ev.kind === "evidence" || ev.kind === "contradict"}
          {@const isContra = ev.kind === "contradict"}
          {@const sourceType = ev.kind === "evidence" ? ev.sourceType : null}
          {@const frameId = ev.kind === "evidence" ? ev.frameId : null}
          {@const category = ev.kind === "evidence" ? ev.category : null}
          {@const thumbUrl =
            sourceType === "screen" && frameId != null
              ? (thumbnails.get(frameId) ?? null)
              : null}
          <div class="tl-time">
            <span class="rel">{relativeTime(ev.atMs)}</span>
            {#if clockTime(ev.atMs)}
              <span class="clock">{clockTime(ev.atMs)}</span>
            {/if}
          </div>
          <div class="tl-gutter" bind:this={anchors[i]}></div>
          <div
            class="ev-card"
            class:ev-card--contradict={isContra}
            role="button"
            tabindex="0"
            onclick={() => openEvidence(frameId, ev.activityId)}
            onkeydown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                openEvidence(frameId, ev.activityId);
              }
            }}
          >
            <div
              class="ev-thumb"
              class:is-screen={sourceType !== "audio"}
              class:is-audio={sourceType === "audio"}
            >
              {#if thumbUrl}
                <img class="ev-thumb-img" src={thumbUrl} alt="" />
              {/if}
              <span
                class="ev-src"
                class:scr={sourceType !== "audio"}
                class:mic={sourceType === "audio"}
              >
                {sourceType === "audio" ? "mic" : "scr"}
              </span>
            </div>
            <div class="ev-info">
              {#if !isContra && category}
                <div class="ev-cat">
                  <span class="cat-tag">
                    <span
                      class="cat-dot"
                      style="background:var({catColorVar(category)})"
                    ></span>
                    {catLabel(category)}
                  </span>
                </div>
              {/if}
              <div class="ev-title">{ev.title}</div>
              <div class="ev-foot">
                <span class="stance" class:stance--contradict={isContra}>
                  {isContra ? "contradicts" : "supports"}
                </span>
                <span class="ev-link"
                  >{frameId != null ? "view frame →" : "view in Timeline →"}</span
                >
              </div>
            </div>
          </div>
        {:else if ev.kind === "marker"}
          <div class="tl-time faint">
            <span class="rel">{relativeTime(ev.atMs)}</span>
          </div>
          <div class="tl-gutter" bind:this={anchors[i]}></div>
          <div
            class="mk"
            class:mk--up={ev.direction === "reinforced"}
            class:mk--down={ev.direction === "decayed"}
          >
            <span class="g" aria-hidden="true"
              >{ev.direction === "reinforced" ? "↑" : "↓"}</span
            >
            <span class="txt"
              >confidence <b>{Math.round(ev.from * 100)}→{Math.round(ev.to * 100)}</b>
              · {ev.direction === "reinforced" ? "reinforced" : "decayed"}</span
            >
            <span class="when">{relativeTime(ev.atMs)}</span>
          </div>
        {:else if ev.kind === "replaced"}
          <div class="tl-time faint">
            <span class="rel">{relativeTime(ev.atMs)}</span>
            {#if clockTime(ev.atMs)}
              <span class="clock">{clockTime(ev.atMs)}</span>
            {/if}
          </div>
          <div class="tl-gutter" bind:this={anchors[i]}></div>
          <div class="rp">
            <div class="rp-head">
              <span class="rp-label">Replaced an earlier take</span>
              <span class="when">{relativeTime(ev.atMs)}</span>
            </div>
            <p class="rp-old">{ev.statement}</p>
          </div>
        {:else if ev.kind === "formed"}
          <div class="tl-time faint">
            <span class="rel">{relativeTime(ev.atMs)}</span>
            {#if clockTime(ev.atMs)}
              <span class="clock">{clockTime(ev.atMs)}</span>
            {/if}
          </div>
          <div class="tl-gutter" bind:this={anchors[i]}></div>
          <div class="origin">
            <span class="origin-badge">✦ formed</span>
            <span class="origin-title">Conclusion first formed</span>
            <span class="origin-text">Started at <b>{pct(ev.confidence)}%</b>.</span>
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
  /* ============================== STORY FRAMING ============================== */
  .story-head {
    margin: 0 0 4px;
  }
  .story-title {
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .story-sub {
    font-size: var(--text-sm);
    line-height: 1.45;
    color: var(--app-text-muted);
    margin: 0 0 14px;
  }
  .story-legend {
    display: flex;
    gap: 16px;
    margin: 0 0 14px;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    flex-wrap: wrap;
  }
  .story-legend .li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .lg-line {
    width: 16px;
    height: 0;
    border-top: 2px solid var(--app-accent);
  }
  .lg-node {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-accent);
    border: 1.5px solid var(--app-surface);
    box-shadow: 0 0 0 1px var(--app-accent-border);
  }
  .lg-mk {
    color: var(--app-accent-strong);
  }
  .lg-mk.down {
    color: var(--app-danger);
  }

  /* ============================== TIMELINE ============================== */
  .tl-track {
    position: relative;
  }
  .tl-spine-svg {
    position: absolute;
    top: 0;
    left: 92px;
    width: 72px;
    pointer-events: none;
  }
  .tl-spine-svg .glow {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 8;
    opacity: 0.1;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .tl-spine-svg .line {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 2.25;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .tl-spine-svg .axis {
    stroke: var(--app-border-strong);
    stroke-width: 1;
    stroke-dasharray: 2 4;
  }
  .tl-spine-svg .node {
    fill: var(--app-accent);
    stroke: var(--app-surface);
    stroke-width: 2;
  }
  .tl-spine-svg .node.dim {
    fill: var(--app-text-faint);
  }
  .tl-spine-svg .node.bad {
    fill: var(--app-danger);
  }
  .tl-spine-svg .node.origin {
    fill: var(--app-surface);
    stroke: var(--app-accent);
    stroke-width: 2.25;
  }

  /* Rows size to their content (no fixed height / clip) so cards breathe and the
     measured spine geometry always spans the full track. align-items:center
     keeps the zero-size .tl-gutter anchor on each card's vertical centre. */
  .tl-row {
    display: grid;
    grid-template-columns: 92px 72px 1fr;
    align-items: center;
  }
  /* Zero-size spine anchor: measured for each node's Y (see measure()). */
  .tl-gutter {
    width: 0;
    height: 0;
    justify-self: center;
  }
  .tl-time {
    text-align: right;
    padding-right: 16px;
    align-self: center;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tl-time .rel {
    font-size: var(--text-sm);
    color: var(--app-text);
    font-variant-numeric: tabular-nums;
  }
  .tl-time .clock {
    font-size: var(--text-xs);
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }
  .tl-time.faint .rel {
    color: var(--app-text-subtle);
  }

  /* ---- evidence / contradiction card ---- */
  .ev-card {
    display: grid;
    grid-template-columns: 52px 1fr;
    gap: 12px;
    align-items: start;
    padding: 11px 12px;
    margin: 7px 0;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      background 0.12s ease,
      box-shadow 0.12s ease;
  }
  .ev-card:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-subtle);
    box-shadow: 0 1px 0 var(--app-border);
  }
  .ev-card:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .ev-card--contradict {
    border-color: var(--app-danger-border);
  }
  .ev-card--contradict:hover {
    border-color: var(--app-danger);
  }
  .ev-thumb {
    width: 52px;
    height: 40px;
    border-radius: 5px;
    border: 1px solid var(--app-border);
    overflow: hidden;
    position: relative;
    flex: 0 0 auto;
  }
  .ev-thumb.is-screen {
    background: var(--app-source-screen-bg);
  }
  .ev-thumb.is-audio {
    background: var(--app-source-mic-bg);
  }
  .ev-thumb-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .ev-src {
    position: absolute;
    left: 3px;
    bottom: 3px;
    font-size: 7px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    padding: 0 3px;
    border-radius: 3px;
    background: var(--app-bg);
  }
  .ev-src.scr {
    color: var(--app-source-screen);
  }
  .ev-src.mic {
    color: var(--app-source-mic);
  }
  .ev-info {
    min-width: 0;
  }
  .ev-cat {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    margin-bottom: 5px;
  }
  .cat-tag {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .cat-dot {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
  /* Evidence title clamped to ONE line to keep cards scannable and compact. */
  .ev-title {
    font-size: var(--text-base);
    line-height: 1.4;
    color: var(--app-text-strong);
    margin-bottom: 7px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ev-foot {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .stance {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    padding: 1px 7px;
    border-radius: 4px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .stance--contradict {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
    color: var(--app-danger);
  }
  .ev-link {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--app-text-muted);
    border-bottom: 1px dotted var(--app-border-strong);
  }
  .ev-card:hover .ev-link {
    color: var(--app-accent-strong);
    border-bottom-color: var(--app-accent-border);
  }

  /* ---- confidence-change marker (compact pill, NO causal claim) ---- */
  .mk {
    display: inline-flex;
    align-items: center;
    gap: 9px;
    margin: 4px 0;
    padding: 4px 11px 4px 9px;
    border-radius: 999px;
    align-self: center;
    width: fit-content;
    font-size: var(--text-sm);
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .mk .g {
    font-size: var(--text-base);
  }
  .mk b {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .mk .txt {
    color: var(--app-text-muted);
  }
  .mk .when {
    color: var(--app-text-faint);
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    padding-left: 2px;
  }
  .mk--up .txt b {
    color: var(--app-accent-strong);
  }
  .mk--down {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
    color: var(--app-danger);
  }
  .mk--down .txt b {
    color: var(--app-danger);
  }

  /* ---- replaced-an-earlier-take (ADR 0046) ---- */
  .rp {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-self: center;
    width: fit-content;
    max-width: 100%;
    margin: 4px 0;
    padding: 7px 11px;
    border: 1px dashed var(--app-border-strong);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .rp-head {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .rp-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .rp .when {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--app-text-faint);
  }
  .rp-old {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ---- origin (bottom) ---- */
  .origin {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin: 6px 0;
    padding: 12px;
    border: 1px dashed var(--app-border-strong);
    border-radius: 7px;
    background: var(--app-surface-subtle);
    align-self: center;
  }
  .origin-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    width: fit-content;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .origin-title {
    font-size: var(--text-base);
    color: var(--app-text-strong);
    font-weight: 600;
  }
  .origin-text {
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    line-height: 1.5;
  }
  .origin-text b {
    color: var(--app-text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  @media (prefers-reduced-motion: reduce) {
    .ev-card {
      transition: none;
    }
  }
</style>
