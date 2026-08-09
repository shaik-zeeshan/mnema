// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
//
// Ports the `console.assert` self-check from
// `docs/onboarding/mockups/input-components/parts/switches.part.html` into real
// tests: the 270 MB/day anchor, the manifest download sums, and the cost deltas
// the preview strip prints.
import { describe, expect, it } from "bun:test";
import {
  ANCHOR_SHARE_MB,
  DEFAULT_EMBED_DIMS,
  costDelta,
  featureCost,
  framesPerDay,
  frameVectorMb,
} from "./feature-cost";
import { normalizeFeatures, applyToggle, preview } from "./feature-rules";
import { ANCHOR_INTERVAL_S, estimateDailyStorageMb } from "./disk-estimate";
import { CAPTURE_INTERVAL_LADDER_S } from "../components/capture-rate";
import { NOMIC_BYTES, SPEAKRS_BYTES, WHISPER_BASE_BYTES } from "./resolve-setup";

function state(overrides = {}) {
  const { permissions, ...rest } = overrides;
  return normalizeFeatures({
    permissions: { screen: true, microphone: true, systemAudio: true, ...(permissions ?? {}) },
    screen: true,
    microphone: true,
    systemAudio: true,
    ocr: true,
    transcription: true,
    speakerSeparation: true,
    semanticSearch: false,
    aiFeatures: false,
    privacy: true,
    transcribeMicrophone: false,
    transcribeSystemAudio: false,
    recognizeSavedPeople: false,
    ...rest,
  });
}

/** The exact set the 270 MB/day anchor was measured with. */
const anchorCtx = { captureIntervalSeconds: ANCHOR_INTERVAL_S };

describe("the measured anchor", () => {
  it("decomposes into shares that sum back to 270", () => {
    const total =
      ANCHOR_SHARE_MB.screen +
      ANCHOR_SHARE_MB.ocr +
      ANCHOR_SHARE_MB.audioSource * 2 +
      ANCHOR_SHARE_MB.transcript * 2;
    expect(total).toBe(270);
  });

  it("reproduces 270 MB/day for the set it was measured with", () => {
    // Screen + OCR + both audio sources + transcription, no embeddings.
    expect(featureCost(state(), anchorCtx).diskMbPerDay).toBeCloseTo(270, 6);
  });

  it("keeps summing to estimateDailyStorageMb across the whole ladder", () => {
    for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
      expect(
        featureCost(state(), { captureIntervalSeconds: intervalS }).diskMbPerDay,
      ).toBeCloseTo(estimateDailyStorageMb(intervalS), 5);
    }
  });

  it("defaults to the 2 s ladder default when no interval is given", () => {
    expect(featureCost(state()).diskMbPerDay).toBeCloseTo(405, 5);
  });
});

describe("featureCost — disk per row", () => {
  it("charges OCR nothing while screen capture is off", () => {
    const off = applyToggle(state(), "screen");
    const cost = featureCost(off, anchorCtx);
    expect(cost.diskByFeature.screen).toBe(0);
    expect(cost.diskByFeature.ocr).toBe(0);
  });

  it("charges one transcript per audio source", () => {
    const both = featureCost(state(), anchorCtx).diskByFeature.transcription;
    const one = featureCost(state({ systemAudio: false }), anchorCtx).diskByFeature
      .transcription;
    expect(both).toBeCloseTo(ANCHOR_SHARE_MB.transcript * 2, 6);
    expect(one).toBeCloseTo(ANCHOR_SHARE_MB.transcript, 6);
  });

  it("charges nothing for diarization, AI features or privacy", () => {
    const cost = featureCost(state({ semanticSearch: true, aiFeatures: true }), anchorCtx);
    expect(cost.diskByFeature.speakerSeparation).toBe(0);
    expect(cost.diskByFeature.aiFeatures).toBe(0);
    expect(cost.diskByFeature.privacy).toBe(0);
  });

  it("puts Semantic Search on top of the anchor, never inside it", () => {
    const base = featureCost(state(), anchorCtx).diskMbPerDay;
    const withSearch = featureCost(state({ semanticSearch: true }), anchorCtx);
    // One 768-dim int8 vector per frame-document, plus a flat transcript term.
    expect(withSearch.diskByFeature.semanticSearch).toBeCloseTo(
      frameVectorMb(ANCHOR_INTERVAL_S) + 2,
      6,
    );
    expect(withSearch.diskMbPerDay).toBeCloseTo(
      base + withSearch.diskByFeature.semanticSearch,
      6,
    );
    expect(Math.round(frameVectorMb(ANCHOR_INTERVAL_S))).toBe(7);
  });

  it("prices a vector at one byte per dimension, not four", () => {
    // Migration 0039 stores every vector through `vec_quantize_int8(?, 'unit')`,
    // so an f32 figure here overstates the row by 4× on the screen where the
    // user decides whether to keep Semantic Search on. The literal is what pins
    // int8 — recomputing `framesPerDay * DIMS / 1e6` here would just restate the
    // implementation against itself and pass for any formula.
    expect(Math.round(frameVectorMb(ANCHOR_INTERVAL_S))).toBe(7);
    // A narrower model tier (issue #190) costs proportionally less.
    expect(frameVectorMb(ANCHOR_INTERVAL_S, 384)).toBeCloseTo(
      frameVectorMb(ANCHOR_INTERVAL_S) / 2,
      6,
    );
  });

  it("prices the SELECTED model's width, not a fixed 768", () => {
    // The `dims` parameter existed for this and had no caller passing it, so
    // onboarding quoted every tier at the default width. `granite-small-r2` is
    // 384-dim, so its row was shown at 2× its real disk — the same class of
    // overstatement as the f32 bug above, on the same screen.
    const wide = featureCost(state({ semanticSearch: true }), {
      ...anchorCtx,
      models: { semanticSearchModelId: "nomic-embed-text-v1.5" },
    });
    const narrow = featureCost(state({ semanticSearch: true }), {
      ...anchorCtx,
      models: { semanticSearchModelId: "granite-embedding-small-english-r2" },
    });

    expect(narrow.diskByFeature.semanticSearch).toBeCloseTo(
      frameVectorMb(ANCHOR_INTERVAL_S, 384) + 2,
      6,
    );
    expect(narrow.diskByFeature.semanticSearch).toBeLessThan(
      wide.diskByFeature.semanticSearch,
    );
    // An unknown or absent id falls back to the default tier rather than throwing.
    const unknown = featureCost(state({ semanticSearch: true }), {
      ...anchorCtx,
      models: { semanticSearchModelId: "a-model-that-does-not-exist" },
    });
    expect(unknown.diskByFeature.semanticSearch).toBeCloseTo(
      wide.diskByFeature.semanticSearch,
      6,
    );
  });

  it("stops charging for frame vectors when there are no frames to read", () => {
    const noScreen = applyToggle(state({ semanticSearch: true }), "screen");
    // Transcript vectors survive; frame vectors do not.
    expect(featureCost(noScreen, anchorCtx).diskByFeature.semanticSearch).toBe(2);
  });
});

describe("featureCost — download", () => {
  it("is the manifest sum for the default set", () => {
    const cost = featureCost(state({ semanticSearch: true }));
    expect(cost.downloadBytes).toBe(SPEAKRS_BYTES + WHISPER_BASE_BYTES + NOMIC_BYTES);
    expect(cost.downloadBytes).toBe(1_115_434_189);
  });

  it("prices each row from the work-list", () => {
    const cost = featureCost(state({ semanticSearch: true }));
    expect(cost.downloadByFeature.transcription).toBe(WHISPER_BASE_BYTES);
    expect(cost.downloadByFeature.speakerSeparation).toBe(SPEAKRS_BYTES);
    expect(cost.downloadByFeature.semanticSearch).toBe(NOMIC_BYTES);
    expect(cost.downloadByFeature.screen).toBe(0);
  });

  it("charges nothing for a model already on disk", () => {
    const on = (modelId) => ({
      modelId,
      displayName: modelId,
      byteSize: 1,
      installed: true,
    });
    const cost = featureCost(state({ semanticSearch: true }), {
      installed: {
        audioTranscription: on("base"),
        speakerAnalysis: on("pyannote-community-1-wespeaker"),
        semanticSearch: on("nomic-embed-text-v1.5"),
      },
    });
    expect(cost.downloadBytes).toBe(0);
  });

  it("queues only the embedding model once every audio source is off", () => {
    const noAudio = applyToggle(
      applyToggle(state({ semanticSearch: true }), "microphone"),
      "systemAudio",
    );
    expect(noAudio.transcription).toBe(false);
    expect(featureCost(noAudio).downloadBytes).toBe(NOMIC_BYTES);
  });

  it("reads as approximate exactly when Semantic Search is on", () => {
    expect(featureCost(state({ semanticSearch: true })).approximate).toBe(true);
    expect(featureCost(state()).approximate).toBe(false);
  });
});

describe("costDelta — what the preview strip prints", () => {
  it("frees audio, a transcript and two downloads when the last mic goes", () => {
    // Microphone as the ONLY audio source, so cutting it really cuts two rows.
    const micOnly = state({ systemAudio: false, semanticSearch: true });
    const p = preview(micOnly, "microphone");
    expect(p.cascade).toEqual(["transcription", "speakerSeparation"]);

    const delta = costDelta(micOnly, p.after, anchorCtx);
    expect(delta.diskMbPerDay).toBeCloseTo(
      -(ANCHOR_SHARE_MB.audioSource + ANCHOR_SHARE_MB.transcript + 2),
      6,
    );
    expect(delta.downloadBytes).toBe(-(WHISPER_BASE_BYTES + SPEAKRS_BYTES));
  });

  it("frees the transcript and speakrs when transcription goes", () => {
    const before = state({ semanticSearch: true });
    const p = preview(before, "transcription");
    expect(p.cascade).toEqual(["speakerSeparation"]);

    const delta = costDelta(before, p.after, anchorCtx);
    expect(delta.diskMbPerDay).toBeCloseTo(
      -(ANCHOR_SHARE_MB.transcript * 2 + 2),
      6,
    );
    expect(delta.downloadBytes).toBe(-(WHISPER_BASE_BYTES + SPEAKRS_BYTES));
  });

  it("costs the embedding model when Semantic Search comes on", () => {
    const before = state();
    const p = preview(before, "semanticSearch");
    const delta = costDelta(before, p.after, anchorCtx);
    expect(delta.downloadBytes).toBe(NOMIC_BYTES);
    expect(delta.diskMbPerDay).toBeGreaterThan(0);
  });

  it("is zero for a refused toggle, because `after` is the same state", () => {
    const locked = state({
      microphone: false,
      systemAudio: false,
      permissions: { microphone: false },
    });
    const p = preview(locked, "microphone");
    expect(p.noop).toBe(true);
    expect(costDelta(locked, p.after)).toEqual({ diskMbPerDay: 0, downloadBytes: 0 });
  });

  it("costs nothing either way for the AI row", () => {
    const before = state();
    const p = preview(before, "aiFeatures");
    expect(p.next).toBe(true);
    expect(costDelta(before, p.after)).toEqual({ diskMbPerDay: 0, downloadBytes: 0 });
  });

  it("is symmetric: undoing a flip returns the totals to where they were", () => {
    const before = state({ semanticSearch: true });
    const after = applyToggle(before, "semanticSearch");
    const out = costDelta(before, after, anchorCtx);
    const back = costDelta(after, before, anchorCtx);
    expect(back.diskMbPerDay).toBeCloseTo(-out.diskMbPerDay, 6);
    expect(back.downloadBytes).toBe(-out.downloadBytes);
  });
});
