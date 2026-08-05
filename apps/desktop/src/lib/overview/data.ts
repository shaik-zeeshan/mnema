// The Overview bento's one read pass. Every tile's data comes from a command
// that already shipped — this module adds no aggregation and no Rust.
//
// Round-4 decision G8 ("honest numbers only") is enforced here by omission:
// where the mockup drew a number this machine cannot produce (today's bytes on
// disk, a storage budget, a conversation pull-quote, a moment-citation count),
// the field simply does not exist and the tile renders without it.
//
// G11: the digest is read through `get_latest_user_context_digest` — the
// READ-ONLY door. Never `get_user_context_digest`, which generates via the LLM
// engine; a tile that mounts on app open must never start a model call.

import { invoke } from "@tauri-apps/api/core";

import { framePreviewAssetUrl } from "$lib/frame-preview";
import type { ConversationCluster, Moment } from "$lib/highlights";
import type { ConversationSummary } from "$lib/insights/conversation";
import { humanizeError } from "$lib/format-error";
import type { DayCoverage, FramePreviewDto } from "$lib/types/app-infra";
import type {
  Conclusion,
  RecordingSettings,
  UserContextDigest,
  UserContextStatus,
} from "$lib/types/recording";
import type { SystemFacts } from "$lib/types/system-facts";
import { startOfLocalDay } from "./format";

/** One tile's slice of the read: loaded value, or the message to show instead.
 *  A tile never renders a spinner — it renders chrome, then this. */
export interface Cell<T> {
  data: T | null;
  error: string | null;
}

/** A moment plus the asset URL its frame actually renders from. The frame may
 *  live inside a segment video rather than as a file, so the URL comes from
 *  `get_frame_preview`, not from `Moment.filePath`. */
export interface MomentCard extends Moment {
  url: string | null;
}

export interface OverviewSnapshot {
  coverage: Cell<DayCoverage[]>;
  moments: Cell<MomentCard[]>;
  digest: Cell<UserContextDigest | null>;
  conversations: Cell<ConversationCluster[]>;
  conclusions: Cell<Conclusion[]>;
  context: Cell<UserContextStatus | null>;
  asks: Cell<ConversationSummary[]>;
  facts: Cell<SystemFacts | null>;
  settings: Cell<RecordingSettings | null>;
}

async function cell<T>(load: Promise<T>): Promise<Cell<T>> {
  try {
    return { data: await load, error: null };
  } catch (error) {
    return { data: null, error: humanizeError(error) };
  }
}

/** Frames for the moments strip. Five frames = five preview calls; the preview
 *  command is cached backend-side, and a frame whose preview fails degrades to
 *  a placeholder tile rather than failing the strip. */
async function loadMoments(startMs: number, endMs: number): Promise<MomentCard[]> {
  const moments = await invoke<Moment[]>("get_moments", { startMs, endMs, limit: 5 });
  return Promise.all(
    moments.map(async (moment) => ({
      ...moment,
      url: await invoke<FramePreviewDto | null>("get_frame_preview", {
        request: { frameId: moment.frameId },
      })
        .then((preview) => (preview ? framePreviewAssetUrl(preview.filePath) : null))
        .catch(() => null),
    })),
  );
}

/** One pass over every Overview read, for the local day containing `now`.
 *  Failures are per-tile: one dead command never blanks the bento. */
export async function loadOverview(now: Date = new Date()): Promise<OverviewSnapshot> {
  const startMs = startOfLocalDay(now);
  const endMs = startMs + 86_400_000;

  const [coverage, moments, digest, conversations, conclusions, context, asks, facts, settings] =
    await Promise.all([
      cell(invoke<DayCoverage[]>("list_day_coverage")),
      cell(loadMoments(startMs, endMs)),
      cell(invoke<UserContextDigest | null>("get_latest_user_context_digest")),
      cell(invoke<ConversationCluster[]>("get_conversations", { startMs, endMs })),
      cell(invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false })),
      cell(invoke<UserContextStatus | null>("get_user_context_status")),
      cell(invoke<ConversationSummary[]>("list_conversations", { limit: 4, offset: 0 })),
      cell(invoke<SystemFacts | null>("get_system_facts")),
      cell(invoke<RecordingSettings | null>("get_recording_settings")),
    ]);

  return { coverage, moments, digest, conversations, conclusions, context, asks, facts, settings };
}

/** The newest Conclusion per Subject, newest Subject first — the Subjects tile's
 *  three rows. `list_user_context_conclusions` returns per-belief rows; a
 *  Subject with five beliefs would otherwise fill the tile on its own. */
export function subjectRows(conclusions: Conclusion[], limit: number): Conclusion[] {
  const bySubject = new Map<string, Conclusion>();
  for (const c of conclusions) {
    const key = c.subject.toLocaleLowerCase();
    const seen = bySubject.get(key);
    if (!seen || c.lastSupportedAtMs > seen.lastSupportedAtMs) bySubject.set(key, c);
  }
  return [...bySubject.values()]
    .sort((a, b) => b.lastSupportedAtMs - a.lastSupportedAtMs)
    .slice(0, limit);
}
