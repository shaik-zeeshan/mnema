// Subjects destination — the data layer, shared by the index and the detail.
//
// Nothing here is new backend: every read is a command the shipping Subjects
// surface (`lib/insights/Subjects.svelte` + `SubjectDetail.svelte`) already
// makes. The tiering / search / timeline maths is imported from the existing
// pure modules (`subjectsTiers.ts`, `subjectSearch.ts`, `subjectTimeline.ts`) —
// their thresholds win over the mockup's constants.
//
// One deliberate difference from the shipping list: trajectories are kept as
// full `ConfidenceSnapshot`s (confidence + snapshotAtMs) rather than bare
// numbers, because page 09 draws the sparkline spaced by TIME. The timestamps
// were always on the wire; the old surface dropped them.
import { invoke } from "@tauri-apps/api/core";
import { goto } from "$app/navigation";
import type {
  Activity,
  ActivityEvidenceRef,
  AiRuntimeStatus,
  Conclusion,
  ConfidenceSnapshot,
  SubjectView,
  UserContextStatus,
} from "$lib/types/recording";
import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
import { framePreviewAssetUrl } from "$lib/frame-preview";
import { deriveTrend, type TierSubject, type Trend } from "$lib/insights/subjectsTiers";

/** One conclusion's confidence line, oldest-first, for the row sparkline. */
export interface SparkSeries {
  points: ConfidenceSnapshot[];
  faded: boolean;
}

/** A subject row. Satisfies `TierSubject`, so `buildTiers` groups it directly. */
export interface SubjectRow extends TierSubject {
  subject: string;
  /** Highest-confidence first. */
  conclusions: Conclusion[];
  conclusionCount: number;
  pinned: boolean;
  headline: string;
  series: SparkSeries[];
}

/** subject → (conclusionId → oldest-first snapshots). */
export type History = Map<string, Map<number, ConfidenceSnapshot[]>>;

// ---- reads ------------------------------------------------------------------

export function fetchConclusions(): Promise<Conclusion[]> {
  return invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: true });
}

export function fetchSubject(subject: string): Promise<SubjectView> {
  return invoke<SubjectView>("get_user_context_subject", { subject });
}

/** Is the Reasoning Engine on? Best-effort — a failed probe reads as off-unknown
 *  (null), which the empty state renders as the neutral "nothing yet" copy. */
export async function fetchEngineOn(): Promise<boolean | null> {
  const [ai, ctx] = await Promise.all([
    invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
    invoke<UserContextStatus>("get_user_context_status").catch(() => null),
  ]);
  if (!ai && !ctx) return null;
  return Boolean(ai?.enabled && ai?.available) || Boolean(ctx?.engineAvailable);
}

/** Fetch every subject's real confidence history, bounded concurrency. A failed
 *  subject simply keeps its flat baseline. */
export async function fetchHistory(subjects: string[]): Promise<History> {
  const out: History = new Map();
  let cursor = 0;
  const worker = async (): Promise<void> => {
    while (cursor < subjects.length) {
      const subject = subjects[cursor++];
      try {
        const view = await fetchSubject(subject);
        const byId = new Map<number, ConfidenceSnapshot[]>();
        for (const t of view.trajectories) byId.set(t.conclusionId, t.history);
        out.set(subject, byId);
      } catch {
        // best-effort
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(4, subjects.length) }, worker));
  return out;
}

/** Resolve the Activities a set of evidence refs cite, via the same bounded
 *  paged scan both shipping surfaces use. Unresolved ids simply yield nothing. */
export async function resolveActivities(
  wanted: Set<number>,
): Promise<Map<number, Activity>> {
  const resolved = new Map<number, Activity>();
  if (wanted.size === 0) return resolved;
  const PAGE = 200;
  const MAX_PAGES = 6; // bounded scan; evidence is recent for live subjects
  for (let page = 0; page < MAX_PAGES; page++) {
    let batch: Activity[];
    try {
      batch = await invoke<Activity[]>("list_user_context_activities", {
        limit: PAGE,
        offset: page * PAGE,
      });
    } catch {
      break;
    }
    if (batch.length === 0) break;
    for (const a of batch) if (wanted.has(a.id)) resolved.set(a.id, a);
    if (resolved.size >= wanted.size) break;
    if (batch.length < PAGE) break;
  }
  return resolved;
}

/** Frame previews for evidence thumbnails. Best-effort; missing ids draw the
 *  empty placeholder box. */
export async function loadFramePreviews(
  frameIds: number[],
): Promise<Map<number, string>> {
  const out = new Map<number, string>();
  if (frameIds.length === 0) return out;
  try {
    const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
      request: { frameIds },
    });
    for (const entry of response.previews) {
      if (entry.preview) out.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
    }
  } catch {
    // best-effort
  }
  return out;
}

// ---- writes -----------------------------------------------------------------

export function setPinned(id: number, pinned: boolean): Promise<void> {
  return invoke("user_context_set_pinned", { id, pinned });
}

export function dismissConclusion(id: number): Promise<void> {
  return invoke("user_context_dismiss_conclusion", { id });
}

// ---- row projection ---------------------------------------------------------

/** Group conclusions into subject rows and sort them the way the whole surface
 *  reads: live subjects by top confidence desc, faded sunk to the bottom. */
export function buildRows(conclusions: Conclusion[], history: History): SubjectRow[] {
  const groups = new Map<string, Conclusion[]>();
  for (const c of conclusions) {
    const bucket = groups.get(c.subject);
    if (bucket) bucket.push(c);
    else groups.set(c.subject, [c]);
  }

  const rows: SubjectRow[] = [];
  for (const [subject, cs] of groups) {
    const byId = history.get(subject);
    const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
    const top = sorted[0];
    // deriveTrend wants bare confidence numbers; the timestamps stay on `series`.
    const numbers = byId
      ? new Map([...byId].map(([id, pts]) => [id, pts.map((p) => p.confidence)]))
      : undefined;
    rows.push({
      subject,
      conclusions: sorted,
      conclusionCount: cs.length,
      pinned: cs.some((c) => c.pinned),
      faded: cs.every((c) => c.status === "faded"),
      headline: top?.statement ?? subject,
      lastMovedAtMs: cs.reduce((a, c) => Math.max(a, c.updatedAtMs, c.lastSupportedAtMs), 0),
      trend: deriveTrend(cs, numbers),
      topConfidence: top?.confidence ?? 0,
      series: sorted.map((c) => ({
        faded: c.status === "faded",
        points: seriesPoints(byId?.get(c.id), c),
      })),
    });
  }

  rows.sort(
    (a, b) =>
      Number(a.faded) - Number(b.faded) ||
      b.topConfidence - a.topConfidence ||
      a.subject.localeCompare(b.subject),
  );
  return rows;
}

/** A drawable line needs two points. One snapshot (or none) flattens into a
 *  baseline spanning formation → last support, so a flat line still draws. */
function seriesPoints(
  history: ConfidenceSnapshot[] | undefined,
  c: Conclusion,
): ConfidenceSnapshot[] {
  if (history && history.length >= 2) return history;
  if (history && history.length === 1) {
    const only = history[0];
    return [only, { confidence: only.confidence, snapshotAtMs: c.lastSupportedAtMs }];
  }
  return [
    { confidence: c.confidence, snapshotAtMs: c.formedAtMs },
    { confidence: c.confidence, snapshotAtMs: c.lastSupportedAtMs },
  ];
}

// ---- evidence ---------------------------------------------------------------

/** Distinct activity ids cited across a set of conclusions, in first-seen order. */
export function evidenceIds(conclusions: Conclusion[]): number[] {
  const seen = new Set<number>();
  const order: number[] = [];
  for (const c of conclusions) {
    for (const e of c.evidence) {
      if (seen.has(e.activityId)) continue;
      seen.add(e.activityId);
      order.push(e.activityId);
    }
  }
  return order;
}

export interface EvidenceChip {
  activityId: number;
  /** "scr" / "mic" — the capture family the activity's first raw ref names. */
  kind: "scr" | "mic";
  atMs: number | null;
  frameId: number | null;
}

export function chipFor(activity: Activity): EvidenceChip {
  const ref = activity.evidence?.[0];
  return {
    activityId: activity.id,
    kind: ref?.subjectType === "audio_segment" ? "mic" : "scr",
    atMs: activity.startedAtMs ?? null,
    frameId: ref?.subjectType === "frame" ? ref.subjectId : null,
  };
}

/** Hand a raw evidence ref to the Timeline window (the legacy best-effort
 *  span hand-off). A frame ref is peeked in place by the caller instead. */
export async function openRefInTimeline(
  ref: ActivityEvidenceRef | undefined,
): Promise<void> {
  try {
    if (ref?.subjectType === "audio_segment") {
      await invoke("open_capture_result_in_main_window", {
        kind: "audio",
        frameId: null,
        audioSegmentId: ref.subjectId,
        spanStartMs: null,
        alignedFrameId: null,
      });
      return;
    }
    if (ref?.subjectType === "frame") {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: ref.subjectId,
        audioSegmentId: null,
      });
      return;
    }
  } catch {
    // fall through to a plain Timeline navigation
  }
  void goto("/");
}

// ---- formatting -------------------------------------------------------------

/** Whole percent. Page 09 is binding: `82%`, never `0.82`, anywhere. */
export function pct(confidence: number): number {
  return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
}

export function relativeTime(ms: number | null): string {
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
  return `${Math.floor(day / 365)}y ago`;
}

/** Coarse span between two instants — "over 9d", "over 4h". */
export function spanLabel(fromMs: number, toMs: number): string {
  const diff = Math.max(0, toMs - fromMs);
  const hr = Math.round(diff / 3_600_000);
  if (hr < 1) return "under an hour";
  if (hr < 48) return `${hr}h`;
  return `${Math.round(hr / 24)}d`;
}

export function trendLabel(t: Trend): string {
  return t === "up" ? "▲ warming" : t === "down" ? "▼ cooling" : "– steady";
}

export function trendClass(t: Trend): string {
  return t === "up" ? "trend--warm" : t === "down" ? "trend--cool" : "";
}

export function clockLabel(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms) || ms <= 0) return "";
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}
