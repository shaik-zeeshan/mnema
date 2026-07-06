// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  assignSpeakerColors,
  audioFooterLeft,
  buildTurnViews,
  captionFromTurns,
  clipStartOffsetSec,
  frameIndexForMs,
  isFallbackSpeaker,
  partitionEvidence,
  receiptViewState,
  scheduleClipSeek,
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

describe("sourceKindReadable", () => {
  it("names the capture input (the 'via …' subtitle), not the speaker", () => {
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

  it("resolves speaker by diarized voice, not source kind: mic turns are Speaker N too", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    const byKey = Object.fromEntries(views.map((v) => [v.key, v]));
    // The mic turn is NOT forced to "You" — the mic captures the room, so it's
    // attributed by its diarized cluster like any other voice (name in Timeline).
    expect(byKey["10:1"].speaker).toBe("Speaker 1");
    expect(byKey["10:1"].isFallback).toBe(true);
    expect(byKey["20:2"].speaker).toBe("Bob"); // personId 7 resolves live
    expect(byKey["20:3"].speaker).toBe("Speaker 3"); // unnamed fallback
    expect(byKey["20:3"].isFallback).toBe(true);
    // colors cycle the palette in first-appearance order (no "You" pin in play)
    expect(byKey["10:1"].colorVar).toBe("--cat-meetings");
    expect(byKey["20:2"].colorVar).toBe("--cat-research");
    expect(byKey["20:3"].colorVar).toBe("--cat-entertainment");
  });

  it("drops a wordless turn so an over-cluster never adds a phantom speaker", () => {
    const views = buildTurnViews(segments, citedRefs, profiles);
    expect(views.find((v) => v.key === "20:4")).toBeUndefined(); // wordless → gone
    expect(views.some((v) => v.speaker === "Speaker 4")).toBe(false); // no phantom
    expect(views.find((v) => v.key === "20:3").text).toBe("back to me"); // words kept
    expect(views.find((v) => v.key === "20:2").text).toBe("hi there");
  });

  it("drops a segment whose startedAt won't parse instead of leaking NaN wall-clock turns", () => {
    // Date.parse returns NaN (never throws) on a bad timestamp; a NaN startMs would
    // survive the wordless filter (it has text) and, because `NaN > ms` is always
    // false, hijack activeKeyAt's playhead scan. Mirror loadStrip's finite guard.
    const bad = { id: 99, sourceKind: "microphone", startedAt: "not-a-date", endedAt: "also-bad" };
    const views = buildTurnViews(
      [
        {
          segment: bad,
          turns: [{ id: 1, personId: null, speakerLabel: "Speaker 1", startMs: 0, endMs: 1000, transcriptText: "ghost" }],
        },
      ],
      [],
      [],
    );
    expect(views).toEqual([]); // NaN-timed turn dropped, never surfaced
  });
});

describe("clipStartOffsetSec — Play/Space never seeks a clip past its segment", () => {
  const turn = { segmentStartMs: 10_000, endMs: 13_000 }; // a 3s turn starting 10s in

  it("returns the in-segment offset when the head sits inside the turn window", () => {
    expect(clipStartOffsetSec(turn, 11_500)).toBeCloseTo(1.5); // 1.5s into the segment
    expect(clipStartOffsetSec(turn, 10_000)).toBe(0); // exactly the segment head
    expect(clipStartOffsetSec(turn, 13_000)).toBeCloseTo(3); // the turn's end
  });

  it("returns 0 for a head outside the turn — the different-segment poster frame case", () => {
    // The frame playhead sitting in a LATER segment. The old inline math
    // (550_000 - 10_000)/1000 = 540s → scheduleClipSeek clamps to the clip's
    // duration → the clip plays AT its end → `ended` → auto-advance SKIPS the turn.
    expect(clipStartOffsetSec(turn, 550_000)).toBe(0);
    expect(clipStartOffsetSec(turn, 5_000)).toBe(0); // before the segment
  });

  it("returns 0 for a null/absent head (plain Play with no scrub target)", () => {
    expect(clipStartOffsetSec(turn, null)).toBe(0);
    expect(clipStartOffsetSec(turn, undefined)).toBe(0);
  });
});

// Faithful mini-mock of the <audio> element's listener semantics: an
// addEventListener {once} listener is removed only when it FIRES, NOT when
// `src` changes — so a pending metadata-seek from a superseded clip survives a
// src swap and fires against the LATER src. onloadedmetadata is a single-slot
// property (assigning replaces the prior handler; null clears it).
function fakeAudio() {
  const once = []; // pending addEventListener("loadedmetadata", …, {once:true})
  return {
    currentTime: 0,
    duration: 120,
    src: "",
    readyState: 0, // 0 = HAVE_NOTHING (fresh src); ≥1 = HAVE_METADATA (same-segment re-seek)
    onloadedmetadata: null,
    addEventListener(type, fn, opts) {
      if (type === "loadedmetadata") once.push({ fn, once: !!opts?.once });
    },
    // The DOM fires the on* property handler, then addEventListener listeners;
    // {once} ones are dropped after firing. Setting src never touches `once`.
    fireLoadedMetadata() {
      this.onloadedmetadata?.();
      const survivors = [];
      for (const l of once) {
        l.fn();
        if (!l.once) survivors.push(l);
      }
      once.length = 0;
      once.push(...survivors);
    },
  };
}

describe("scheduleClipSeek — stale metadata-seek listener (ADR 0049 scrub→select)", () => {
  it("does NOT seek a new clip (offset 0) to a superseded scrub's offset", () => {
    const el = fakeAudio();
    // Scrub release lands 30s into the segment → deferred seek scheduled.
    scheduleClipSeek(el, 30);
    // Before the first clip's metadata loads, the user clicks a transcript row:
    // onSelect → playClip with no seekToMs → offset 0 (start at the segment head).
    // A real <audio> keeps the still-pending {once} listener across the src swap.
    el.src = "data:audio/second-clip";
    scheduleClipSeek(el, 0);
    // The new clip's metadata finally loads.
    el.fireLoadedMetadata();
    // It must start at its head, not mid-segment at the previous clip's 30s.
    expect(el.currentTime).toBe(0);
  });

  it("still applies and clamps a genuine (un-superseded) scrub seek", () => {
    const el = fakeAudio();
    el.duration = 45;
    scheduleClipSeek(el, 30);
    el.fireLoadedMetadata();
    expect(el.currentTime).toBe(30);
    const clamped = fakeAudio();
    clamped.duration = 20;
    scheduleClipSeek(clamped, 30);
    clamped.fireLoadedMetadata();
    expect(clamped.currentTime).toBe(20); // clamped to real length
  });

  it("a later scrub replaces an earlier pending seek", () => {
    const el = fakeAudio();
    scheduleClipSeek(el, 30);
    el.src = "data:audio/second-clip";
    scheduleClipSeek(el, 5);
    el.fireLoadedMetadata();
    expect(el.currentTime).toBe(5);
  });
});

describe("scheduleClipSeek — same-segment re-seek (readyState≥1 applies now)", () => {
  it("seeks immediately when metadata is already loaded, with NO loadedmetadata event", () => {
    const el = fakeAudio();
    el.readyState = 1; // HAVE_METADATA — a same-segment re-seek never re-fires loadedmetadata
    scheduleClipSeek(el, 30);
    expect(el.currentTime).toBe(30); // applied synchronously, not deferred
    expect(el.onloadedmetadata).toBeNull(); // no pending deferred seek left armed
  });

  it("clamps an immediate (readyState≥1) seek to the real duration", () => {
    const el = fakeAudio();
    el.readyState = 1;
    el.duration = 20;
    scheduleClipSeek(el, 30);
    expect(el.currentTime).toBe(20);
  });

  it("still defers to loadedmetadata when metadata isn't loaded yet (fresh src)", () => {
    const el = fakeAudio(); // readyState 0
    scheduleClipSeek(el, 30);
    expect(el.currentTime).toBe(0); // not applied until metadata is in
    el.fireLoadedMetadata();
    expect(el.currentTime).toBe(30);
  });
});
