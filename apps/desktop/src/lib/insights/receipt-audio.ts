// receipt-audio — pure, DOM-free helpers for the Activity Receipt's audio
// evidence (ADR 0049): evidence partitioning, the audio-clock→frame-index map,
// the empty-state discriminator, speaker attribution (late-bound by id), and
// the tick / clip / footer view models the component renders. No Svelte, no
// invoke — unit-tested in bun. The invoke-touching hydration that produces the
// `AudioCitation[]` these consume lives in receipt-audio-loader.ts.

import type { ActivityEvidenceRef } from "$lib/types/recording";
import type {
  AudioSegmentDto,
  AudioSegmentMediaDto,
  AudioSegmentSourceKind,
  PersonProfileDto,
  SpeakerTurnDto,
} from "$lib/types/app-infra";

/**
 * A cited audio segment, hydrated for display. `turns` are held raw (never a
 * frozen name) so speaker attribution resolves live against the current
 * `PersonProfileDto[]` at render — naming a voice in Timeline shows on reopen.
 */
export interface AudioCitation {
  audioSegmentId: number;
  capturedAtMs: number | null; // segment start (wall-clock) → tick + clip start
  isHeadline: boolean;
  sourceKind: AudioSegmentSourceKind | null;
  startMs: number | null;
  endMs: number | null;
  turns: SpeakerTurnDto[];
  caption: string;
}

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

/** Fraction [0,1] of `ms` across a span (mirrors the component's posFor). */
export function posFraction(ms: number, startMs: number, endMs: number): number {
  const span = endMs - startMs;
  if (span <= 0) return 0;
  return Math.min(1, Math.max(0, (ms - startMs) / span));
}

/** microphone ≈ the user, system_audio ≈ whoever they were with. */
export function sourceKindLabel(sourceKind: AudioSegmentSourceKind): "You" | "Other side" {
  return sourceKind === "microphone" ? "You" : "Other side";
}
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

/** Distinct, order-preserving live-resolved speaker names across turns. */
export function distinctSpeakerNames(
  turns: SpeakerTurnDto[],
  profiles: PersonProfileDto[],
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const t of turns) {
    const name = speakerDisplay(t, profiles);
    if (!seen.has(name)) {
      seen.add(name);
      out.push(name);
    }
  }
  return out;
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

/** Assemble a display citation from a cited ref + its hydrated DTOs (pure). */
export function buildCitation(
  ref: { subjectId: number; capturedAtMs?: number | null; isHeadline: boolean },
  segment: AudioSegmentDto | null,
  turns: SpeakerTurnDto[],
): AudioCitation {
  const startMs = segment ? Date.parse(segment.startedAt) : (ref.capturedAtMs ?? null);
  return {
    audioSegmentId: ref.subjectId,
    capturedAtMs: ref.capturedAtMs ?? startMs,
    isHeadline: ref.isHeadline,
    sourceKind: segment?.sourceKind ?? null,
    startMs,
    endMs: segment ? Date.parse(segment.endedAt) : null,
    turns,
    caption: captionFromTurns(turns),
  };
}

/** The data: URL an <audio> element plays (matches how Timeline plays audio). */
export function audioDataUrl(media: AudioSegmentMediaDto): string {
  return `data:${media.mimeType};base64,${media.dataBase64}`;
}

/** The audio-only footer's left cell: honest about why there are no frames. */
export function audioFooterLeft(frameEvidenceCount: number): string {
  return frameEvidenceCount > 0
    ? "0 screen frames — screen frames have expired"
    : "0 screen frames — captured as audio";
}

/**
 * The audio-only footer's speaker roster: mic segments contribute "You",
 * system segments contribute each live-resolved name (an unnamed "Speaker N"
 * gets the "name in Timeline" nudge). Distinct, order = citation order.
 */
export function audioSpeakerSummary(
  citations: AudioCitation[],
  profiles: PersonProfileDto[],
): string {
  const out = new Set<string>();
  for (const c of citations) {
    if (c.sourceKind === "microphone") {
      out.add("You");
      continue;
    }
    if (c.turns.length === 0) {
      out.add("Other side");
      continue;
    }
    for (const name of distinctSpeakerNames(c.turns, profiles)) {
      out.add(isFallbackSpeaker(name) ? `${name} (unnamed → name in Timeline)` : name);
    }
  }
  return [...out].join(" · ");
}

// ── Render view models (keep the .svelte $derived block to one-liners) ──────

export interface AudioTickView {
  id: number;
  pos: number;
  headline: boolean;
  speaker: string;
  /** Attribution line — the tooltip's lavender uppercase label. */
  label: string;
  /** Cited transcript snippet ("" when the segment had no transcribed turns). */
  caption: string;
}

/** One clickable lavender tick per cited audio segment, with its tooltip copy. */
export function audioTickViews(
  citations: AudioCitation[],
  profiles: PersonProfileDto[],
  startMs: number,
  endMs: number,
): AudioTickView[] {
  return citations.map((c) => {
    const source = c.sourceKind ? sourceKindLabel(c.sourceKind) : "";
    const readable = c.sourceKind ? sourceKindReadable(c.sourceKind) : "";
    const names = distinctSpeakerNames(c.turns, profiles);
    const speaker = c.sourceKind === "microphone" ? "You" : (names[0] ?? "Other side");
    const label = [
      source && `${source}${readable ? ` (${readable})` : ""}`,
      "spoken evidence",
      c.isHeadline && "headline",
    ]
      .filter(Boolean)
      .join(" · ");
    return {
      id: c.audioSegmentId,
      pos: c.capturedAtMs == null ? 0 : posFraction(c.capturedAtMs, startMs, endMs),
      headline: c.isHeadline,
      speaker,
      label,
      caption: c.caption ?? "",
    };
  });
}

export interface AudioCurrentView {
  citation: AudioCitation | null;
  ordinal: number;
  total: number;
  source: string;
  readable: string;
  name: string;
}

/** The segment the audio surface is showing: the active clip, else the
 *  headline (else the first) — plus its live-resolved attribution. */
export function audioCurrentView(
  citations: AudioCitation[],
  profiles: PersonProfileDto[],
  activeClipId: number | null,
): AudioCurrentView {
  const headline = citations.find((c) => c.isHeadline) ?? citations[0] ?? null;
  const currentId = activeClipId ?? headline?.audioSegmentId ?? null;
  const citation = citations.find((c) => c.audioSegmentId === currentId) ?? headline;
  const ordinal = citation
    ? citations.findIndex((c) => c.audioSegmentId === citation.audioSegmentId) + 1
    : 0;
  const source = citation?.sourceKind ? sourceKindLabel(citation.sourceKind) : "";
  const readable = citation?.sourceKind ? sourceKindReadable(citation.sourceKind) : "";
  const names = citation ? distinctSpeakerNames(citation.turns, profiles) : [];
  const name = citation?.sourceKind === "microphone" ? "You" : (names[0] ?? "Other side");
  return { citation, ordinal, total: citations.length, source, readable, name };
}

/** "microphone · 10:04:02–10:05:47 · 1×" — the inert rate line on a clip. */
export function clipRateLabelOf(c: AudioCitation | null, fmt: (ms: number) => string): string {
  if (!c) return "1×";
  const readable = c.sourceKind ? sourceKindReadable(c.sourceKind) : "audio";
  const span = c.startMs != null && c.endMs != null ? `${fmt(c.startMs)}–${fmt(c.endMs)} · ` : "";
  return `${readable} · ${span}1×`;
}
