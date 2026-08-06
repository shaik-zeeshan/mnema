// ── Overview bento — one read per tile, all of them optional ─────────────────
// The Overview (⌘2) is a read-only board: it never generates, never writes, and
// every tile degrades to an honest empty state on its own. So each read is
// isolated in its own try/catch — one dead command must not blank the board.
//
// Commands used (all already registered; this slice adds no backend):
//   get_moments / get_conversations   day highlights (`highlights.rs`)
//   list_day_coverage                 per-local-day capture (G6's query, G11's tile)
//   get_latest_user_context_digest    read-only digest — NEVER the generating door
//   list_conversations                Ask history (the conversation store)
//   list_user_context_conclusions     range-scoped, so it stays bounded
//
// G8: no number is invented here. A tile with no data says so.
// The pure shaping half lives in `overview-shape.ts` (and is what the tests hit).

import { invoke } from "@tauri-apps/api/core";
import { framePreviewAssetUrl } from "$lib/frame-preview";
import type { ConversationCluster, Moment } from "$lib/highlights";
import type { ConversationSummary } from "$lib/insights/conversation";
import {
  openThreadSentence,
  subjectsTile,
  todayRange,
  weekFromCoverage,
} from "$lib/overview/overview-shape";
import type { SubjectsTileSummary, WeekDay } from "$lib/overview/overview-shape";
import type { DayCoverage, FramePreviewDto, GetFramePreviewRequest } from "$lib/types/app-infra";
import type { Conclusion, UserContextDigest } from "$lib/types/recording";

/** Frames shown in the moments strip. More than fits on screen is wasted decode. */
const MOMENT_LIMIT = 8;
/** Ask-history rows read for the 2×1 tile. */
const ASK_LIMIT = 4;

export interface MomentCard {
  moment: Moment;
  /** Asset URL for the decoded frame, or null when the segment is still in
   *  flight (the caption still carries the fact — no placeholder art). */
  previewUrl: string | null;
}

export interface OverviewSnapshot {
  capturedTodayMs: number;
  week: WeekDay[];
  moments: MomentCard[];
  conversations: ConversationCluster[];
  digest: UserContextDigest | null;
  /** G11: Open Threads v1 is the digest's own prose sentence — nothing extracted. */
  openThread: string | null;
  asks: ConversationSummary[];
  conclusions: Conclusion[];
  /** Page 09's door: subjects = client-side group-by over the whole dossier's
   *  conclusions (unscoped, unlike `conclusions` above which is today-only). */
  subjects: SubjectsTileSummary;
  /** Page 10's door footer: non-dismissed conclusions in the whole dossier. */
  dossierCount: number;
}

export const EMPTY_SNAPSHOT: OverviewSnapshot = {
  capturedTodayMs: 0,
  week: [],
  moments: [],
  conversations: [],
  digest: null,
  openThread: null,
  asks: [],
  conclusions: [],
  subjects: { rows: [], activeCount: 0, fadingCount: 0 },
  dossierCount: 0,
};

async function safe<T>(run: () => Promise<T>, fallback: T): Promise<T> {
  try {
    return await run();
  } catch {
    return fallback;
  }
}

/** Decode the moments' frames. One `get_frame_preview` per moment (≤8), in
 *  parallel — a frame whose segment is still being written just has no image. */
async function withPreviews(moments: Moment[]): Promise<MomentCard[]> {
  return Promise.all(
    moments.map(async (moment) => ({
      moment,
      previewUrl: await safe(async () => {
        const dto = await invoke<FramePreviewDto | null>("get_frame_preview", {
          request: { frameId: moment.frameId } satisfies GetFramePreviewRequest,
        });
        return dto ? framePreviewAssetUrl(dto.filePath) : null;
      }, null),
    })),
  );
}

/** Load every tile at once. Never throws: a failed read is an empty tile. */
export async function loadOverview(now: Date = new Date()): Promise<OverviewSnapshot> {
  const { startMs, endMs } = todayRange(now);
  const [moments, conversations, coverage, digest, asks, conclusions, allConclusions] = await Promise.all([
    safe(() => invoke<Moment[]>("get_moments", { startMs, endMs, limit: MOMENT_LIMIT }), []),
    safe(() => invoke<ConversationCluster[]>("get_conversations", { startMs, endMs }), []),
    safe(() => invoke<DayCoverage[]>("list_day_coverage"), []),
    safe(() => invoke<UserContextDigest | null>("get_latest_user_context_digest"), null),
    safe(
      () => invoke<ConversationSummary[]>("list_conversations", { limit: ASK_LIMIT, offset: 0 }),
      [],
    ),
    safe(
      () =>
        invoke<Conclusion[]>("list_user_context_conclusions", {
          includeFaded: false,
          startMs,
          endMs,
        }),
      [],
    ),
    // The Subjects door: the whole dossier (same read the Subjects index makes),
    // so the tile's active/fading split matches what opening it shows.
    safe(
      () => invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: true }),
      [],
    ),
  ]);

  const week = weekFromCoverage(coverage, now);
  return {
    capturedTodayMs: week.find((day) => day.isToday)?.coveredMs ?? 0,
    week,
    moments: await withPreviews(moments),
    conversations,
    digest,
    openThread: openThreadSentence(digest?.narrative),
    asks,
    conclusions,
    subjects: subjectsTile(allConclusions),
    dossierCount: allConclusions.filter((c) => c.status !== "dismissed").length,
  };
}
