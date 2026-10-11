// Quick Look's pure math (bun-tested): the screen strip (the frame nearest
// each 15 s step within ±90 s of the match), the 7-cell window that follows
// the scrub, and small labels. No Svelte, no tauri.
import type { ScrubPreviewMissingReason } from "$lib/types/app-infra";
import type { AskAiSource } from "$lib/insights/conversation";
import type { RankedResult } from "./result-sections";

export const STEP_MS = 15_000;
// ±6 steps = the ±90 s `list_frame_summaries_in_range` window.
export const RANGE = 6;
export const STRIP_CELLS = 7;

export type StripFrame = { id: number; ms: number };

// One entry per step k ∈ [-RANGE, RANGE] (index k + RANGE): the captured frame
// nearest matchMs + k·15 s, or null when nothing was captured in the window.
// Step 0 is always the hit's own frame. Sparse capture can repeat a frame
// across steps — honest, and its label says how far it really is.
export function pickStrip(
  frames: readonly StripFrame[],
  match: StripFrame,
): (StripFrame | null)[] {
  return Array.from({ length: 2 * RANGE + 1 }, (_, i) => {
    const k = i - RANGE;
    if (k === 0) return match;
    const at = match.ms + k * STEP_MS;
    let best: StripFrame | null = null;
    for (const f of frames) {
      if (best === null || Math.abs(f.ms - at) < Math.abs(best.ms - at)) best = f;
    }
    return best;
  });
}

// `size` consecutive positions in [lo, hi] around `k`, centred where room
// allows (screen: 7 of −6…6; audio: 4 turns of 0…n−1, wide enough to read).
export function stripWindow(k: number, lo: number, hi: number, size = STRIP_CELLS): number[] {
  const start = Math.max(lo, Math.min(k - Math.floor((size - 1) / 2), hi - size + 1));
  const out: number[] = [];
  for (let j = start; j <= hi && out.length < size; j++) out.push(j);
  return out;
}

// m:ss (h:mm:ss past the hour).
export function clock(ms: number): string {
  const t = Math.max(0, Math.round(Math.abs(ms) / 1000));
  const ss = String(t % 60).padStart(2, "0");
  const m = Math.floor(t / 60);
  return m >= 60 ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

// "+0:15" / "−0:45" from the match.
export function signedClock(ms: number): string {
  return `${ms >= 0 ? "+" : "−"}${clock(ms)}`;
}

// What the stage says instead of a picture (`missingReason` from
// `get_frame_scrub_previews`). The frame's text is still shown beside it.
export function missingCopy(reason: ScrubPreviewMissingReason): { title: string; text: string } {
  switch (reason) {
    case "direct_file_missing":
    case "segment_video_missing":
      return {
        title: "This frame’s video is missing",
        text: "Its recording isn’t on disk any more. The text read off it is still here.",
      };
    case "frame_not_found":
      return { title: "This frame is gone", text: "It was deleted after the search ran." };
    case "decode_failed":
      return {
        title: "This frame couldn’t be decoded",
        text: "Its recording is damaged at this point. The text read off it is still here.",
      };
    default:
      return {
        title: "This frame can’t be shown",
        text: "Mnema couldn’t find it inside its recording. The text read off it is still here.",
      };
  }
}

// A cited source as a Quick Look result: its own search hit when the list has
// it (same frame / same recording, nearest span), else one built from the
// citation — no snippet or match boxes, so the stage shows the bare frame.
export function sourceResult(s: AskAiSource, ranked: readonly RankedResult[]): RankedResult | null {
  if (s.kind === "frame") {
    if (s.frameId == null) return null;
    const hit = ranked.find((r) => r.kind === "frame" && r.frame.representativeFrame.id === s.frameId);
    if (hit) return hit;
    const frame = {
      id: s.frameId, sessionId: "", filePath: "", capturedAt: s.startedAt, width: null, height: null,
      appBundleId: null, appName: s.appName, windowTitle: s.windowTitle, url: s.url, ocrText: null,
      processorVersion: null, equivalenceHint: null, createdAt: s.startedAt, updatedAt: s.startedAt,
    };
    return {
      kind: "frame",
      frame: {
        rank: 0, groupKey: `cite-f${s.frameId}`, representativeFrame: frame, groupStartAt: s.startedAt,
        groupEndAt: s.endedAt, matchCount: 0, snippet: "", appBundleId: null, appName: s.appName,
        windowTitle: s.windowTitle, url: s.url, thumbnailFrameId: s.frameId, textSourceKind: "direct",
        hasSecretRedactions: false, secretRedactionCount: 0, foundByMeaning: false, matchBoxes: [],
      },
    };
  }
  if (s.audioSegmentId == null) return null;
  const span = s.spanStartMs ?? 0;
  let hit: RankedResult | null = null;
  for (const r of ranked) {
    if (r.kind !== "audio" || r.audio.audioSegment.id !== s.audioSegmentId) continue;
    if (!hit || (hit.kind === "audio" && Math.abs(r.audio.spanStartMs - span) < Math.abs(hit.audio.spanStartMs - span))) hit = r;
  }
  if (hit) return hit;
  const sourceKind = s.sourceKind === "microphone" ? "microphone" : "system_audio";
  const at = (ms: number) => new Date(new Date(s.startedAt).getTime() + ms).toISOString();
  return {
    kind: "audio",
    audio: {
      rank: 0, groupKey: `cite-a${s.audioSegmentId}-${span}`,
      audioSegment: {
        id: s.audioSegmentId, sourceKind, sourceSessionId: "", segmentIndex: 0, filePath: "",
        startedAt: s.startedAt, endedAt: s.endedAt, createdAt: s.startedAt, updatedAt: s.startedAt,
      },
      sourceKind, spanStartMs: span, spanEndMs: span, absoluteStartAt: at(span), absoluteEndAt: at(span),
      matchCount: 0, snippet: "",
      alignedFrame: s.alignedFrameId == null ? null : {
        id: s.alignedFrameId, sessionId: "", filePath: "", capturedAt: at(span), width: null, height: null,
        appBundleId: null, appName: null, windowTitle: null, url: null, ocrText: null, processorVersion: null,
        equivalenceHint: null, createdAt: at(span), updatedAt: at(span),
      },
      hasSecretRedactions: false, secretRedactionCount: 0, foundByMeaning: false,
    },
  };
}
