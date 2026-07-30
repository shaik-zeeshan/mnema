// audio-drawer-view — pure, DOM-free view model for the Timeline audio drawer's
// reading surface (docs/transcription/mockups/transcription-reader.html).
//
// Everything here is derivation over DTOs the app already loads: speaker marks
// (colour × shape), karaoke words, waveform bars, cluster stats and the
// name-field validation the repair slide-over gates its apply button on. No
// Svelte, no invoke — unit-tested in bun alongside the drawer components.
//
// Colour comes from `assignSpeakerColors` in lib/insights/receipt-audio.ts,
// re-keyed on clusterId, so the drawer and the Activity Receipt share ONE
// palette and one first-appearance cycle.

import { assignSpeakerColors } from "$lib/insights/receipt-audio";
import type {
  PersonProfileDto,
  SpeakerAnalysisProvenance,
  SpeakerAnalysisStructuredPayload,
  SpeakerClusterDto,
  SpeakerRecognitionConfidence,
  SpeakerTurnDto,
  TranscriptionSegment,
  TranscriptionWord,
} from "$lib/types/app-infra";

export type AudioSegmentSource = "microphone" | "systemAudio";

/** Which drawer transport shortcut a keydown matched. The page resolves this
 *  (it owns the user's rebindable bindings) and the drawer just acts on it. */
export type DrawerShortcut =
  | "playPause"
  | "seekBack"
  | "seekForward"
  | "seekBackFast"
  | "seekForwardFast";

export type AudioTranscriptStatus =
  | "idle"
  | "loading"
  | "success"
  | "empty"
  | "missing"
  | "running"
  | "error";

export type AudioSegmentRecord = {
  id: number;
  source: AudioSegmentSource;
  sessionId: string;
  segmentIndex: number;
  fileName: string;
  filePath: string;
  startUnixMs: number;
  endUnixMs: number;
  durationSeconds: number;
};

/** Consecutive turns of one cluster, merged into one readable paragraph. */
export type SpeakerTranscriptGroup = {
  clusterId: number;
  speakerLabel: string;
  personId: number | null;
  suggestedPersonId: number | null;
  recognitionConfidence: SpeakerRecognitionConfidence | null;
  recognitionScore: number | null;
  suggestedMergeTargetClusterId: number | null;
  suggestedMergeScore: number | null;
  startMs: number;
  endMs: number;
  text: string;
  overlaps: boolean;
  turnIds: number[];
};

export type SpeakerInlineAction = "confirm" | "reject" | "merge";

// ── Grouping + speaker labels ───────────────────────────────────────────────
// All pure over (turns, clusters, profiles), so the drawer resolves every label
// itself instead of the page passing a dozen formatter callbacks down.

/** Consecutive same-cluster turns merged into readable paragraphs. Turns with no
 *  transcript text are dropped: a diarizer cluster with no words is not a
 *  speaker. */
export function buildSpeakerGroups(
  turns: SpeakerTurnDto[],
  clusters: SpeakerClusterDto[],
): SpeakerTranscriptGroup[] {
  const groups: SpeakerTranscriptGroup[] = [];
  for (const turn of turns) {
    const text = turn.transcriptText?.trim() ?? "";
    if (!text) continue;
    const previous = groups.at(-1);
    if (previous && previous.clusterId === turn.clusterId) {
      previous.endMs = Math.max(previous.endMs, turn.endMs);
      previous.text = `${previous.text} ${text}`.trim();
      previous.overlaps = previous.overlaps || turn.overlaps;
      previous.turnIds.push(turn.id);
      continue;
    }
    const cluster = clusters.find((c) => c.id === turn.clusterId);
    groups.push({
      clusterId: turn.clusterId,
      speakerLabel: turn.speakerLabel,
      personId: turn.personId,
      suggestedPersonId: turn.suggestedPersonId,
      recognitionConfidence: turn.recognitionConfidence,
      recognitionScore: turn.recognitionScore,
      suggestedMergeTargetClusterId: cluster?.suggestedMergeTargetClusterId ?? null,
      suggestedMergeScore: cluster?.suggestedMergeScore ?? null,
      startMs: turn.startMs,
      endMs: turn.endMs,
      text,
      overlaps: turn.overlaps,
      turnIds: [turn.id],
    });
  }
  return groups;
}

export function speakerCleanLabel(label: string): string {
  return label.replace(/^Maybe\s+/i, "").trim();
}

export function isDefaultSpeakerLabel(label: string): boolean {
  return /^unknown speaker\s+\d+$/i.test(speakerCleanLabel(label));
}

export function speakerProfileName(
  profiles: PersonProfileDto[],
  personId: number | null,
): string | null {
  if (personId == null) return null;
  return profiles.find((profile) => profile.id === personId)?.displayName ?? null;
}

/** The name shown in the gutter: the linked person, else the cluster's label. */
export function speakerPersistedName(
  group: SpeakerTranscriptGroup,
  profiles: PersonProfileDto[],
): string {
  if (group.personId != null) {
    return speakerCleanLabel(speakerProfileName(profiles, group.personId) ?? group.speakerLabel);
  }
  return speakerCleanLabel(group.speakerLabel);
}

/** Unknown is a legitimate state, not an error — this only says "not yet named". */
export function speakerIsUnnamed(
  group: SpeakerTranscriptGroup,
  profiles: PersonProfileDto[],
): boolean {
  if (group.personId != null) return false;
  return isDefaultSpeakerLabel(group.speakerLabel);
}

export function speakerClusterOptionLabel(
  cluster: SpeakerClusterDto,
  profiles: PersonProfileDto[],
): string {
  if (cluster.personId != null) {
    const name = speakerProfileName(profiles, cluster.personId) ?? cluster.speakerLabel;
    // Owner-only auto-linking applies a name without anyone confirming it, so the
    // one thing the user must be able to do is tell those apart from the names
    // they set themselves — that is what `person_link_auto` is recorded FOR
    // (migration 0053), and Settings promises it in as many words: "Automatic
    // labels are marked as automatic and can be undone."
    return cluster.personLinkAuto ? `${name} (auto)` : name;
  }
  if (cluster.suggestedPersonId != null) {
    return (
      speakerProfileName(profiles, cluster.suggestedPersonId) ??
      `Maybe ${speakerCleanLabel(cluster.speakerLabel)}`
    );
  }
  return speakerCleanLabel(cluster.speakerLabel);
}

export function speakerSuggestedPersonName(
  group: SpeakerTranscriptGroup,
  profiles: PersonProfileDto[],
): string {
  return (
    speakerProfileName(profiles, group.suggestedPersonId) ??
    speakerCleanLabel(group.speakerLabel)
  );
}

export function suggestedMergeTargetLabel(
  group: SpeakerTranscriptGroup,
  clusters: SpeakerClusterDto[],
  profiles: PersonProfileDto[],
): string | null {
  const targetId = group.suggestedMergeTargetClusterId;
  if (targetId == null) return null;
  const target = clusters.find((cluster) => cluster.id === targetId);
  return target ? speakerClusterOptionLabel(target, profiles) : null;
}

export function isFirstVisibleClusterOccurrence(
  groups: SpeakerTranscriptGroup[],
  group: SpeakerTranscriptGroup,
  index: number,
): boolean {
  return groups.findIndex((candidate) => candidate.clusterId === group.clusterId) === index;
}

/** Both callers pass a cosine (`recognitionScore`, `suggestedMergeScore`), so
 *  the value is already a fraction — no percentage sniffing. A float epsilon
 *  over 1 renders as `1.00`, and a genuinely out-of-range score renders loudly
 *  instead of being silently rescaled. */
export function formatScore(score: number | null | undefined): string | null {
  if (score == null || !Number.isFinite(score)) return null;
  return score.toFixed(2);
}

/** The suggestion chip's meta line: `high · 0.88` (score only in dev mode). */
export function suggestionMetaLabel(
  group: SpeakerTranscriptGroup,
  showScore: boolean,
): string | null {
  const confidence = group.recognitionConfidence ?? null;
  const score = showScore ? formatScore(group.recognitionScore) : null;
  if (confidence && score) return `${confidence} · ${score}`;
  return confidence ?? score;
}

/** The gutter's inline suggestion chip, or null when this turn shouldn't carry
 *  one (already linked, no suggestion, or a repeat of the same cluster). */
export function suggestionChipFor(
  groups: SpeakerTranscriptGroup[],
  group: SpeakerTranscriptGroup,
  index: number,
  profiles: PersonProfileDto[],
  showScore: boolean,
): { name: string; meta: string | null } | null {
  if (group.suggestedPersonId == null || group.personId != null) return null;
  if (!isFirstVisibleClusterOccurrence(groups, group, index)) return null;
  return {
    name: speakerSuggestedPersonName(group, profiles),
    meta: suggestionMetaLabel(group, showScore),
  };
}

/** Where the "play 8s of each" previews start: this cluster's first turn, then
 *  the merge candidate's. Pure; the caller bounds playback. */
export function samplePreviewHeadsMs(
  group: SpeakerTranscriptGroup,
  turns: SpeakerTurnDto[],
): number[] {
  const head = (clusterId: number) =>
    turns.find((turn) => turn.clusterId === clusterId)?.startMs ?? null;
  return [head(group.clusterId), group.suggestedMergeTargetClusterId != null
    ? head(group.suggestedMergeTargetClusterId)
    : null].filter((ms): ms is number => ms != null);
}

// ── Dual encoding: colour AND shape ─────────────────────────────────────────
// Colour alone fails in greyscale, at low contrast, and under any form of
// colour blindness — the shape is the accessibility half of the pair, not
// decoration (Google Recorder's trick, mockup annotation (b)5).

export type SpeakerShape = "circle" | "square" | "triangle" | "diamond";

const SPEAKER_SHAPES: SpeakerShape[] = ["circle", "square", "triangle", "diamond"];

export interface SpeakerMark {
  /** CSS custom-property NAME, e.g. "--cat-meetings". */
  colorVar: string;
  shape: SpeakerShape;
}

/**
 * Cluster id → {colour, shape}, both assigned in first-appearance order so two
 * clusters visible in the same segment never collide on either channel until
 * there are more than four of them. Pure.
 */
export function assignSpeakerMarks(orderedClusterIds: number[]): Map<number, SpeakerMark> {
  const colors = assignSpeakerColors(orderedClusterIds.map(String));
  const out = new Map<number, SpeakerMark>();
  let next = 0;
  for (const clusterId of orderedClusterIds) {
    if (out.has(clusterId)) continue;
    out.set(clusterId, {
      colorVar: colors.get(String(clusterId)) ?? "",
      shape: SPEAKER_SHAPES[next % SPEAKER_SHAPES.length],
    });
    next += 1;
  }
  return out;
}

// ── Karaoke ─────────────────────────────────────────────────────────────────

export interface KaraokeWord {
  text: string;
  startMs: number;
  endMs: number;
}

/**
 * The payload words that belong to `[startMs, endMs]`, matched on midpoint so a
 * word straddling a turn boundary lands in exactly one paragraph. Empty when the
 * provider emitted no word timings for the range — the caller then degrades to
 * the paragraph-as-seek-target fallback.
 */
export function karaokeWordsForRange(
  words: TranscriptionWord[],
  startMs: number,
  endMs: number,
): KaraokeWord[] {
  const out: KaraokeWord[] = [];
  for (const word of words) {
    if (
      typeof word?.startMs !== "number" ||
      typeof word?.endMs !== "number" ||
      typeof word?.text !== "string"
    ) {
      continue;
    }
    const text = word.text.trim();
    if (!text) continue;
    const mid = (word.startMs + word.endMs) / 2;
    // Half-open at the START — `(startMs, endMs]`. Adjacent paragraphs share an
    // edge (contiguous transcription runs always do), and an inclusive-both-ends
    // test hands a word centred exactly on that edge to BOTH of them, which
    // renders it twice. Excluding the start rather than the end keeps the word
    // in the paragraph that owns the audio you already heard, and never drops it
    // at the tail of a paragraph followed by silence.
    if (mid <= startMs || mid > endMs) continue;
    out.push({ text, startMs: word.startMs, endMs: Math.max(word.startMs, word.endMs) });
  }
  out.sort((a, b) => a.startMs - b.startMs);
  return out;
}

/** Slack around a word's bounds: bridges the sub-frame gap between adjacent
 *  words (and `timeupdate`'s ~250 ms granularity) so the highlight doesn't
 *  blink off between them, while a real silence still clears it. */
export const KARAOKE_TOLERANCE_MS = 150;

/**
 * Index of the word holding the floor at `currentMs`, or -1 when playback sits
 * in a gap (before the first word, in a silence, or past the last one). Bounded
 * at BOTH ends on purpose: keying off `startMs` alone left the last spoken word
 * lit through every pause.
 */
export function activeKaraokeIndex(
  words: KaraokeWord[],
  currentMs: number,
  toleranceMs = KARAOKE_TOLERANCE_MS,
): number {
  for (let i = 0; i < words.length; i += 1) {
    const word = words[i];
    if (currentMs < word.startMs - toleranceMs) break; // silence before this word
    const next = words[i + 1];
    // The tolerance must never let a finished word outrank the one now speaking —
    // and equally, it must never hand the floor to the NEXT word while this one is
    // still being spoken. It only bridges the gap AFTER this word ends, so words
    // closer together than the tolerance (adjacent words usually abut) don't make
    // the highlight run up to 150 ms ahead of the audio on every word.
    if (next && currentMs >= Math.max(word.endMs, next.startMs - toleranceMs)) continue;
    return currentMs <= word.endMs + toleranceMs ? i : -1;
  }
  return -1;
}

/** A paragraph whose word timings cover less than this fraction of its
 *  characters is NOT karaoke-able: rendering only the words we have would
 *  silently drop the rest of the text off the page. */
export const KARAOKE_MIN_COVERAGE = 0.8;

/**
 * The karaoke words for one paragraph, or null when the reader must degrade to
 * the whole-paragraph seek target: no `words[]` at all, none in this range, or a
 * partial `words[]` covering under `KARAOKE_MIN_COVERAGE` of the text.
 */
export function karaokeForGroup(
  words: TranscriptionWord[],
  group: Pick<SpeakerTranscriptGroup, "startMs" | "endMs" | "text">,
): KaraokeWord[] | null {
  if (words.length === 0) return null;
  const picked = karaokeWordsForRange(words, group.startMs, group.endMs);
  if (picked.length === 0) return null;
  const joined = picked.map((w) => w.text).join(" ");
  if (joined.length < group.text.length * KARAOKE_MIN_COVERAGE) return null;
  // The symmetric failure, and the one that MISATTRIBUTES speech rather than hiding
  // it: words this paragraph does not own. Turns from different clusters overlap in
  // time (`SpeakerTurnDto.overlaps` is exactly that flag) and the backend gives each
  // word to ONE turn by maximum overlap, so a nested turn's words sit inside this
  // paragraph's range while their TEXT belongs to the other paragraph. Karaoke
  // replaces the paragraph text with these words, so rendering them here prints one
  // speaker's words under another's name, and again in their own paragraph below.
  // Counted without whitespace: the " " join would otherwise read as a leak for a
  // script whose word tokens are one character (CJK).
  // ponytail: a ratio, like the lower bound — a leak too small to move it stays.
  const spoken = (text: string) => text.replace(/\s+/g, "").length;
  if (spoken(joined) > spoken(group.text) / KARAOKE_MIN_COVERAGE) return null;
  return picked;
}

/**
 * Index of the paragraph holding the playhead, or null before playback has ever
 * moved (a drawer just opened highlights nothing).
 *
 * Start-bounded ON PURPOSE: the paragraph highlight is "where you are in the
 * document", so it must never blank. It holds through the silence after a
 * paragraph ends and stays on the last one past the final word. The asymmetry
 * with `activeKaraokeIndex` (bounded at both ends) is the design: the coarse
 * level anchors position, the fine level is precise and may show nothing.
 * Bounding this one too would flicker the highlight — and the follow-mode
 * auto-scroll — off and on across every inter-speaker gap.
 */
export function activeGroupIndex(
  groups: Pick<SpeakerTranscriptGroup, "startMs">[],
  currentMs: number,
  started: boolean,
): number | null {
  if (groups.length === 0) return null;
  if (!started) return null;
  for (let i = groups.length - 1; i >= 0; i -= 1) {
    if (currentMs >= groups[i].startMs) return i;
  }
  return null;
}

// ── Seek boundaries ─────────────────────────────────────────────────────────
// Clicking text seeks, and the target has to respect the two boundaries the
// data actually carries: the speaker turn and the transcription run. Providers
// drift word timings a few hundred ms past a turn edge, and an unclamped seek
// there lands in the NEXT speaker — the paragraph you clicked stops matching
// what you hear.

/** `ms` pulled into `[startMs, endMs]` (tolerating a reversed range). */
export function clampSeekMs(ms: number, startMs: number, endMs: number): number {
  const lo = Math.min(startMs, endMs);
  const hi = Math.max(startMs, endMs);
  if (!Number.isFinite(ms)) return lo;
  return Math.min(hi, Math.max(lo, ms));
}

/** Where a karaoke word click seeks: the word's start, held inside its turn. */
export function wordSeekMs(
  word: Pick<KaraokeWord, "startMs">,
  group: Pick<SpeakerTranscriptGroup, "startMs" | "endMs">,
): number {
  return clampSeekMs(word.startMs, group.startMs, group.endMs);
}

/**
 * Where a degraded (no `words[]`) paragraph click seeks: the start of the
 * transcription run nearest the paragraph's own start, clamped into the turn.
 *
 * ponytail: a degraded paragraph is ONE seek target, so there is no click
 * offset to resolve a run from — and `buildSpeakerGroups` keeps no per-run or
 * per-turn boundary (only `turnIds`), so intra-paragraph precision genuinely
 * does not survive grouping. This snaps to a real transcription boundary
 * instead of the raw turn edge and stops there; splitting the paragraph into
 * per-run buttons is the upgrade path if finer seeking is ever wanted.
 */
export function segmentSeekMs(
  segments: Pick<TranscriptionSegment, "startMs" | "endMs">[],
  group: Pick<SpeakerTranscriptGroup, "startMs" | "endMs">,
): number {
  let best: number | null = null;
  for (const segment of segments) {
    if (typeof segment?.startMs !== "number" || !Number.isFinite(segment.startMs)) continue;
    // Only runs that overlap this paragraph can be the thing that was clicked.
    if (segment.endMs < group.startMs || segment.startMs > group.endMs) continue;
    if (
      best == null ||
      Math.abs(segment.startMs - group.startMs) < Math.abs(best - group.startMs)
    ) {
      best = segment.startMs;
    }
  }
  return clampSeekMs(best ?? group.startMs, group.startMs, group.endMs);
}

// ── Waveform scrubber ───────────────────────────────────────────────────────

export interface WaveBar {
  /** 0–100. */
  heightPct: number;
  /** CSS custom-property name of the speaker holding the floor, else null (gap). */
  colorVar: string | null;
  atMs: number;
}

/**
 * `peaks` (0..1 amplitude, one per bucket, as returned by
 * `get_audio_segment_waveform_peaks`) laid over the turns so each bar's HUE is
 * whoever held the floor at that moment. An empty `peaks` yields no bars and the
 * caller falls back to the plain scrub bar — no error state, no empty box.
 */
export function waveformBars(
  peaks: number[],
  durationMs: number,
  turns: { startMs: number; endMs: number; clusterId: number }[],
  marks: Map<number, SpeakerMark>,
): WaveBar[] {
  if (!Array.isArray(peaks) || peaks.length === 0 || !(durationMs > 0)) return [];
  const step = durationMs / peaks.length;
  return peaks.map((peak, i) => {
    const atMs = Math.round(i * step + step / 2);
    const turn = turns.find((t) => atMs >= t.startMs && atMs <= t.endMs);
    const amp = Number.isFinite(peak) ? Math.max(0, Math.min(1, peak)) : 0;
    return {
      // Floor at 4% so a silent bucket still draws a hairline instead of nothing.
      heightPct: Math.max(4, Math.round(amp * 100)),
      colorVar: turn ? marks.get(turn.clusterId)?.colorVar ?? null : null,
      atMs,
    };
  });
}

// ── Repair slide-over ───────────────────────────────────────────────────────

/** `cluster 7 · 11 turns · 1m 48s` — derived by summing the cluster's turns. */
export function clusterSummaryLabel(clusterId: number, turns: SpeakerTurnDto[]): string {
  const mine = turns.filter((t) => t.clusterId === clusterId);
  const totalMs = mine.reduce((sum, t) => sum + Math.max(0, t.endMs - t.startMs), 0);
  const turnLabel = mine.length === 1 ? "1 turn" : `${mine.length} turns`;
  return `cluster ${clusterId} · ${turnLabel} · ${formatCompactDuration(totalMs)}`;
}

export function formatCompactDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return m > 0 ? `${m}m ${s.toString().padStart(2, "0")}s` : `${s}s`;
}

export interface NameValidation {
  ok: boolean;
  message: string | null;
}

/**
 * Live validation for the repair panel's name field. Two reachable failures:
 * empty, and "still the placeholder label" (the `Unknown speaker N` /
 * `Speaker N` form the diarizer emits). Apply is gated on `ok`.
 */
export function validateSpeakerName(value: string): NameValidation {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return { ok: false, message: "A voice needs a name before it can become a saved person." };
  }
  if (/^(unknown\s+speaker|speaker|voice)\s*\d*$/i.test(trimmed)) {
    return {
      ok: false,
      message: "Still the placeholder label. Give it a real name, or link to a saved person below.",
    };
  }
  return { ok: true, message: null };
}

/** `· 34 samples` for a person option (PersonProfileDto.embeddingCount). */
export function embeddingCountLabel(count: number | null | undefined): string {
  if (count == null || !Number.isFinite(count) || count <= 0) return "";
  return count === 1 ? "1 sample" : `${count} samples`;
}

// ── Header status pill + which panel replaces the reader ────────────────────

export type StatusTone = "ok" | "work" | "warn" | "bad" | "idle";

export interface StatusPill {
  tone: StatusTone;
  label: string;
  busy: boolean;
}

/** The header's live status vocabulary. Pure so every branch is testable. */
export function drawerStatusPill(input: {
  source: AudioSegmentSource;
  transcriptStatus: AudioTranscriptStatus;
  speakerAnalysisRunning: boolean;
  speakerAnalysisFailed: boolean;
  distinctSpeakers: number;
}): StatusPill {
  const systemAudio = input.source === "systemAudio";
  if (input.speakerAnalysisRunning) return { tone: "work", label: "speakers", busy: true };
  if (input.transcriptStatus === "loading") return { tone: "work", label: "loading", busy: true };
  if (input.transcriptStatus === "running") {
    return { tone: "work", label: systemAudio ? "detecting speech" : "processing", busy: true };
  }
  if (input.transcriptStatus === "error") {
    return { tone: "bad", label: systemAudio ? "speech detection failed" : "error", busy: false };
  }
  if (input.speakerAnalysisFailed) {
    return { tone: "bad", label: "speaker analysis failed", busy: false };
  }
  if (input.transcriptStatus === "empty") return { tone: "idle", label: "no speech", busy: false };
  if (input.transcriptStatus === "missing") return { tone: "warn", label: "not run", busy: false };
  if (input.transcriptStatus === "success") {
    return {
      tone: "ok",
      label: input.distinctSpeakers > 0 ? `${input.distinctSpeakers} speakers` : "completed",
      busy: false,
    };
  }
  return { tone: "idle", label: "unavailable", busy: false };
}

export type DrawerPanelKind =
  | "reader"
  | "skeleton"
  | "processing"
  | "no-speech"
  | "failed"
  | "speakers-failed"
  | "not-run";

/** `reader` only when there is actually something to read: a success status with
 *  zero paragraphs is the no-speech outcome, not a blank page. */
export function drawerPanelKind(input: {
  transcriptStatus: AudioTranscriptStatus;
  speakerAnalysisFailed: boolean;
  ignoreSpeakerFailure: boolean;
  groupCount: number;
}): DrawerPanelKind {
  switch (input.transcriptStatus) {
    case "loading":
      return "skeleton";
    case "running":
      return "processing";
    case "empty":
      return "no-speech";
    case "error":
      return "failed";
    case "success":
      break;
    default:
      return "not-run";
  }
  if (input.speakerAnalysisFailed && !input.ignoreSpeakerFailure) return "speakers-failed";
  return input.groupCount > 0 ? "reader" : "no-speech";
}

/**
 * When diarization produced nothing usable (or the user chose to read without
 * speakers), the transcript's own timed runs become the paragraphs. Cluster id
 * −1 marks them unattributed so the repair door stays shut.
 */
export function transcriptFallbackGroups(
  segments: TranscriptionSegment[],
  text: string | null,
  durationSeconds: number,
): SpeakerTranscriptGroup[] {
  const runs: Pick<TranscriptionSegment, "startMs" | "endMs" | "text">[] =
    segments.length > 0
      ? segments
      : text?.trim()
        ? [{ startMs: 0, endMs: Math.round(durationSeconds * 1000), text: text.trim() }]
        : [];
  return runs.map((run, i) => ({
    clusterId: -1,
    speakerLabel: "Voice",
    personId: null,
    suggestedPersonId: null,
    recognitionConfidence: null,
    recognitionScore: null,
    suggestedMergeTargetClusterId: null,
    suggestedMergeScore: null,
    startMs: run.startMs,
    endMs: run.endMs,
    text: run.text,
    overlaps: false,
    turnIds: [-(i + 1)],
  }));
}

// ── Frame 6 state panels ────────────────────────────────────────────────────

/** Speaker-analysis provenance from a processing result's structured payload.
 *  Tolerant — null on anything malformed, never throws. */
export function parseSpeakerAnalysisProvenance(
  structuredPayloadJson: string | null | undefined,
): SpeakerAnalysisProvenance | null {
  if (!structuredPayloadJson) return null;
  try {
    const parsed = JSON.parse(structuredPayloadJson) as SpeakerAnalysisStructuredPayload;
    return parsed?.metadata?.provenance ?? null;
  } catch {
    return null;
  }
}

/** `stage 2 of 3` — speech detection (system audio only) → transcription →
 *  speaker analysis. */
export function processingStageLabel(
  source: AudioSegmentSource,
  processor: "system_audio_speech_activity" | "audio_transcription" | "speaker_analysis",
): string {
  const stages: string[] =
    source === "systemAudio"
      ? ["system_audio_speech_activity", "audio_transcription", "speaker_analysis"]
      : ["audio_transcription", "speaker_analysis"];
  const index = stages.indexOf(processor);
  if (index < 0) return "";
  return `stage ${index + 1} of ${stages.length}`;
}

/** `queued 40s ago` from an ISO timestamp; "" when it won't parse. */
export function queuedAgoLabel(queuedAt: string | null | undefined, nowMs = Date.now()): string {
  if (!queuedAt) return "";
  const at = Date.parse(queuedAt);
  if (!Number.isFinite(at)) return "";
  const elapsed = Math.max(0, nowMs - at);
  return `queued ${formatCompactDuration(elapsed)} ago`;
}

/** The mono footnote under the no-speech panel: `skipReason silent · audioPeak 0.004`. */
export function provenanceFootnote(provenance: SpeakerAnalysisProvenance | null): string {
  if (!provenance) return "";
  const parts: string[] = [];
  if (provenance.skipReason) parts.push(`skipReason ${provenance.skipReason}`);
  if (typeof provenance.audioPeak === "number") {
    parts.push(`audioPeak ${provenance.audioPeak.toFixed(3)}`);
  }
  if (provenance.chunkingMode) parts.push(`chunkingMode ${provenance.chunkingMode}`);
  if (typeof provenance.chunkCount === "number" && provenance.chunkCount > 0) {
    parts.push(`${provenance.chunkCount} chunks`);
  }
  return parts.join(" · ");
}

// ── Shared formatting the drawer and its panels both need ───────────────────

/** `M:SS` for the transport and the gutter timestamps. */
export function formatPlayerTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, "0")}`;
}

export function formatTranscriptSegmentTitle(
  segment: Pick<TranscriptionSegment, "startMs" | "endMs">,
): string {
  const start = formatPlayerTime(segment.startMs / 1000);
  if (segment.endMs <= segment.startMs) return start;
  return `${start}–${formatPlayerTime(segment.endMs / 1000)}`;
}
