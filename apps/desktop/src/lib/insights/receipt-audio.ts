// receipt-audio — pure, DOM-free helpers for the Activity Receipt's audio
// evidence (ADR 0049): evidence partitioning, the audio-clock→frame-index map,
// the empty-state discriminator, speaker attribution (late-bound by id), and
// the span-wide turn view model the component renders. No Svelte, no invoke —
// unit-tested in bun. The invoke-touching hydration that produces the
// `TurnView[]` these consume lives in receipt-audio-loader.ts.

import type { ActivityEvidenceRef } from "$lib/types/recording";
import type {
  AudioSegmentDto,
  AudioSegmentMediaDto,
  AudioSegmentSourceKind,
  PersonProfileDto,
  SpeakerTurnDto,
} from "$lib/types/app-infra";

/** Split an Activity's evidence into frame refs and audio-segment refs. */
export function partitionEvidence(evidence: ActivityEvidenceRef[]): {
  frames: ActivityEvidenceRef[];
  audio: ActivityEvidenceRef[];
} {
  const frames: ActivityEvidenceRef[] = [];
  const audio: ActivityEvidenceRef[] = [];
  for (const e of evidence) {
    if (e.subjectType === "audio_segment") audio.push(e);
    else if (e.subjectType === "frame") frames.push(e);
  }
  return { frames, audio };
}

/**
 * Nearest strip frame at or just before `targetMs` (frames ascending). Clamped
 * to [0, len-1]; 0 when the target precedes the first frame or the strip is
 * empty. This is how the audio-clocked clip drives the frame viewer.
 */
export function frameIndexForMs(sortedMs: number[], targetMs: number): number {
  let lo = 0;
  let hi = sortedMs.length - 1;
  let ans = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (sortedMs[mid] <= targetMs) {
      ans = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return ans;
}

export type ReceiptViewState = "frames" | "audio-only" | "expired";

/** Which viewer the receipt renders (ADR 0049): frames win; else audio if any
 *  spoken evidence survives; else the honest "footage expired" panel. */
export function receiptViewState(
  frameCount: number,
  audioEvidenceCount: number,
): ReceiptViewState {
  if (frameCount > 0) return "frames";
  if (audioEvidenceCount > 0) return "audio-only";
  return "expired";
}

/** The capture input a segment came through — shown as the "via …" subtitle.
 *  NOT a speaker label: the mic captures the whole room, so it never implies
 *  "You". Speaker attribution is by diarized voice — see {@link buildTurnViews}. */
export function sourceKindReadable(sourceKind: AudioSegmentSourceKind): string {
  return sourceKind === "microphone" ? "microphone" : "system audio";
}

function speakerCleanLabel(label: string): string {
  return label.replace(/^Maybe\s+/i, "").trim();
}
function isDefaultSpeakerLabel(label: string): boolean {
  return /^unknown speaker\s+\d+$/i.test(speakerCleanLabel(label));
}
/** True for the anonymous "Speaker N" fallback (no recognized name yet). */
export function isFallbackSpeaker(name: string): boolean {
  return /^speaker\s+\d+$/i.test(name.trim());
}

/**
 * Live-resolved display name for a turn (ADR 0049 late-binding): the profile's
 * current displayName by personId, else a cleaned "Speaker N" fallback. Never
 * frozen — resolved from `profiles` at render, so naming a voice in Timeline
 * shows here on reopen. Mirrors Timeline's isDefaultSpeakerLabel idea.
 */
export function speakerDisplay(
  turn: { personId: number | null; speakerLabel: string },
  profiles: PersonProfileDto[],
): string {
  if (turn.personId != null) {
    const name = profiles.find((p) => p.id === turn.personId)?.displayName;
    if (name && !isDefaultSpeakerLabel(name)) return name;
  }
  return speakerCleanLabel(turn.speakerLabel).replace(/^unknown\s+/i, "");
}

/** The turns' transcript joined (nulls dropped), truncated to `maxLen`.
 *  ponytail: a segment with no transcribed turns yields an empty caption — the
 *  tick/clip-bar then shows attribution only (no snippet), which is honest. */
export function captionFromTurns(
  turns: { transcriptText: string | null }[],
  maxLen = 220,
): string {
  const text = turns
    .map((t) => (t.transcriptText ?? "").trim())
    .filter((t) => t.length > 0)
    .join(" ")
    .trim();
  if (text.length <= maxLen) return text;
  return `${text.slice(0, maxLen - 1).trimEnd()}…`;
}

/** The data: URL an <audio> element plays (matches how Timeline plays audio). */
export function audioDataUrl(media: AudioSegmentMediaDto): string {
  return `data:${media.mimeType};base64,${media.dataBase64}`;
}

/**
 * Seek a clip to `offsetSec`, clamped to the real length. `offsetSec <= 0`
 * means "start at the head" (no seek). A same-segment re-seek (scrub click within
 * the already-loaded clip) keeps `readyState >= 1` and never re-fires
 * `loadedmetadata` — WKWebView won't re-fire it for an identical/unchanged src —
 * so apply the seek NOW when metadata is already in; only defer to
 * `loadedmetadata` for a freshly-set src (currentTime doesn't stick until then).
 *
 * Always clears any stale pending seek first, and uses the single-slot
 * `onloadedmetadata` PROPERTY, not an addEventListener {once} listener: a {once}
 * listener is removed only when it FIRES, not when the element's `src` is
 * swapped, so a pending seek from a superseded clip would survive and fire
 * against the LATER src. Assigning/clearing the property drops any still-pending
 * seek so a plain (head-start) clip never inherits a prior scrub's offset.
 */
export function scheduleClipSeek(el: HTMLAudioElement, offsetSec: number): void {
  el.onloadedmetadata = null; // drop any superseded pending seek
  if (offsetSec <= 0) return; // head-start: nothing to seek
  const apply = () => {
    el.currentTime = Number.isFinite(el.duration) ? Math.min(offsetSec, el.duration) : offsetSec;
  };
  if (el.readyState >= 1) apply();
  else el.onloadedmetadata = apply;
}

/**
 * The in-segment start offset (seconds) for reliving `turn` from wall-clock
 * `atMs`. Returns 0 (the segment head) when `atMs` is null or falls OUTSIDE the
 * turn's own window [segmentStartMs, endMs]. Pure.
 *
 * The guard matters for the Play/Space start-path: in frames mode the head it
 * passes is the poster FRAME's wall-clock, which in a multi-segment span sits in
 * a DIFFERENT segment than the selected turn. Without the guard, `(atMs -
 * segmentStartMs)` is enormous → scheduleClipSeek clamps it to the clip's
 * duration → the clip plays AT its end → `ended` fires → auto-advance silently
 * SKIPS the turn the user asked to hear. A scrub-release stays exact because its
 * `atMs` is chosen to fall inside the resolved turn.
 */
export function clipStartOffsetSec(
  turn: { segmentStartMs: number; endMs: number },
  atMs: number | null | undefined,
): number {
  if (atMs == null || atMs < turn.segmentStartMs || atMs > turn.endMs) return 0;
  return Math.max(0, (atMs - turn.segmentStartMs) / 1000);
}

/** The audio-only footer's left cell: honest about why there are no frames. */
export function audioFooterLeft(frameEvidenceCount: number): string {
  return frameEvidenceCount > 0
    ? "0 screen frames — screen frames have expired"
    : "0 screen frames — captured as audio";
}

// ── Span-wide turn model (Receipt redesign, slice 1) ─────────────────────────

/** One diarized speaker turn within an Activity's span, hydrated for display.
 *  Absolute epoch times, live-resolved speaker + color; ordered by start. */
export interface TurnView {
  key: string; // stable selection key: `${audioSegmentId}:${turnId}`
  turnId: number;
  audioSegmentId: number;
  segmentStartMs: number; // absolute epoch = Date.parse(seg.startedAt); clocks clip frames
  startMs: number; // absolute epoch = Date.parse(seg.startedAt) + turn.startMs
  endMs: number; // absolute epoch = Date.parse(seg.startedAt) + turn.endMs
  speaker: string; // live-resolved diarized voice: profile name or "Speaker N" (never source-kind derived)
  isFallback: boolean; // the unnamed "Speaker N" form
  colorVar: string; // a CSS custom-property NAME, e.g. "--cat-communication"
  sourceKind: AudioSegmentSourceKind | null;
  sourceMeta: string; // "microphone" / "system audio"; "" when null
  text: string; // trimmed per-turn transcript; always non-empty (wordless turns are dropped)
  cited: boolean; // audioSegmentId is in the cited ref-set
  isHeadline: boolean; // audioSegmentId === the headline cited ref's subjectId
}

/** One hydrated segment + its turns, as returned by the loader. */
export interface HydratedSegment {
  segment: AudioSegmentDto;
  turns: SpeakerTurnDto[];
}

/** Non-"You" speaker color cycle (category channels), assigned in first-
 *  appearance order and wrapping past four distinct other speakers. */
const SPEAKER_COLOR_PALETTE = [
  "--cat-meetings",
  "--cat-research",
  "--cat-entertainment",
  "--cat-creating",
];

/**
 * Speaker-name → CSS var-name. "You" is pinned to --cat-communication (the
 * audio channel lavender) and never consumes a palette slot; other distinct
 * names cycle {@link SPEAKER_COLOR_PALETTE} in first-appearance order. Same
 * name always maps to the same var. Pure.
 */
export function assignSpeakerColors(orderedNames: string[]): Map<string, string> {
  const out = new Map<string, string>();
  let next = 0;
  for (const name of orderedNames) {
    if (out.has(name)) continue;
    if (name === "You") {
      out.set(name, "--cat-communication");
      continue;
    }
    out.set(name, SPEAKER_COLOR_PALETTE[next % SPEAKER_COLOR_PALETTE.length]);
    next++;
  }
  return out;
}

/**
 * Build the ordered turn view model: flatten every turn across `segments`,
 * lift each to absolute epoch (segment start + in-segment offset), resolve its
 * speaker live via `speakerDisplay` (diarized voice → profile name or
 * `Speaker N`) — never source-kind derived, since the mic captures the whole
 * room, not just the owner — mark cited/headline by segment id, and color by
 * first-appearance order. Ascending by `startMs` (tie-break
 * `turnId`). Never throws. Wordless turns (no transcript text) are dropped —
 * matching Timeline — so a diarizer that over-clusters one voice into a real
 * cluster + a silent one never surfaces the silent one as a phantom speaker.
 */
export function buildTurnViews(
  segments: HydratedSegment[],
  citedRefs: { subjectId: number; isHeadline: boolean }[],
  profiles: PersonProfileDto[],
): TurnView[] {
  const citedIds = new Set(citedRefs.map((r) => r.subjectId));
  const headlineId = citedRefs.find((r) => r.isHeadline)?.subjectId ?? null;

  const rows = segments
    .flatMap(({ segment, turns }) => {
      const segStart = Date.parse(segment.startedAt);
      return turns.map((turn) => {
        const speaker = speakerDisplay(turn, profiles);
        return {
          key: `${segment.id}:${turn.id}`,
          turnId: turn.id,
          audioSegmentId: segment.id,
          segmentStartMs: segStart,
          startMs: segStart + turn.startMs,
          endMs: segStart + turn.endMs,
          speaker,
          isFallback: isFallbackSpeaker(speaker),
          sourceKind: segment.sourceKind ?? null,
          sourceMeta: segment.sourceKind ? sourceKindReadable(segment.sourceKind) : "",
          text: (turn.transcriptText ?? "").trim(),
          cited: citedIds.has(segment.id),
          isHeadline: segment.id === headlineId,
        };
      });
    })
    // A diarizer cluster with no transcribed words is not a speaker; drop it
    // before roster/color assignment so it never adds a phantom "Speaker N" whose
    // rows are all "—" (the exact `if (!text) continue` rule the Timeline reader
    // uses). ponytail: an all-wordless cited segment therefore shows no turns —
    // acceptable, it has no spoken evidence to read; the frame/audio player stays.
    // Also drop a turn whose segment start won't parse (Date.parse → NaN): a NaN
    // wall-clock never breaks activeKeyAt's `t.startMs > ms` scan (so it would
    // steal the karaoke highlight) and never matches turnAtMs' `ms >= startMs`
    // (scrub-to-relive silently finds nothing). Mirrors loadStrip's finite guard.
    .filter((r) => r.text.length > 0 && Number.isFinite(r.startMs) && Number.isFinite(r.endMs));

  rows.sort((a, b) => a.startMs - b.startMs || a.turnId - b.turnId);
  const colors = assignSpeakerColors(rows.map((r) => r.speaker));
  return rows.map((r) => ({ ...r, colorVar: colors.get(r.speaker) ?? "" }));
}

/**
 * Audio-only footer roster: distinct live-resolved speaker names in turn order;
 * an unnamed voice gets the "name in Timeline" nudge.
 */
export function turnSpeakerRoster(turns: TurnView[]): string {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const t of turns) {
    if (seen.has(t.speaker)) continue;
    seen.add(t.speaker);
    out.push(t.isFallback ? `${t.speaker} (unnamed → name in Timeline)` : t.speaker);
  }
  return out.join(" · ");
}
