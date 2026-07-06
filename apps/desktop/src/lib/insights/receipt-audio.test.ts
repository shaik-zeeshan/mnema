// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  assignSpeakerColors,
  audioFooterLeft,
  buildTurnViews,
  captionFromTurns,
  frameIndexForMs,
  isFallbackSpeaker,
  partitionEvidence,
  receiptViewState,
  sourceKindLabel,
  sourceKindReadable,
  speakerDisplay,
  turnSpeakerRoster,
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

describe("turnSpeakerRoster", () => {
  it("lists distinct speakers in turn order and nudges an unnamed voice", () => {
    const turns = [
      { speaker: "You", isFallback: false },
      { speaker: "Alice", isFallback: false },
      { speaker: "You", isFallback: false }, // repeat dropped
      { speaker: "Speaker 2", isFallback: true },
    ];
    expect(turnSpeakerRoster(turns)).toBe("You · Alice · Speaker 2 (unnamed → name in Timeline)");
  });
  it("is empty for no turns", () => {
    expect(turnSpeakerRoster([])).toBe("");
  });
});

describe("audioFooterLeft", () => {
  it("is honest about expired vs never-captured frames", () => {
    expect(audioFooterLeft(0)).toBe("0 screen frames — captured as audio");
    expect(audioFooterLeft(3)).toBe("0 screen frames — screen frames have expired");
  });
});

describe("assignSpeakerColors", () => {
  it("pins You to the audio channel lavender", () => {
    expect(assignSpeakerColors(["You"]).get("You")).toBe("--cat-communication");
  });
  it("gives other names distinct palette colors in first-appearance order", () => {
    const colors = assignSpeakerColors(["Bob", "Carol"]);
    expect(colors.get("Bob")).toBe("--cat-meetings");
    expect(colors.get("Carol")).toBe("--cat-research");
    expect(colors.get("Bob")).not.toBe(colors.get("Carol"));
  });
  it("reuses the same color for a repeated name and never spends a slot on You", () => {
    const colors = assignSpeakerColors(["Bob", "You", "Carol", "Bob"]);
    expect(colors.get("Bob")).toBe("--cat-meetings");
    expect(colors.get("You")).toBe("--cat-communication");
    expect(colors.get("Carol")).toBe("--cat-research"); // You did not consume a slot
  });
});

describe("buildTurnViews", () => {
  const micStart = "2026-07-06T10:00:00.000Z";
  const sysStart = "2026-07-06T10:10:00.000Z";
  const segMic = { id: 10, sourceKind: "microphone", startedAt: micStart, endedAt: "2026-07-06T10:05:00.000Z" };
  const segSys = { id: 20, sourceKind: "system_audio", startedAt: sysStart, endedAt: "2026-07-06T10:15:00.000Z" };
  const segments = [
    // deliberately out of order to prove the builder sorts by absolute start
    {
      segment: segSys,
      turns: [
        { id: 2, personId: 7, speakerLabel: "Unknown Speaker 2", startMs: 1000, endMs: 3000, transcriptText: "hi there" },
        { id: 3, personId: null, speakerLabel: "Unknown Speaker 3", startMs: 4000, endMs: 6000, transcriptText: "back to me" },
        // Over-cluster artifact: a real diarizer cluster with no transcribed
        // words. Must be dropped (Timeline's `if (!text) continue`), never a
        // phantom "Speaker 4".
        { id: 4, personId: null, speakerLabel: "Unknown Speaker 4", startMs: 7000, endMs: 8000, transcriptText: null },
      ],
    },
    {
      segment: segMic,
      turns: [
        { id: 1, personId: null, speakerLabel: "Unknown Speaker 1", startMs: 2000, endMs: 5000, transcriptText: "hello from mic" },
      ],
    },
  ];
  const profiles = [{ id: 7, displayName: "Bob" }];
  const citedRefs = [{ subjectId: 20, isHeadline: true }];

  it("lifts each turn to absolute epoch = segment start + in-segment offset", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    const byKey = Object.fromEntries(views.map((v) => [v.key, v]));
    expect(byKey["10:1"].startMs).toBe(Date.parse(micStart) + 2000);
    expect(byKey["10:1"].endMs).toBe(Date.parse(micStart) + 5000);
    expect(byKey["20:2"].startMs).toBe(Date.parse(sysStart) + 1000);
    expect(byKey["20:3"].endMs).toBe(Date.parse(sysStart) + 6000);
    expect(byKey["20:2"].sourceMeta).toBe("system audio");
    expect(byKey["10:1"].sourceMeta).toBe("microphone");
  });

  it("orders ascending by absolute startMs across segments", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    expect(views.map((v) => v.key)).toEqual(["10:1", "20:2", "20:3"]);
    for (let i = 1; i < views.length; i++) {
      expect(views[i].startMs).toBeGreaterThanOrEqual(views[i - 1].startMs);
    }
  });

  it("marks cited by ref-set membership and flags the headline segment's turns", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    const mic = views.find((v) => v.key === "10:1");
    const sys = views.find((v) => v.key === "20:2");
    expect(mic.cited).toBe(false); // segment 10 not cited
    expect(mic.isHeadline).toBe(false);
    expect(sys.cited).toBe(true); // segment 20 is cited
    expect(sys.isHeadline).toBe(true); // 20 is the headline ref
  });

  it("resolves speaker live: mic → You, personId → profile name, else Speaker N", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    const byKey = Object.fromEntries(views.map((v) => [v.key, v]));
    expect(byKey["10:1"].speaker).toBe("You");
    expect(byKey["10:1"].isFallback).toBe(false);
    expect(byKey["20:2"].speaker).toBe("Bob"); // personId 7 resolves live
    expect(byKey["20:3"].speaker).toBe("Speaker 3"); // unnamed fallback
    expect(byKey["20:3"].isFallback).toBe(true);
    // colors: You pinned, Bob + Speaker 3 get distinct palette entries
    expect(byKey["10:1"].colorVar).toBe("--cat-communication");
    expect(byKey["20:2"].colorVar).toBe("--cat-meetings");
    expect(byKey["20:3"].colorVar).toBe("--cat-research");
  });

  it("drops a wordless turn so an over-cluster never adds a phantom speaker", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    expect(views.find((v) => v.key === "20:4")).toBeUndefined(); // wordless → gone
    expect(views.some((v) => v.speaker === "Speaker 4")).toBe(false); // no phantom
    expect(views.find((v) => v.key === "20:3").text).toBe("back to me"); // words kept
    expect(views.find((v) => v.key === "20:2").text).toBe("hi there");
  });
});
