// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  activeGroupIndex,
  activeKaraokeIndex,
  assignSpeakerMarks,
  buildSpeakerGroups,
  clampSeekMs,
  segmentSeekMs,
  wordSeekMs,
  drawerPanelKind,
  drawerStatusPill,
  transcriptFallbackGroups,
  clusterSummaryLabel,
  embeddingCountLabel,
  formatPlayerTime,
  formatScore,
  formatTranscriptSegmentTitle,
  isDefaultSpeakerLabel,
  karaokeForGroup,
  karaokeWordsForRange,
  parseSpeakerAnalysisProvenance,
  processingStageLabel,
  provenanceFootnote,
  queuedAgoLabel,
  samplePreviewHeadsMs,
  speakerCleanLabel,
  speakerClusterOptionLabel,
  speakerIsUnnamed,
  speakerPersistedName,
  speakerProfileName,
  speakerSuggestedPersonName,
  suggestedMergeTargetLabel,
  suggestionChipFor,
  validateSpeakerName,
  waveformBars,
  type SpeakerTranscriptGroup,
} from "./audio-drawer-view";
import type {
  PersonProfileDto,
  SpeakerClusterDto,
  SpeakerTurnDto,
  TranscriptionWord,
} from "$lib/types/app-infra";

function turn(over: Partial<SpeakerTurnDto>): SpeakerTurnDto {
  return {
    id: 1,
    audioSegmentId: 1,
    sessionId: "s",
    clusterId: 7,
    segmentClusterId: null,
    providerClusterId: "0",
    speakerLabel: "Unknown speaker 1",
    personId: null,
    suggestedPersonId: null,
    recognitionConfidence: null,
    recognitionScore: null,
    startMs: 0,
    endMs: 1000,
    transcriptText: "hi",
    overlaps: false,
    ...over,
  };
}

function group(over: Partial<SpeakerTranscriptGroup> = {}): SpeakerTranscriptGroup {
  return {
    clusterId: 7,
    speakerLabel: "Unknown Speaker 1",
    personId: null,
    suggestedPersonId: null,
    recognitionConfidence: null,
    recognitionScore: null,
    suggestedMergeTargetClusterId: null,
    suggestedMergeScore: null,
    startMs: 0,
    endMs: 1000,
    text: "hi",
    overlaps: false,
    turnIds: [1],
    ...over,
  };
}

function cluster(over: Partial<SpeakerClusterDto> = {}): SpeakerClusterDto {
  return {
    id: 7,
    sessionId: "s",
    provider: "speakrs",
    modelId: null,
    providerClusterId: "0",
    speakerLabel: "Unknown Speaker 1",
    personId: null,
    suggestedPersonId: null,
    recognitionConfidence: null,
    recognitionScore: null,
    suggestedMergeTargetClusterId: null,
    suggestedMergeScore: null,
    ...over,
  };
}

const PROFILES: PersonProfileDto[] = [
  { id: 1, displayName: "Priya", notes: null, embeddingCount: 4, createdAt: "", updatedAt: "" },
  {
    id: 2,
    displayName: "Daniel Okafor",
    notes: null,
    embeddingCount: 12,
    createdAt: "",
    updatedAt: "",
  },
];

describe("assignSpeakerMarks", () => {
  it("gives distinct colour AND shape to the first four clusters", () => {
    const marks = assignSpeakerMarks([3, 7, 11, 12, 3]);
    const shapes = [3, 7, 11, 12].map((id) => marks.get(id)!.shape);
    expect(new Set(shapes).size).toBe(4);
    const colors = [3, 7, 11, 12].map((id) => marks.get(id)!.colorVar);
    expect(new Set(colors).size).toBe(4);
    // stable per cluster across repeats
    expect(marks.get(3)!.shape).toBe("circle");
  });

  it("reuses a colour past four clusters but never the same colour AND shape", () => {
    const marks = assignSpeakerMarks([1, 2, 3, 4, 5]);
    // Four colours, so the fifth speaker has to share one — that is the ceiling.
    expect(marks.get(5)!.colorVar).toBe(marks.get(1)!.colorVar);
    // ...but sharing the SHAPE too would make them indistinguishable, which is
    // the defect this offset exists to prevent.
    expect(marks.get(5)!.shape).not.toBe(marks.get(1)!.shape);
  });

  it("keeps colour-and-shape pairs unique for sixteen clusters", () => {
    const ids = Array.from({ length: 16 }, (_, i) => i + 1);
    const marks = assignSpeakerMarks(ids);
    const pairs = ids.map((id) => `${marks.get(id)!.colorVar}/${marks.get(id)!.shape}`);
    expect(new Set(pairs).size).toBe(16);
  });
});

describe("karaokeWordsForRange", () => {
  const words: TranscriptionWord[] = [
    { startMs: 0, endMs: 400, text: "one" },
    { startMs: 400, endMs: 900, text: " two " },
    { startMs: 900, endMs: 1400, text: "three" },
    { startMs: 5000, endMs: 5400, text: "later" },
  ];

  it("selects by midpoint so a straddling word lands in one paragraph only", () => {
    const a = karaokeWordsForRange(words, 0, 1000);
    const b = karaokeWordsForRange(words, 1000, 6000);
    // "three" spans 900–1400, midpoint 1150 → belongs to the second paragraph only
    expect(a.map((w) => w.text)).toEqual(["one", "two"]);
    expect(b.map((w) => w.text)).toEqual(["three", "later"]);
  });

  it("gives a word straddling a shared paragraph boundary to exactly one paragraph", () => {
    // Adjacent paragraphs meeting at 1000 (contiguous transcription runs are the
    // norm on the fallback path) and a word centred on that boundary.
    const straddling: TranscriptionWord[] = [{ startMs: 800, endMs: 1200, text: "boundary" }];
    const first = karaokeWordsForRange(straddling, 0, 1000);
    const second = karaokeWordsForRange(straddling, 1000, 2000);
    expect(first.length + second.length).toBe(1);
  });

  it("returns empty for a provider with no word timings (the fallback branch)", () => {
    expect(karaokeWordsForRange([], 0, 1000)).toEqual([]);
    // malformed entries are dropped, never thrown on
    expect(karaokeWordsForRange([{} as TranscriptionWord], 0, 1000)).toEqual([]);
  });

  it("tracks the active word by playback time", () => {
    const w = karaokeWordsForRange(words, 0, 1000);
    expect(activeKaraokeIndex(w, -1000)).toBe(-1);
    expect(activeKaraokeIndex(w, 0)).toBe(0);
    expect(activeKaraokeIndex(w, 500)).toBe(1);
  });

  it("keeps the word being spoken lit until it ends, even with the next one abutting", () => {
    // "one" runs 0–400 and "two" starts at 400: at 300ms the user still hears
    // "one", so the highlight must not have jumped ahead already.
    const w = karaokeWordsForRange(words, 0, 1000);
    expect(activeKaraokeIndex(w, 300)).toBe(0);
    expect(activeKaraokeIndex(w, 399)).toBe(0);
  });

  it("goes dark in a silence instead of leaving the last word lit", () => {
    const w = karaokeWordsForRange(words, 0, 6000);
    // "three" ends at 1400, "later" starts at 5000 → the gap owns nobody
    expect(activeKaraokeIndex(w, 3000)).toBe(-1);
    // and past the end of the last word
    expect(activeKaraokeIndex(w, 99999)).toBe(-1);
    // a hairline gap between adjacent words keeps the earlier one lit
    expect(activeKaraokeIndex(w, 1450)).toBe(2);
  });
});

describe("seek boundaries", () => {
  const group = { startMs: 4000, endMs: 9000 };

  it("clamps a word that starts before its turn", () => {
    // provider drift: the word claims 3400 but its turn opens at 4000
    expect(wordSeekMs({ startMs: 3400 }, group)).toBe(4000);
  });

  it("clamps a word that starts after its turn ends", () => {
    expect(wordSeekMs({ startMs: 9600 }, group)).toBe(9000);
  });

  it("leaves an in-bounds word alone", () => {
    expect(wordSeekMs({ startMs: 6200 }, group)).toBe(6200);
    // reversed range and non-finite input never escape the turn
    expect(clampSeekMs(6200, 9000, 4000)).toBe(6200);
    expect(clampSeekMs(Number.NaN, 4000, 9000)).toBe(4000);
  });

  it("picks the transcription run nearest the paragraph, not the turn edge", () => {
    const segments = [
      { startMs: 0, endMs: 3000, text: "before" },
      { startMs: 4200, endMs: 6000, text: "first run" },
      { startMs: 6100, endMs: 9000, text: "second run" },
      { startMs: 20_000, endMs: 22_000, text: "after" },
    ];
    // the turn opens at 4000 but transcription only starts speaking at 4200
    expect(segmentSeekMs(segments, group)).toBe(4200);
    // a paragraph whose nearest overlapping run started earlier clamps up to
    // the paragraph's own start rather than seeking into the previous speaker
    expect(segmentSeekMs(segments, { startMs: 6500, endMs: 9000 })).toBe(6500);
  });

  it("falls back to the paragraph start with no usable runs", () => {
    expect(segmentSeekMs([], group)).toBe(4000);
    expect(segmentSeekMs([{ startMs: 20_000, endMs: 22_000, text: "elsewhere" }], group)).toBe(4000);
  });
});

describe("waveformBars", () => {
  const turns = [{ startMs: 0, endMs: 1000, clusterId: 7 }];
  const marks = assignSpeakerMarks([7]);

  it("returns no bars on an empty peaks array (fall back to the plain scrubber)", () => {
    expect(waveformBars([], 2000, turns, marks)).toEqual([]);
    expect(waveformBars([0.5], 0, turns, marks)).toEqual([]);
  });

  it("hues each bar by the speaker holding the floor and leaves gaps uncoloured", () => {
    const bars = waveformBars([1, 1, 1, 1], 2000, turns, marks);
    expect(bars).toHaveLength(4);
    expect(bars[0].colorVar).toBe(marks.get(7)!.colorVar);
    expect(bars[0].heightPct).toBe(100);
    // second half of the 2s segment has no turn covering it
    expect(bars[3].colorVar).toBeNull();
  });

  it("floors a silent bucket to a visible hairline", () => {
    expect(waveformBars([0], 1000, turns, marks)[0].heightPct).toBe(4);
  });

  it("clamps a peak above 1 and survives a non-finite one", () => {
    // ponytail: no below-0 case — the 4% floor swallows it, so such a test could
    // never fail even with the lower clamp deleted.
    expect(waveformBars([2, Number.NaN], 1000, turns, marks).map((b) => b.heightPct))
      .toEqual([100, 4]);
  });
});

describe("validateSpeakerName", () => {
  it("rejects empty and placeholder labels, accepts a real name", () => {
    expect(validateSpeakerName("  ").ok).toBe(false);
    expect(validateSpeakerName("Unknown speaker 2").ok).toBe(false);
    expect(validateSpeakerName("Speaker 3").ok).toBe(false);
    expect(validateSpeakerName("Voice").ok).toBe(false);
    expect(validateSpeakerName("Priya Raghunathan").ok).toBe(true);
    expect(validateSpeakerName("Priya Raghunathan").message).toBeNull();
  });
});

describe("labels", () => {
  it("summarises a cluster's turns and total speaking time", () => {
    expect(
      clusterSummaryLabel(7, [
        turn({ id: 1, startMs: 0, endMs: 60_000 }),
        turn({ id: 2, startMs: 60_000, endMs: 108_000 }),
        turn({ id: 3, clusterId: 9, startMs: 0, endMs: 5_000 }),
      ]),
    ).toBe("cluster 7 · 2 turns · 1m 48s");
  });

  it("uses the singular for one turn and still names an empty cluster", () => {
    expect(clusterSummaryLabel(7, [turn({ id: 1, startMs: 0, endMs: 5000 })]))
      .toBe("cluster 7 · 1 turn · 5s");
    expect(clusterSummaryLabel(4, [])).toBe("cluster 4 · 0 turns · 0s");
  });

  it("names the pipeline stage per source kind", () => {
    expect(processingStageLabel("microphone", "audio_transcription")).toBe("stage 1 of 2");
    expect(processingStageLabel("systemAudio", "audio_transcription")).toBe("stage 2 of 3");
    expect(processingStageLabel("systemAudio", "speaker_analysis")).toBe("stage 3 of 3");
  });

  it("formats queued-ago and the provenance footnote", () => {
    const now = Date.parse("2026-07-25T10:00:40Z");
    expect(queuedAgoLabel("2026-07-25T10:00:00Z", now)).toBe("queued 40s ago");
    expect(queuedAgoLabel(null, now)).toBe("");
    expect(queuedAgoLabel("nonsense", now)).toBe("");
    expect(provenanceFootnote({ skipReason: "silent", audioPeak: 0.004 })).toBe(
      "skipReason silent · audioPeak 0.004",
    );
    expect(provenanceFootnote({ chunkingMode: "safe_chunked", chunkCount: 5 })).toBe(
      "chunkingMode safe_chunked · 5 chunks",
    );
    expect(provenanceFootnote(null)).toBe("");
  });

  it("labels embedding sample counts", () => {
    expect(embeddingCountLabel(34)).toBe("34 samples");
    expect(embeddingCountLabel(1)).toBe("1 sample");
    expect(embeddingCountLabel(0)).toBe("");
    expect(embeddingCountLabel(null)).toBe("");
  });
});

describe("drawerStatusPill", () => {
  const base = {
    source: "microphone" as const,
    transcriptStatus: "success" as const,
    speakerAnalysisRunning: false,
    speakerAnalysisFailed: false,
    distinctSpeakers: 3,
  };

  it("counts speakers on the happy path", () => {
    expect(drawerStatusPill(base)).toEqual({ tone: "ok", label: "3 speakers", busy: false });
  });

  it("prefers the in-flight speaker pass over a competing busy status AND a failure", () => {
    // `busy` alone can't tell this branch from any other busy one, so assert the
    // whole pill against statuses that would otherwise win.
    expect(
      drawerStatusPill({ ...base, speakerAnalysisRunning: true, transcriptStatus: "loading" }),
    ).toEqual({ tone: "work", label: "speakers", busy: true });
    expect(
      drawerStatusPill({
        ...base,
        speakerAnalysisRunning: true,
        transcriptStatus: "error",
        speakerAnalysisFailed: true,
      }),
    ).toEqual({ tone: "work", label: "speakers", busy: true });
  });

  it("uses system-audio wording for the speech-detection stage", () => {
    expect(drawerStatusPill({ ...base, source: "systemAudio", transcriptStatus: "running" }).label)
      .toBe("detecting speech");
    expect(drawerStatusPill({ ...base, transcriptStatus: "running" }).label).toBe("processing");
  });

  it("marks a failed speaker pass bad even though the transcript succeeded", () => {
    const pill = drawerStatusPill({ ...base, speakerAnalysisFailed: true });
    expect(pill.tone).toBe("bad");
    expect(pill.label).toBe("speaker analysis failed");
  });

  it("treats not-run as a warning, not an error", () => {
    expect(drawerStatusPill({ ...base, transcriptStatus: "missing" }).tone).toBe("warn");
  });

  it("uses system-audio wording for a FAILED transcript too, not just a running one", () => {
    expect(
      drawerStatusPill({ ...base, source: "systemAudio", transcriptStatus: "error" }),
    ).toEqual({ tone: "bad", label: "speech detection failed", busy: false });
    expect(drawerStatusPill({ ...base, transcriptStatus: "error" })).toEqual({
      tone: "bad",
      label: "error",
      busy: false,
    });
  });

  it("covers the rest of the vocabulary: loading, empty, no speakers, idle", () => {
    expect(drawerStatusPill({ ...base, transcriptStatus: "loading" })).toEqual({
      tone: "work",
      label: "loading",
      busy: true,
    });
    expect(drawerStatusPill({ ...base, transcriptStatus: "empty" })).toEqual({
      tone: "idle",
      label: "no speech",
      busy: false,
    });
    // transcribed, but diarization attributed it to nobody
    expect(drawerStatusPill({ ...base, distinctSpeakers: 0 })).toEqual({
      tone: "ok",
      label: "completed",
      busy: false,
    });
    expect(drawerStatusPill({ ...base, transcriptStatus: "idle" })).toEqual({
      tone: "idle",
      label: "unavailable",
      busy: false,
    });
  });
});

describe("drawerPanelKind", () => {
  const base = {
    transcriptStatus: "success" as const,
    speakerAnalysisFailed: false,
    ignoreSpeakerFailure: false,
    groupCount: 4,
  };

  it("maps each status to its panel", () => {
    expect(drawerPanelKind({ ...base, transcriptStatus: "loading" })).toBe("skeleton");
    expect(drawerPanelKind({ ...base, transcriptStatus: "running" })).toBe("processing");
    expect(drawerPanelKind({ ...base, transcriptStatus: "empty" })).toBe("no-speech");
    expect(drawerPanelKind({ ...base, transcriptStatus: "error" })).toBe("failed");
    expect(drawerPanelKind({ ...base, transcriptStatus: "missing" })).toBe("not-run");
    expect(drawerPanelKind({ ...base, transcriptStatus: "idle" })).toBe("not-run");
    expect(drawerPanelKind(base)).toBe("reader");
  });

  it("offers the read-without-speakers escape and honours it", () => {
    expect(drawerPanelKind({ ...base, speakerAnalysisFailed: true })).toBe("speakers-failed");
    expect(
      drawerPanelKind({ ...base, speakerAnalysisFailed: true, ignoreSpeakerFailure: true }),
    ).toBe("reader");
  });

  it("never renders a blank reader: success with nothing to read is no-speech", () => {
    expect(drawerPanelKind({ ...base, groupCount: 0 })).toBe("no-speech");
  });
});

describe("transcriptFallbackGroups", () => {
  it("turns timed runs into unattributed paragraphs", () => {
    const groups = transcriptFallbackGroups(
      [
        { startMs: 0, endMs: 900, text: "hello" },
        { startMs: 900, endMs: 1800, text: "there" },
      ],
      null,
      5,
    );
    expect(groups).toHaveLength(2);
    // cluster -1 keeps the repair door shut for text with no diarized owner
    expect(groups.every((g) => g.clusterId === -1)).toBe(true);
    expect(groups[1].text).toBe("there");
  });

  it("falls back to one whole-segment paragraph, then to nothing", () => {
    const one = transcriptFallbackGroups([], "  a whole transcript ", 12);
    expect(one).toHaveLength(1);
    expect(one[0]).toMatchObject({ startMs: 0, endMs: 12000, text: "a whole transcript" });
    expect(transcriptFallbackGroups([], "   ", 12)).toEqual([]);
    expect(transcriptFallbackGroups([], null, 12)).toEqual([]);
  });
});

describe("suggestionChipFor / samplePreviewHeadsMs", () => {
  const groups = [
    { clusterId: 7, personId: null, suggestedPersonId: 2, recognitionConfidence: "high",
      recognitionScore: 0.88, suggestedMergeTargetClusterId: 9, suggestedMergeScore: 0.71,
      speakerLabel: "Unknown speaker 2", startMs: 0, endMs: 10, text: "a", overlaps: false, turnIds: [1] },
    { clusterId: 7, personId: null, suggestedPersonId: 2, recognitionConfidence: "high",
      recognitionScore: 0.88, suggestedMergeTargetClusterId: 9, suggestedMergeScore: 0.71,
      speakerLabel: "Unknown speaker 2", startMs: 20, endMs: 30, text: "b", overlaps: false, turnIds: [2] },
    { clusterId: 3, personId: 1, suggestedPersonId: null, recognitionConfidence: null,
      recognitionScore: null, suggestedMergeTargetClusterId: null, suggestedMergeScore: null,
      speakerLabel: "Priya", startMs: 40, endMs: 50, text: "c", overlaps: false, turnIds: [3] },
  ];
  const profiles = [
    { id: 1, displayName: "Priya", notes: null, embeddingCount: 1, createdAt: "", updatedAt: "" },
    { id: 2, displayName: "Daniel Okafor", notes: null, embeddingCount: 12, createdAt: "", updatedAt: "" },
  ];

  it("chips only the FIRST visible turn of an unconfirmed cluster", () => {
    expect(suggestionChipFor(groups, groups[0], 0, profiles, true)).toEqual({
      name: "Daniel Okafor",
      meta: "high · 0.88",
    });
    expect(suggestionChipFor(groups, groups[1], 1, profiles, true)).toBeNull();
    // already linked → nothing to suggest
    expect(suggestionChipFor(groups, groups[2], 2, profiles, true)).toBeNull();
  });

  it("hides the raw score outside developer mode", () => {
    expect(suggestionChipFor(groups, groups[0], 0, profiles, false)!.meta).toBe("high");
  });

  it("previews this cluster's head then the merge candidate's", () => {
    const turns = [
      turn({ id: 1, clusterId: 7, startMs: 5000 }),
      turn({ id: 2, clusterId: 9, startMs: 42_000 }),
    ];
    expect(samplePreviewHeadsMs(groups[0], turns)).toEqual([5000, 42_000]);
    // no merge candidate → ONE preview, this cluster's own head
    expect(samplePreviewHeadsMs({ ...groups[2], clusterId: 7 }, turns)).toEqual([5000]);
    // a merge candidate with no loaded turn yields one head, not a hole
    expect(samplePreviewHeadsMs(groups[0], [turns[0]])).toEqual([5000]);
    // neither this cluster nor a candidate has a turn → nothing to play
    expect(samplePreviewHeadsMs(groups[2], turns)).toEqual([]);
  });
});

describe("buildSpeakerGroups", () => {
  it("merges only CONSECUTIVE turns of one cluster", () => {
    const groups = buildSpeakerGroups(
      [
        turn({ id: 1, clusterId: 7, startMs: 0, endMs: 5000, transcriptText: "hello" }),
        // a provider that hands back a shorter tail must not SHRINK the paragraph:
        // endMs bounds both the karaoke range and the seek clamp.
        turn({ id: 2, clusterId: 7, startMs: 1000, endMs: 2000, transcriptText: "again" }),
        turn({ id: 3, clusterId: 9, startMs: 5000, endMs: 6000, transcriptText: "hi" }),
        turn({ id: 4, clusterId: 7, startMs: 6000, endMs: 7000, transcriptText: "back" }),
      ],
      [],
    );
    // cluster 7 recurring AFTER 9 opens a new paragraph; folding it back into the
    // first would silently reorder the transcript.
    expect(groups.map((g) => [g.clusterId, g.text])).toEqual([
      [7, "hello again"],
      [9, "hi"],
      [7, "back"],
    ]);
    expect(groups[0].turnIds).toEqual([1, 2]);
    expect(groups[0].endMs).toBe(5000);
    expect(groups[2].startMs).toBe(6000);
  });

  it("drops turns with no transcript text", () => {
    const groups = buildSpeakerGroups(
      [
        turn({ id: 1, clusterId: 7, transcriptText: "hello" }),
        turn({ id: 2, clusterId: 9, transcriptText: "   " }),
        turn({ id: 3, clusterId: 4, transcriptText: null }),
        turn({ id: 4, clusterId: 7, transcriptText: "world" }),
      ],
      [],
    );
    // A wordless cluster is not a speaker, so the two cluster-7 turns become
    // adjacent and merge across the gap.
    expect(groups).toHaveLength(1);
    expect(groups[0].text).toBe("hello world");
    expect(groups[0].turnIds).toEqual([1, 4]);
  });

  it("OR-propagates overlaps forward on merge and never backwards", () => {
    const merged = buildSpeakerGroups(
      [
        turn({ id: 1, clusterId: 7, overlaps: false, transcriptText: "a" }),
        turn({ id: 2, clusterId: 7, overlaps: true, transcriptText: "b" }),
      ],
      [],
    );
    expect(merged[0].overlaps).toBe(true);
    // a merge never CLEARS a flag already set
    const sticky = buildSpeakerGroups(
      [
        turn({ id: 1, clusterId: 7, overlaps: true, transcriptText: "a" }),
        turn({ id: 2, clusterId: 7, overlaps: false, transcriptText: "b" }),
      ],
      [],
    );
    expect(sticky[0].overlaps).toBe(true);
    // …and a later paragraph's overlap must not mark the earlier one
    const later = buildSpeakerGroups(
      [
        turn({ id: 1, clusterId: 7, overlaps: false, transcriptText: "a" }),
        turn({ id: 2, clusterId: 9, overlaps: true, transcriptText: "b" }),
      ],
      [],
    );
    expect(later.map((g) => g.overlaps)).toEqual([false, true]);
  });

  it("nulls the merge suggestion when the cluster row is absent", () => {
    const row = cluster({ id: 7, suggestedMergeTargetClusterId: 9, suggestedMergeScore: 0.71 });
    const [withRow] = buildSpeakerGroups([turn({ clusterId: 7 })], [row]);
    expect(withRow.suggestedMergeTargetClusterId).toBe(9);
    expect(withRow.suggestedMergeScore).toBe(0.71);
    // clusters loaded for a different session/segment must not leak a suggestion
    const [without] = buildSpeakerGroups([turn({ clusterId: 7 })], [{ ...row, id: 42 }]);
    expect(without.suggestedMergeTargetClusterId).toBeNull();
    expect(without.suggestedMergeScore).toBeNull();
  });
});

describe("activeKaraokeIndex tolerance", () => {
  // The literal edges below encode the 150 ms default on purpose: writing them as
  // `KARAOKE_TOLERANCE_MS ± 1` would pass for any value the constant ever takes.
  const lone = [{ text: "solo", startMs: 1000, endMs: 2000 }];

  it("lights a word 150ms early and holds it 150ms past its end", () => {
    expect(activeKaraokeIndex(lone, 849)).toBe(-1);
    expect(activeKaraokeIndex(lone, 850)).toBe(0);
    expect(activeKaraokeIndex(lone, 2150)).toBe(0);
    expect(activeKaraokeIndex(lone, 2151)).toBe(-1);
  });

  it("goes dark mid-gap and hands over exactly 150ms before the next word", () => {
    const pair = [
      { text: "first", startMs: 0, endMs: 1000 },
      { text: "second", startMs: 2000, endMs: 3000 },
    ];
    expect(activeKaraokeIndex(pair, 1150)).toBe(0);
    expect(activeKaraokeIndex(pair, 1151)).toBe(-1);
    expect(activeKaraokeIndex(pair, 1849)).toBe(-1);
    expect(activeKaraokeIndex(pair, 1850)).toBe(1);
  });

  it("honours an explicit tolerance, and has nothing to light with no words", () => {
    expect(activeKaraokeIndex(lone, 999, 0)).toBe(-1);
    expect(activeKaraokeIndex(lone, 1000, 0)).toBe(0);
    expect(activeKaraokeIndex(lone, 2000, 0)).toBe(0);
    expect(activeKaraokeIndex(lone, 2001, 0)).toBe(-1);
    expect(activeKaraokeIndex([], 500)).toBe(-1);
  });
});

describe("karaokeForGroup", () => {
  it("degrades a paragraph another speaker's overlapping words leak into", () => {
    // Cross-cluster overlap is real diarizer output (`SpeakerTurnDto.overlaps`, set by
    // mark_cross_cluster_overlaps), typically a short backchannel nested inside a long
    // turn. The backend gives each word to exactly ONE turn by maximum overlap
    // (best_turn_for_timed_text_run), so the nested cluster owns "mm hmm" and the long
    // turn's text does not contain it. Karaoke REPLACES the paragraph's text with the
    // words it picks, so returning the nested cluster's words here prints one speaker's
    // words under another speaker's name — and a second time in their own paragraph.
    const overlapping: TranscriptionWord[] = [
      { startMs: 0, endMs: 1000, text: "so" },
      { startMs: 1000, endMs: 2000, text: "anyway" },
      { startMs: 2500, endMs: 3000, text: "mm" },
      { startMs: 3000, endMs: 3500, text: "hmm" },
      { startMs: 5000, endMs: 6000, text: "right" },
    ];
    const host = group({ clusterId: 1, startMs: 0, endMs: 10_000, text: "so anyway right" });
    expect(karaokeForGroup(overlapping, host)).toBeNull();
    // ...while the nested paragraph still karaokes the words it does own.
    const nested = group({ clusterId: 2, startMs: 2000, endMs: 4000, text: "mm hmm" });
    expect(karaokeForGroup(overlapping, nested)?.map((w) => w.text)).toEqual(["mm", "hmm"]);
  });

  it("keeps karaoke for a script whose word tokens are shorter than the join space", () => {
    // Guard against an over-broad upper bound: the " " join makes the joined length
    // LONGER than the paragraph for per-character tokens, which must not read as a leak.
    const cjk: TranscriptionWord[] = [
      { startMs: 0, endMs: 200, text: "\u4f60" },
      { startMs: 200, endMs: 400, text: "\u597d" },
      { startMs: 400, endMs: 600, text: "\u4e16" },
      { startMs: 600, endMs: 800, text: "\u754c" },
    ];
    expect(
      karaokeForGroup(cjk, group({ startMs: 0, endMs: 1000, text: "\u4f60\u597d\u4e16\u754c" })),
    ).toHaveLength(4);
  });

  const words: TranscriptionWord[] = [
    { startMs: 0, endMs: 400, text: "one" },
    { startMs: 400, endMs: 900, text: "two" },
    { startMs: 900, endMs: 1400, text: "three" },
  ];

  it("karaokes a paragraph its words actually cover", () => {
    const picked = karaokeForGroup(words, group({ startMs: 0, endMs: 2000, text: "one two three" }));
    expect(picked?.map((w) => w.text)).toEqual(["one", "two", "three"]);
  });

  it("degrades to the paragraph when the words cover too little of it", () => {
    // ~20% covered: rendering only these would drop most of the text off the page.
    const sparse = group({
      startMs: 0,
      endMs: 2000,
      text: "one two three and a great deal more the provider never timed at all",
    });
    expect(karaokeForGroup(words, sparse)).toBeNull();
  });

  it("keeps karaoke at exactly the 80% coverage edge and drops it one char below", () => {
    const timed: TranscriptionWord[] = [
      { startMs: 0, endMs: 400, text: "abcd" },
      { startMs: 400, endMs: 900, text: "efg" },
    ];
    // joined = "abcd efg" = 8 chars
    expect(karaokeForGroup(timed, group({ startMs: 0, endMs: 2000, text: "0123456789" }))).toHaveLength(2);
    expect(karaokeForGroup(timed, group({ startMs: 0, endMs: 2000, text: "01234567890" }))).toBeNull();
  });

  it("degrades with no words at all and with none in this paragraph's range", () => {
    expect(karaokeForGroup([], group({ text: "one two three" }))).toBeNull();
    expect(karaokeForGroup(words, group({ startMs: 9000, endMs: 10_000, text: "elsewhere" }))).toBeNull();
  });
});

describe("activeGroupIndex", () => {
  const groups = [
    group({ clusterId: 7, startMs: 0, endMs: 1000 }),
    group({ clusterId: 9, startMs: 5000, endMs: 6000 }),
  ];

  it("highlights nothing until playback has actually moved", () => {
    expect(activeGroupIndex(groups, 5500, false)).toBeNull();
    expect(activeGroupIndex(groups, 5500, true)).toBe(1);
    expect(activeGroupIndex([], 5500, true)).toBeNull();
    expect(activeGroupIndex([group({ startMs: 2000 })], 1999, true)).toBeNull();
    expect(activeGroupIndex([group({ startMs: 2000 })], 2000, true)).toBe(0);
  });

  it("picks the last paragraph already started — and keeps it lit past its end", () => {
    // Start-bounded by design: the paragraph highlight is the reader's position
    // anchor, so it holds through the silence after a paragraph and stays on the
    // last one past the final word. Only `activeKaraokeIndex` blanks.
    expect(activeGroupIndex(groups, 4999, true)).toBe(0); // 4s past group 0's endMs
    expect(activeGroupIndex(groups, 999_999, true)).toBe(1);
  });
});

describe("speaker labels", () => {
  it("strips a leading Maybe and recognises the diarizer's placeholder label", () => {
    expect(speakerCleanLabel("Maybe Priya")).toBe("Priya");
    expect(speakerCleanLabel("maybe   Priya  ")).toBe("Priya");
    expect(speakerCleanLabel("Priya")).toBe("Priya");
    expect(speakerCleanLabel("Not Maybe Priya")).toBe("Not Maybe Priya");
    // speakrs emits "Unknown Speaker N" (crates/speaker-analysis/.../speakrs.rs)
    expect(isDefaultSpeakerLabel("Unknown Speaker 1")).toBe(true);
    expect(isDefaultSpeakerLabel("Maybe Unknown Speaker 12")).toBe(true);
    expect(isDefaultSpeakerLabel("Priya")).toBe(false);
  });

  it("falls back to the cluster label when the linked profile is gone", () => {
    expect(speakerProfileName(PROFILES, null)).toBeNull();
    expect(speakerProfileName(PROFILES, 2)).toBe("Daniel Okafor");
    expect(speakerProfileName(PROFILES, 404)).toBeNull();
    // a person deleted out from under a still-linked cluster must not blank the gutter
    expect(speakerPersistedName(group({ personId: 404, speakerLabel: "Maybe Priya" }), PROFILES))
      .toBe("Priya");
    expect(speakerPersistedName(group({ personId: 1 }), PROFILES)).toBe("Priya");
    expect(speakerPersistedName(group({ speakerLabel: "Unknown Speaker 1" }), PROFILES))
      .toBe("Unknown Speaker 1");
    expect(
      speakerSuggestedPersonName(
        group({ suggestedPersonId: 404, speakerLabel: "Maybe Priya" }),
        PROFILES,
      ),
    ).toBe("Priya");
  });

  it("calls only an unlinked placeholder unnamed", () => {
    expect(speakerIsUnnamed(group({ speakerLabel: "Unknown Speaker 1" }), PROFILES)).toBe(true);
    expect(speakerIsUnnamed(group({ speakerLabel: "Priya" }), PROFILES)).toBe(false);
    // a link wins even while the label is still the placeholder
    expect(speakerIsUnnamed(group({ personId: 1, speakerLabel: "Unknown Speaker 1" }), PROFILES))
      .toBe(false);
  });

  it("labels a cluster option linked > suggested > raw, tolerating a dangling id", () => {
    expect(speakerClusterOptionLabel(cluster({ personId: 1, suggestedPersonId: 2 }), PROFILES))
      .toBe("Priya");
    expect(speakerClusterOptionLabel(cluster({ personId: 404, speakerLabel: "Priya" }), PROFILES))
      .toBe("Priya");
    expect(speakerClusterOptionLabel(cluster({ suggestedPersonId: 2 }), PROFILES))
      .toBe("Daniel Okafor");
    expect(
      speakerClusterOptionLabel(
        cluster({ suggestedPersonId: 404, speakerLabel: "Unknown Speaker 3" }),
        PROFILES,
      ),
    ).toBe("Maybe Unknown Speaker 3");
    expect(speakerClusterOptionLabel(cluster({ speakerLabel: "Maybe Priya" }), PROFILES))
      .toBe("Priya");
  });

  // Owner-only auto-linking (migration 0053) names a speaker with nobody
  // confirming it. Settings states the guarantee outright — "Automatic labels are
  // marked as automatic and can be undone" — so a label that reads identically to
  // one the user set themselves makes that copy false and leaves an unconfirmed
  // biometric match unauditable.
  it("marks an auto-applied name as automatic, and leaves a confirmed one alone", () => {
    expect(speakerClusterOptionLabel(cluster({ personId: 1, personLinkAuto: true }), PROFILES))
      .toBe("Priya (auto)");
    expect(speakerClusterOptionLabel(cluster({ personId: 1, personLinkAuto: false }), PROFILES))
      .toBe("Priya");
    // The marker follows the link, not the guess: a mere suggestion is already
    // phrased as one and nobody applied it.
    expect(
      speakerClusterOptionLabel(cluster({ suggestedPersonId: 2, personLinkAuto: true }), PROFILES),
    ).toBe("Daniel Okafor");
  });

  it("never invents a name for a merge target that no longer exists", () => {
    const target = cluster({ id: 9, personId: 2 });
    expect(suggestedMergeTargetLabel(group(), [target], PROFILES)).toBeNull();
    expect(
      suggestedMergeTargetLabel(group({ suggestedMergeTargetClusterId: 9 }), [target], PROFILES),
    ).toBe("Daniel Okafor");
    // the target was merged away or deleted: no label beats a wrong one
    expect(
      suggestedMergeTargetLabel(
        group({ suggestedMergeTargetClusterId: 9 }),
        [cluster({ id: 3 })],
        PROFILES,
      ),
    ).toBeNull();
  });
});

describe("formatScore", () => {
  it("returns null for anything unscorable", () => {
    expect(formatScore(null)).toBeNull();
    expect(formatScore(undefined)).toBeNull();
    expect(formatScore(Number.NaN)).toBeNull();
    expect(formatScore(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("renders the cosine as-is, including a float epsilon past 1", () => {
    expect(formatScore(0)).toBe("0.00");
    expect(formatScore(0.88)).toBe("0.88");
    expect(formatScore(1)).toBe("1.00");
    expect(formatScore(1.0000001)).toBe("1.00");
  });

  it("does not rescale an out-of-range score into a plausible-looking fraction", () => {
    expect(formatScore(71)).toBe("71.00");
  });
});

describe("parseSpeakerAnalysisProvenance", () => {
  it("reads the provenance, and returns null on anything else instead of throwing", () => {
    const payload = JSON.stringify({
      metadata: { provenance: { skipReason: "silent", audioPeak: 0.004 } },
    });
    expect(parseSpeakerAnalysisProvenance(payload)).toEqual({
      skipReason: "silent",
      audioPeak: 0.004,
    });
    expect(parseSpeakerAnalysisProvenance("{not json")).toBeNull();
    expect(parseSpeakerAnalysisProvenance("{}")).toBeNull();
    expect(parseSpeakerAnalysisProvenance(JSON.stringify({ metadata: null }))).toBeNull();
    expect(parseSpeakerAnalysisProvenance(null)).toBeNull();
    expect(parseSpeakerAnalysisProvenance("")).toBeNull();
  });
});

describe("time formatting", () => {
  it("formats M:SS and never shows a negative or NaN clock", () => {
    expect(formatPlayerTime(0)).toBe("0:00");
    expect(formatPlayerTime(5)).toBe("0:05");
    expect(formatPlayerTime(65.9)).toBe("1:05");
    expect(formatPlayerTime(3661)).toBe("61:01");
    expect(formatPlayerTime(-3)).toBe("0:00");
    expect(formatPlayerTime(Number.NaN)).toBe("0:00");
  });

  it("titles a transcript run as a range, collapsing a zero-length one", () => {
    expect(formatTranscriptSegmentTitle({ startMs: 5000, endMs: 12_000 })).toBe("0:05–0:12");
    expect(formatTranscriptSegmentTitle({ startMs: 5000, endMs: 5000 })).toBe("0:05");
    expect(formatTranscriptSegmentTitle({ startMs: 5000, endMs: 1000 })).toBe("0:05");
  });
});
