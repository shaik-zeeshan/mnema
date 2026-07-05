// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  audioFooterLeft,
  audioSpeakerSummary,
  audioTickViews,
  captionFromTurns,
  frameIndexForMs,
  isFallbackSpeaker,
  partitionEvidence,
  receiptViewState,
  sourceKindLabel,
  sourceKindReadable,
  speakerDisplay,
} from "./receipt-audio";

const evidence = [
  { subjectType: "frame", subjectId: 1, isHeadline: false },
  { subjectType: "audio_segment", subjectId: 2, capturedAtMs: 1000, isHeadline: true },
  { subjectType: "frame", subjectId: 3, isHeadline: true },
  { subjectType: "audio_segment", subjectId: 4, capturedAtMs: 2000, isHeadline: false },
];

describe("partitionEvidence", () => {
  it("splits frame refs from audio_segment refs, preserving order", () => {
    const { frames, audio } = partitionEvidence(evidence);
    expect(frames.map((e) => e.subjectId)).toEqual([1, 3]);
    expect(audio.map((e) => e.subjectId)).toEqual([2, 4]);
  });
  it("ignores unknown subject types", () => {
    const { frames, audio } = partitionEvidence([{ subjectType: "other", subjectId: 9, isHeadline: false }]);
    expect(frames).toEqual([]);
    expect(audio).toEqual([]);
  });
});

describe("frameIndexForMs", () => {
  const ms = [100, 200, 300, 400];
  it("returns the nearest frame at or before the target", () => {
    expect(frameIndexForMs(ms, 250)).toBe(1); // 200 is the last ≤ 250
    expect(frameIndexForMs(ms, 300)).toBe(2); // exact hit
    expect(frameIndexForMs(ms, 399)).toBe(2);
  });
  it("clamps below the first frame and above the last", () => {
    expect(frameIndexForMs(ms, 50)).toBe(0);
    expect(frameIndexForMs(ms, 9999)).toBe(3);
  });
  it("is 0 for an empty strip", () => {
    expect(frameIndexForMs([], 123)).toBe(0);
  });
});

describe("receiptViewState", () => {
  it("frames whenever any frame is on disk", () => {
    expect(receiptViewState(5, 0)).toBe("frames");
    expect(receiptViewState(5, 3)).toBe("frames");
  });
  it("audio-only when no frames but audio was cited", () => {
    expect(receiptViewState(0, 2)).toBe("audio-only");
  });
  it("expired when neither frames nor audio remain", () => {
    expect(receiptViewState(0, 0)).toBe("expired");
  });
});

describe("source kind labels", () => {
  it("maps microphone → You and system_audio → Other side", () => {
    expect(sourceKindLabel("microphone")).toBe("You");
    expect(sourceKindLabel("system_audio")).toBe("Other side");
    expect(sourceKindReadable("microphone")).toBe("microphone");
    expect(sourceKindReadable("system_audio")).toBe("system audio");
  });
});

describe("speakerDisplay", () => {
  const profiles = [{ id: 7, displayName: "Alice" }];
  it("resolves a personId to its live profile displayName", () => {
    expect(speakerDisplay({ personId: 7, speakerLabel: "Unknown Speaker 1" }, profiles)).toBe("Alice");
  });
  it("falls back to a cleaned Speaker N for null / unknown-speaker labels", () => {
    expect(speakerDisplay({ personId: null, speakerLabel: "Unknown Speaker 2" }, profiles)).toBe("Speaker 2");
    expect(speakerDisplay({ personId: 99, speakerLabel: "Unknown Speaker 3" }, profiles)).toBe("Speaker 3");
  });
  it("does not treat a default profile name as a real name", () => {
    const dflt = [{ id: 7, displayName: "Unknown Speaker 4" }];
    expect(speakerDisplay({ personId: 7, speakerLabel: "Unknown Speaker 4" }, dflt)).toBe("Speaker 4");
  });
  it("isFallbackSpeaker flags the anonymous form", () => {
    expect(isFallbackSpeaker("Speaker 2")).toBe(true);
    expect(isFallbackSpeaker("Alice")).toBe(false);
  });
});

describe("captionFromTurns", () => {
  it("joins non-null transcripts", () => {
    expect(
      captionFromTurns([{ transcriptText: "hello" }, { transcriptText: null }, { transcriptText: "world" }]),
    ).toBe("hello world");
  });
  it("truncates past the cap with an ellipsis", () => {
    const long = "x".repeat(300);
    const out = captionFromTurns([{ transcriptText: long }], 10);
    expect(out.length).toBe(10);
    expect(out.endsWith("…")).toBe(true);
  });
  it("is empty when no segment has a transcript", () => {
    expect(captionFromTurns([{ transcriptText: null }])).toBe("");
  });
});

describe("audio view models", () => {
  const citations = [
    {
      audioSegmentId: 2,
      capturedAtMs: 1500,
      isHeadline: true,
      sourceKind: "microphone",
      startMs: 1500,
      endMs: 1800,
      turns: [{ personId: null, speakerLabel: "Unknown Speaker 1", transcriptText: "hi" }],
      caption: "hi",
    },
    {
      audioSegmentId: 4,
      capturedAtMs: 2500,
      isHeadline: false,
      sourceKind: "system_audio",
      startMs: 2500,
      endMs: 2900,
      turns: [{ personId: 7, speakerLabel: "Unknown Speaker 2", transcriptText: "yo" }],
      caption: "yo",
    },
  ];
  const profiles = [{ id: 7, displayName: "Alice" }];

  it("positions ticks by captured start across the span", () => {
    const ticks = audioTickViews(citations, profiles, 1000, 3000);
    expect(ticks.map((t) => t.pos)).toEqual([0.25, 0.75]);
    expect(ticks[0].headline).toBe(true);
    expect(ticks[0].speaker).toBe("You");
    expect(ticks[1].speaker).toBe("Alice");
  });

  it("summarizes the footer roster from mic + named/unnamed system speakers", () => {
    expect(audioSpeakerSummary(citations, profiles)).toBe("You · Alice");
    const unnamed = [{ ...citations[1], turns: [{ personId: null, speakerLabel: "Unknown Speaker 2" }] }];
    expect(audioSpeakerSummary(unnamed, profiles)).toBe("Speaker 2 (unnamed → name in Timeline)");
  });

  it("footer left copy is honest about expired vs never-captured frames", () => {
    expect(audioFooterLeft(0)).toBe("0 screen frames — captured as audio");
    expect(audioFooterLeft(3)).toBe("0 screen frames — screen frames have expired");
  });
});
