// subjectTimeline.ts — pure timeline/chart helpers for the Subject detail. No Svelte, no I/O, `import type` only — so it is
// unit-testable under `bun test`, exactly like onboarding-privacy-sync.ts.
//
// buildTimeline merges a conclusion's evidence with its confidence-trajectory
// markers into one chronological (newest-first) stream, with the "formed"
// origin pinned to the bottom. chartPoints maps a history onto the chart.
import type {
  Activity,
  Conclusion,
  ConclusionEvidenceRef,
  ConfidenceSnapshot,
  EvidenceStance,
  SubjectTrajectory,
} from "$lib/types/recording";

// Below this absolute per-step confidence delta, consecutive same-direction
// steps collapse into ONE marker so a warming run renders as one arrow, not 8.
const MICRO_STEP = 0.04;

// Every event carries `confidenceAt` (0..1): where it sits on the confidence
// trajectory line, so the SVG spine can plot one node per row. Computed per
// kind at build time — never read raw off the wire.
export type TimelineEvent = (
  | {
      kind: "evidence";
      atMs: number | null;
      activityId: number;
      stance: EvidenceStance;
      title: string;
      category: string | null;
      sourceType: "screen" | "audio" | null;
      frameId: number | null;
    }
  | {
      kind: "marker";
      atMs: number;
      direction: "reinforced" | "decayed";
      from: number; // 0..1 confidence at the run's start
      to: number; // 0..1 confidence at the run's end
    }
  | { kind: "contradict"; atMs: number | null; activityId: number; title: string }
  // ADR 0046: this belief replaced a wrong earlier one; `statement` is the
  // retired take, `atMs` its retirement time. Audit event, no causal claim.
  | { kind: "replaced"; atMs: number; statement: string }
  | { kind: "formed"; atMs: number; confidence: number }
) & { confidenceAt: number };

/** Linear-interpolate the confidence trajectory at `atMs`, clamped to the
 *  first/last snapshot. Empty history or a null timestamp falls back to
 *  `fallback` (the conclusion's current confidence). Pure. */
export function interpolateConfidence(
  history: readonly ConfidenceSnapshot[],
  atMs: number | null,
  fallback: number,
): number {
  if (atMs === null || history.length === 0) return fallback;
  if (atMs <= history[0].snapshotAtMs) return history[0].confidence;
  const last = history[history.length - 1];
  if (atMs >= last.snapshotAtMs) return last.confidence;
  for (let i = 1; i < history.length; i++) {
    const a = history[i - 1];
    const b = history[i];
    if (atMs <= b.snapshotAtMs) {
      const span = b.snapshotAtMs - a.snapshotAtMs;
      if (span <= 0) return b.confidence;
      const t = (atMs - a.snapshotAtMs) / span;
      return a.confidence + t * (b.confidence - a.confidence);
    }
  }
  return last.confidence;
}

/** Merge a conclusion's evidence + trajectory markers into one newest-first
 *  stream. The `formed` origin is always last; null-timestamp events sort as
 *  oldest (just above `formed`). Ties preserve input order (stable). Markers
 *  and evidence are interleaved by timestamp only — no fabricated causal link
 *  between a marker and any specific evidence item. */
export function buildTimeline(
  conclusion: Conclusion,
  trajectory: SubjectTrajectory | undefined,
  activities: ReadonlyMap<number, Activity>,
): TimelineEvent[] {
  const events: TimelineEvent[] = [];
  const history = trajectory?.history ?? [];
  const fallback = conclusion.confidence;

  // Evidence — mirror SubjectDetail.svelte's EvidenceRow join: resolve the
  // Activity for a richer title/time/category and a source-type hint from the
  // Activity's first raw evidence ref. A `contradict` ref becomes its own event.
  for (const ref of conclusion.evidence) {
    const activity = activities.get(ref.activityId);
    const atMs = activity?.startedAtMs ?? ref.activityStartedAtMs ?? null;
    const title =
      activity?.title ?? ref.activityTitle ?? `Activity #${ref.activityId}`;
    // Evidence/contradict sit on the trajectory at their own timestamp.
    const confidenceAt = interpolateConfidence(history, atMs, fallback);
    if (ref.stance === "contradict") {
      events.push({
        kind: "contradict",
        atMs,
        activityId: ref.activityId,
        title,
        confidenceAt,
      });
      continue;
    }
    const firstRef = activity?.evidence?.[0];
    const sourceType: "screen" | "audio" | null = firstRef
      ? firstRef.subjectType === "audio_segment"
        ? "audio"
        : "screen"
      : null;
    const frameId =
      firstRef && firstRef.subjectType === "frame" ? firstRef.subjectId : null;
    events.push({
      kind: "evidence",
      atMs,
      activityId: ref.activityId,
      stance: ref.stance,
      title,
      category: activity?.category ?? null,
      sourceType,
      frameId,
      confidenceAt,
    });
  }

  events.push(...buildMarkers(trajectory));

  // ADR 0046 audit event: this belief replaced a wrong earlier one.
  if (
    conclusion.replacedStatement &&
    conclusion.replacedAtMs != null
  ) {
    events.push({
      kind: "replaced",
      atMs: conclusion.replacedAtMs,
      statement: conclusion.replacedStatement,
      confidenceAt: interpolateConfidence(history, conclusion.replacedAtMs, fallback),
    });
  }

  const formationConfidence = history[0] ? history[0].confidence : fallback;
  events.push({
    kind: "formed",
    atMs: conclusion.formedAtMs,
    confidence: formationConfidence,
    confidenceAt: formationConfidence,
  });

  return events.sort(compareEvents);
}

/** Walk adjacent snapshot pairs into reinforced/decayed markers. A run of
 *  consecutive same-direction sub-MICRO_STEP steps collapses into one marker
 *  spanning the run's start `from` to end `to`; a step >= MICRO_STEP stands
 *  alone. Zero-delta steps and empty/one-point history yield nothing. */
function buildMarkers(trajectory: SubjectTrajectory | undefined): TimelineEvent[] {
  const history = trajectory?.history ?? [];
  const markers: TimelineEvent[] = [];
  // Pending run of collapsed micro-steps (null when no run is open).
  let run:
    | { direction: "reinforced" | "decayed"; from: number; to: number; atMs: number }
    | null = null;

  const flush = () => {
    if (run) {
      // confidenceAt = the run's end confidence (its node on the trajectory).
      markers.push({ kind: "marker", ...run, confidenceAt: run.to });
      run = null;
    }
  };

  for (let i = 1; i < history.length; i++) {
    const from = history[i - 1].confidence;
    const to = history[i].confidence;
    const atMs = history[i].snapshotAtMs;
    const delta = to - from;
    if (delta === 0) {
      flush();
      continue;
    }
    const direction: "reinforced" | "decayed" =
      delta > 0 ? "reinforced" : "decayed";
    if (Math.abs(delta) >= MICRO_STEP) {
      flush();
      markers.push({ kind: "marker", atMs, direction, from, to, confidenceAt: to });
      continue;
    }
    // Micro step: extend a same-direction run, else start a fresh one.
    if (run && run.direction === direction) {
      run.to = to;
      run.atMs = atMs;
    } else {
      flush();
      run = { direction, from, to, atMs };
    }
  }
  flush();
  // Drop markers that are invisible at display precision: after collapsing a
  // run its endpoints can still round to the same whole percent (e.g. a
  // "DECAYED 90 → 90" no-op). Suppress those — they're visual noise.
  return markers.filter(
    (m) =>
      m.kind !== "marker" ||
      Math.round(m.from * 100) !== Math.round(m.to * 100),
  );
}

/** Sort category: 0 = has timestamp (newest first), 1 = null timestamp
 *  (oldest, stable), 2 = the formed origin (always last). */
function sortCategory(e: TimelineEvent): number {
  if (e.kind === "formed") return 2;
  return e.atMs === null ? 1 : 0;
}

function compareEvents(a: TimelineEvent, b: TimelineEvent): number {
  const ca = sortCategory(a);
  const cb = sortCategory(b);
  if (ca !== cb) return ca - cb;
  if (ca === 0) return (b.atMs as number) - (a.atMs as number); // newest first
  return 0; // stable within null / formed buckets
}

// ---- Confidence-over-time chart geometry -----------------------------------

export interface ChartPoint {
  x: number; // 0..1 across the time domain
  y: number; // 0..1 confidence
}

/** Position of `atMs` on a [fromMs, toMs] time axis, clamped to 0..1. Pure. */
export function chartX(atMs: number, fromMs: number, toMs: number): number {
  return Math.max(0, Math.min(1, (atMs - fromMs) / Math.max(1, toMs - fromMs)));
}

/** One conclusion's Confidence History as a polyline on a shared time axis
 *  (`toMs` = now). The line starts at the first retained snapshot and holds
 *  its last value flat to the right edge — confidence only moves when the
 *  engine writes a snapshot. Empty history => no line. Pure. */
export function chartPoints(
  history: readonly ConfidenceSnapshot[],
  fromMs: number,
  toMs: number,
): ChartPoint[] {
  const pts = history.map((s) => ({ x: chartX(s.snapshotAtMs, fromMs, toMs), y: s.confidence }));
  const last = pts[pts.length - 1];
  if (last && last.x < 1) pts.push({ x: 1, y: last.y });
  return pts;
}
