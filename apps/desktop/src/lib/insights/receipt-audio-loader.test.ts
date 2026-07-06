// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { ReceiptAudioLoader } from "./receipt-audio-loader";

describe("ReceiptAudioLoader.loadSpan — invalid span", () => {
  it("degrades to an empty turn set instead of rejecting on a NaN span", async () => {
    // A corrupt / missing activity timestamp reaches loadSpan as NaN, exactly as
    // it reaches loadStrip. loadStrip wraps the identical
    // `new Date(NaN).toISOString()` throw in its try and renders the honest empty
    // state; loadSpan must be symmetric — never leave an unhandled RangeError that
    // strands the audio evidence at "Loading spoken evidence…".
    let turns = "never";
    const loader = new ReceiptAudioLoader(
      { onProfiles: () => {}, onTurns: (t) => (turns = t) },
      async () => [],
    );
    await expect(loader.loadSpan(NaN, NaN, [])).resolves.toBeUndefined();
    expect(turns).toEqual([]); // honest empty, like loadStrip's expired panel
  });
});

describe("ReceiptAudioLoader.loadSpan — per-segment turn fan-out is bounded", () => {
  // A multi-hour Activity spans dozens of 5-min audio segments (CLAUDE.md caps
  // Capture Segment Duration at 5 min; a mic+system call doubles that). loadSpan
  // fetches every segment's speaker turns; the fan-out must NOT open one
  // list_speaker_turns IPC per segment all at once against the 4-connection
  // owner-reader pool (shared with live capture/OCR reads).
  async function measureLoadSpan(segmentCount: number) {
    // ~5h of mic-only capture at the 5-min segment cap.
    const segments = Array.from({ length: segmentCount }, (_, i) => ({
      id: i + 1,
      startedAt: new Date(1_000_000 + i * 300_000).toISOString(),
      sourceKind: "microphone",
    }));

    let turnsCalls = 0;
    let inFlight = 0;
    let maxInFlight = 0;
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));

    const invoke = async (cmd: string) => {
      if (cmd === "list_person_profiles") return [];
      if (cmd === "list_audio_segments") return segments;
      if (cmd === "list_speaker_turns") {
        turnsCalls++;
        inFlight++;
        maxInFlight = Math.max(maxInFlight, inFlight);
        await gate; // hold every issued call open so real overlap is observable
        inFlight--;
        return [];
      }
      return [];
    };

    const loader = new ReceiptAudioLoader(
      { onProfiles: () => {}, onTurns: () => {} },
      invoke,
    );
    const done = loader.loadSpan(0, segmentCount * 300_000, []);
    // Let loadSpan get past the profiles + segments awaits and issue the whole
    // per-segment fan-out before anything is allowed to resolve.
    await new Promise((r) => setTimeout(r, 0));
    release();
    await done;
    return { turnsCalls, maxInFlight, segmentCount };
  }

  it("issues exactly one list_speaker_turns per segment (the N in N+1)", async () => {
    const { turnsCalls } = await measureLoadSpan(60);
    expect(turnsCalls).toBe(60);
  });

  it("caps concurrent list_speaker_turns IPC instead of bursting all N at once", async () => {
    const { maxInFlight, segmentCount } = await measureLoadSpan(60);
    // Unbounded Promise.all opens all 60 at once (maxInFlight === 60). A bounded
    // fan-out keeps the burst small so it can't monopolize the 4-connection
    // reader pool shared with background capture reads.
    expect(maxInFlight).toBeLessThanOrEqual(8);
    expect(maxInFlight).toBeLessThan(segmentCount);
  });
});

describe("ReceiptAudioLoader.loadSpan — happy-path profiles→turns wiring", () => {
  it("reports the shared profiles then a built TurnView[] with live-resolved speaker", async () => {
    // Locks the positive wiring the gen-guard test only checks the negative of:
    // list_person_profiles → onProfiles, then every segment's list_speaker_turns
    // hydrated into buildTurnViews → onTurns. A real profile whose id matches the
    // turn's personId must live-resolve the diarized voice to the profile name.
    const profile = {
      id: 7,
      displayName: "Ada",
      notes: null,
      embeddingCount: 3,
      createdAt: "2026-07-06T00:00:00.000Z",
      updatedAt: "2026-07-06T00:00:00.000Z",
    };
    const segment = {
      id: 42,
      sourceKind: "microphone",
      sourceSessionId: "sess-1",
      segmentIndex: 0,
      filePath: "/tmp/seg.mov",
      startedAt: "2026-07-06T10:00:00.000Z",
      endedAt: "2026-07-06T10:05:00.000Z",
      createdAt: "2026-07-06T10:00:00.000Z",
      updatedAt: "2026-07-06T10:00:00.000Z",
    };
    const turn = {
      id: 3,
      audioSegmentId: 42,
      sessionId: "sess-1",
      clusterId: 1,
      segmentClusterId: null,
      providerClusterId: "c1",
      speakerLabel: "Speaker 1",
      personId: 7, // matches profile.id → speaker resolves to "Ada"
      suggestedPersonId: null,
      recognitionConfidence: null,
      recognitionScore: null,
      startMs: 1500,
      endMs: 4500,
      transcriptText: "hello world",
      overlaps: false,
    };

    const invoke = async (cmd: string) => {
      if (cmd === "list_person_profiles") return [profile];
      if (cmd === "list_audio_segments") return [segment];
      if (cmd === "list_speaker_turns") return [turn];
      return [];
    };

    let seenProfiles: unknown = "never";
    let seenTurns: unknown = "never";
    const loader = new ReceiptAudioLoader(
      { onProfiles: (p) => (seenProfiles = p), onTurns: (t) => (seenTurns = t) },
      invoke,
    );
    await loader.loadSpan(
      Date.parse(segment.startedAt),
      Date.parse(segment.endedAt),
      [{ subjectId: 42, isHeadline: true }],
    );

    expect(seenProfiles).toEqual([profile]); // onProfiles gets the real directory

    const turns = seenTurns as { key: string; startMs: number; speaker: string }[];
    expect(turns).toHaveLength(1);
    expect(turns[0].key).toBe("42:3"); // `${segId}:${turnId}`
    expect(turns[0].startMs).toBe(Date.parse(segment.startedAt) + turn.startMs);
    expect(turns[0].speaker).toBe("Ada"); // live-resolved from personId → profile
  });
});

describe("ReceiptAudioLoader.fetchMediaSrc", () => {
  it("builds a data: URL from the fetched media (matches audioDataUrl)", async () => {
    const media = { mimeType: "audio/mp4", dataBase64: "QUJD" };
    const invoke = async (cmd: string) => {
      if (cmd === "get_audio_segment_media") return media;
      return [];
    };
    const loader = new ReceiptAudioLoader({ onProfiles: () => {} }, invoke);
    const src = await loader.fetchMediaSrc(42);
    expect(src).toBe("data:audio/mp4;base64,QUJD");
  });

  it("returns null when the media fetch rejects (expired / deleted clip)", async () => {
    // Lock, not a repro: the null path is already correct. A cited segment whose
    // media has aged out of retention rejects get_audio_segment_media; the receipt
    // must surface null (the expired-clip UI) rather than throw.
    const invoke = async (cmd: string) => {
      if (cmd === "get_audio_segment_media") throw new Error("media expired");
      return [];
    };
    const loader = new ReceiptAudioLoader({ onProfiles: () => {} }, invoke);
    await expect(loader.fetchMediaSrc(42)).resolves.toBeNull();
  });
});

describe("ReceiptAudioLoader.loadSpan — generation guard (rapid activity switch)", () => {
  it("a superseded hydration drops its onTurns AND sheds its turn fan-out", async () => {
    // The user reopens a different Activity mid-hydration. The stale run must not
    // (a) emit onTurns — that would clobber the new activity's turns (invariant
    // #4) — nor (b) still fan out one list_speaker_turns per segment against the
    // shared 4-connection reader pool only to discard the results.
    let turnsCalls = 0;
    let onTurnsCalls = 0;
    let releaseSegments: (v: unknown) => void = () => {};
    const invoke = async (cmd: string) => {
      if (cmd === "list_person_profiles") return [];
      // Park the in-flight run here so we can supersede it before it fans out.
      if (cmd === "list_audio_segments") return new Promise((r) => (releaseSegments = r));
      if (cmd === "list_speaker_turns") {
        turnsCalls++;
        return [];
      }
      return [];
    };
    const loader = new ReceiptAudioLoader(
      { onProfiles: () => {}, onTurns: () => onTurnsCalls++ },
      invoke,
    );
    const stale = loader.loadSpan(0, 1000, []); // parks awaiting list_audio_segments
    await new Promise((r) => setTimeout(r, 0));
    loader.reset(); // a newer activity supersedes the in-flight run
    releaseSegments([{ id: 1, startedAt: new Date(0).toISOString(), sourceKind: "microphone" }]);
    await stale;
    expect(onTurnsCalls).toBe(0); // #4: stale turns never clobber the new activity
    expect(turnsCalls).toBe(0); // #6: the superseded fan-out is shed, not run then discarded
  });
});
