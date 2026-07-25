// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  activeKaraokeIndex,
  assignSpeakerMarks,
  clampSeekMs,
  segmentSeekMs,
  wordSeekMs,
  drawerPanelKind,
  drawerStatusPill,
  transcriptFallbackGroups,
  clusterSummaryLabel,
  embeddingCountLabel,
  karaokeWordsForRange,
  processingStageLabel,
  provenanceFootnote,
  queuedAgoLabel,
  samplePreviewHeadsMs,
  suggestionChipFor,
  validateSpeakerName,
  waveformBars,
} from "./audio-drawer-view";
import type { SpeakerTurnDto, TranscriptionWord } from "$lib/types/app-infra";

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

  it("cycles past four clusters instead of blanking", () => {
    const marks = assignSpeakerMarks([1, 2, 3, 4, 5]);
    expect(marks.get(5)!.shape).toBe(marks.get(1)!.shape);
    expect(marks.get(5)!.colorVar).toBe(marks.get(1)!.colorVar);
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
    expect(clampSeekMs(6200, 4000, 9000)).toBe(6200);
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

  it("prefers the in-flight speaker pass over everything else", () => {
    expect(drawerStatusPill({ ...base, speakerAnalysisRunning: true }).busy).toBe(true);
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
    // no merge candidate → one preview
    expect(samplePreviewHeadsMs(groups[2], turns)).toEqual([]);
  });
});
