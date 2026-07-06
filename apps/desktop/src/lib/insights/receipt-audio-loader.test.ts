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
