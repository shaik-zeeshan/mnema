// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  DEFAULT_SEMANTIC_SEARCH_MODEL_ID,
  DEFAULT_SPEAKER_MODEL_ID,
  DEFAULT_TRANSCRIPTION_MODEL_ID,
  NOMIC_BYTES,
  SPEAKRS_BYTES,
  WHISPER_BASE_BYTES,
  resolveSetup,
  savedChoicesFor,
  workListBytes,
} from "./resolve-setup";

const NONE = { screen: false, microphone: false, systemAudio: false };
const SCREEN_ONLY = { screen: true, microphone: false, systemAudio: false };
const SCREEN_MIC = { screen: true, microphone: true, systemAudio: false };
const SCREEN_SYSAUDIO = { screen: true, microphone: false, systemAudio: true };
const ALL = { screen: true, microphone: true, systemAudio: true };

// No live model status at all — the default picks still resolve from the
// built-in fallbacks, which is what a cold first run looks like.
const NOTHING_INSTALLED = {
  speakerAnalysis: null,
  audioTranscription: null,
  semanticSearch: null,
};

/** Live facts for a model, in the shape `resolveSetup` takes. */
const factsFor = (modelId, installed, byteSize = 1) => ({
  modelId,
  displayName: `model ${modelId}`,
  byteSize,
  installed,
});

const installedDefault = (modelId) => factsFor(modelId, true);

const ALL_INSTALLED = {
  speakerAnalysis: installedDefault(DEFAULT_SPEAKER_MODEL_ID),
  audioTranscription: installedDefault(DEFAULT_TRANSCRIPTION_MODEL_ID),
  semanticSearch: installedDefault(DEFAULT_SEMANTIC_SEARCH_MODEL_ID),
};

const ids = (result) => result.workList.map((item) => item.id);

// ── Test 1: table-driven over permission combos × installed-model states ────

describe("resolveSetup — feature set per permission combination", () => {
  const cases = [
    {
      name: "none granted",
      permissions: NONE,
      transcription: false,
      speakerSeparation: false,
      work: ["semanticSearch:local:nomic-embed-text-v1.5"],
    },
    {
      name: "screen only",
      permissions: SCREEN_ONLY,
      transcription: false,
      speakerSeparation: false,
      work: ["semanticSearch:local:nomic-embed-text-v1.5"],
    },
    {
      name: "screen + mic",
      permissions: SCREEN_MIC,
      transcription: true,
      speakerSeparation: true,
      work: [
        `speakerAnalysis:speakrs:${DEFAULT_SPEAKER_MODEL_ID}`,
        `audioTranscription:local_whisper:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
        `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
      ],
    },
    {
      name: "screen + system-audio intent",
      permissions: SCREEN_SYSAUDIO,
      transcription: true,
      speakerSeparation: true,
      work: [
        `speakerAnalysis:speakrs:${DEFAULT_SPEAKER_MODEL_ID}`,
        `audioTranscription:local_whisper:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
        `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
      ],
    },
    {
      name: "all granted",
      permissions: ALL,
      transcription: true,
      speakerSeparation: true,
      work: [
        `speakerAnalysis:speakrs:${DEFAULT_SPEAKER_MODEL_ID}`,
        `audioTranscription:local_whisper:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
        `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
      ],
    },
  ];

  for (const testCase of cases) {
    it(`${testCase.name}: capture sources stay listed, audio chain follows the grant`, () => {
      const result = resolveSetup(testCase.permissions, NOTHING_INSTALLED, null);
      // All capture sources default ON regardless of grant — they must not vanish.
      expect(result.features.screen).toBe(true);
      expect(result.features.microphone).toBe(true);
      expect(result.features.systemAudio).toBe(true);
      // ...but the permission context travels with the state so a row can render
      // "not granted".
      expect(result.features.permissions).toEqual(testCase.permissions);

      expect(result.features.ocr).toBe(true);
      expect(result.features.semanticSearch).toBe(true);
      expect(result.features.transcription).toBe(testCase.transcription);
      expect(result.features.speakerSeparation).toBe(testCase.speakerSeparation);
      // Consent is never pre-ticked.
      expect(result.features.aiFeatures).toBe(false);
    });

    it(`${testCase.name}: work-list is ordered speakrs → Whisper → nomic`, () => {
      const result = resolveSetup(testCase.permissions, NOTHING_INSTALLED, null);
      expect(ids(result)).toEqual(testCase.work);
    });

    it(`${testCase.name}: nothing is queued when every model is installed`, () => {
      const result = resolveSetup(testCase.permissions, ALL_INSTALLED, null);
      expect(result.workList).toEqual([]);
      expect(workListBytes(result.workList)).toBe(0);
    });
  }
});

describe("resolveSetup — work-list omits installed models", () => {
  it("drops only the installed ones and keeps the rest in order", () => {
    const result = resolveSetup(
      ALL,
      { ...NOTHING_INSTALLED, speakerAnalysis: installedDefault(DEFAULT_SPEAKER_MODEL_ID) },
      null,
    );
    expect(ids(result)).toEqual([
      `audioTranscription:local_whisper:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
      `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
    ]);
    expect(workListBytes(result.workList)).toBe(WHISPER_BASE_BYTES + NOMIC_BYTES);
  });

  it("keeps speakrs first when only Whisper is installed", () => {
    const result = resolveSetup(
      ALL,
      {
        ...NOTHING_INSTALLED,
        audioTranscription: installedDefault(DEFAULT_TRANSCRIPTION_MODEL_ID),
      },
      null,
    );
    expect(ids(result)).toEqual([
      `speakerAnalysis:speakrs:${DEFAULT_SPEAKER_MODEL_ID}`,
      `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
    ]);
    expect(workListBytes(result.workList)).toBe(SPEAKRS_BYTES + NOMIC_BYTES);
  });

  it("carries the byte size and cancel-target feature on each item", () => {
    const result = resolveSetup(ALL, NOTHING_INSTALLED, null);
    expect(result.workList.map((item) => item.bytes)).toEqual([
      SPEAKRS_BYTES,
      WHISPER_BASE_BYTES,
      NOMIC_BYTES,
    ]);
    // Cancelling a download disables the PROCESSING feature, never the source.
    expect(result.workList.map((item) => item.feature)).toEqual([
      "speakerSeparation",
      "transcription",
      "semanticSearch",
    ]);
  });
});

// ── Regression: the work-list follows the USER'S pick, not the default ──────
// Before this, `buildWorkList` only queued a model when the selection equalled
// the hard-coded default, so choosing Whisper Small (or a custom embedding
// model) downloaded nothing at all — the Setup screen checked and fetched the
// defaults regardless.

describe("resolveSetup — work-list follows the selected model", () => {
  const SMALL_BYTES = 487_601_967;

  it("queues a non-default transcription model with its live size and name", () => {
    const result = resolveSetup(
      ALL,
      {
        ...NOTHING_INSTALLED,
        audioTranscription: {
          modelId: "small",
          displayName: "Whisper Small",
          byteSize: SMALL_BYTES,
          installed: false,
        },
      },
      { models: { transcriptionModelId: "small" } },
    );
    const item = result.workList.find((entry) => entry.subsystem === "audioTranscription");
    expect(item.modelId).toBe("small");
    expect(item.id).toBe("audioTranscription:local_whisper:small");
    expect(item.label).toBe("Whisper Small");
    expect(item.bytes).toBe(SMALL_BYTES);
    // ...and the default is NOT also queued.
    expect(ids(result)).not.toContain(
      `audioTranscription:local_whisper:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
    );
  });

  it("skips a non-default model that is already installed", () => {
    const result = resolveSetup(
      ALL,
      { ...NOTHING_INSTALLED, audioTranscription: factsFor("small", true) },
      { models: { transcriptionModelId: "small" } },
    );
    expect(result.workList.some((item) => item.subsystem === "audioTranscription")).toBe(
      false,
    );
  });

  it("still queues the DEFAULT model when no live status has loaded", () => {
    const result = resolveSetup(ALL, NOTHING_INSTALLED, null);
    expect(workListBytes(result.workList)).toBe(
      SPEAKRS_BYTES + WHISPER_BASE_BYTES + NOMIC_BYTES,
    );
  });

  it("does not queue a non-default model it cannot size or name", () => {
    // No live facts and not a default: guessing a size would corrupt the
    // free-disk preflight, so the item waits for the status to load.
    const result = resolveSetup(ALL, NOTHING_INSTALLED, {
      models: { transcriptionModelId: "small" },
    });
    expect(result.workList.some((item) => item.subsystem === "audioTranscription")).toBe(
      false,
    );
  });

  it("ignores facts describing a model the selection has moved off", () => {
    // The status stores fall back to a provider's FIRST model, so stale facts
    // must never name or size the item.
    const result = resolveSetup(
      ALL,
      { ...NOTHING_INSTALLED, audioTranscription: factsFor("medium", true) },
      { models: { transcriptionModelId: "small" } },
    );
    expect(result.workList.some((item) => item.subsystem === "audioTranscription")).toBe(
      false,
    );
  });

  it("queues a custom embedding model with an unknown size as zero bytes", () => {
    const result = resolveSetup(
      ALL,
      {
        ...NOTHING_INSTALLED,
        semanticSearch: {
          modelId: "custom/e5-base",
          displayName: "E5 Base",
          byteSize: null,
          installed: false,
        },
      },
      { models: { semanticSearchModelId: "custom/e5-base" } },
    );
    const item = result.workList.find((entry) => entry.subsystem === "semanticSearch");
    expect(item.id).toBe("semanticSearch:local:custom/e5-base");
    expect(item.bytes).toBe(0);
  });
});

describe("resolveSetup — provider defaults", () => {
  it("picks Apple Vision, local Whisper base, speakrs and nomic", () => {
    const { models } = resolveSetup(ALL, NOTHING_INSTALLED, null);
    expect(models).toEqual({
      ocrProvider: "apple_vision",
      ocrModelId: null,
      transcriptionProvider: "local_whisper",
      transcriptionModelId: "base",
      speakerProvider: "speakrs",
      speakerModelId: "pyannote-community-1-wespeaker",
      semanticSearchModelId: "nomic-embed-text-v1.5",
    });
  });

  it("Apple Vision costs zero bytes — OCR never enters the work-list", () => {
    const result = resolveSetup(ALL, NOTHING_INSTALLED, null);
    expect(result.workList.some((item) => item.subsystem === "ocr")).toBe(false);
  });

  it("NEVER selects Deepgram, under any permission or installed state", () => {
    for (const permissions of [NONE, SCREEN_ONLY, SCREEN_MIC, SCREEN_SYSAUDIO, ALL]) {
      for (const installed of [NOTHING_INSTALLED, ALL_INSTALLED]) {
        const result = resolveSetup(permissions, installed, null);
        expect(result.models.transcriptionProvider).toBe("local_whisper");
        expect(result.models.transcriptionProvider).not.toBe("deepgram");
        expect(result.workList.some((item) => item.provider === "deepgram")).toBe(
          false,
        );
      }
    }
  });

  it("preserves a Deepgram choice the user already made in Settings", () => {
    // Saved wins — but the resolver still never queues a local Whisper download
    // for a provider that has no model to fetch.
    const result = resolveSetup(ALL, NOTHING_INSTALLED, {
      models: { transcriptionProvider: "deepgram", transcriptionModelId: null },
    });
    expect(result.models.transcriptionProvider).toBe("deepgram");
    expect(ids(result)).toEqual([
      `speakerAnalysis:speakrs:${DEFAULT_SPEAKER_MODEL_ID}`,
      `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
    ]);
  });
});

describe("resolveSetup — privacy-listed apps are data, not a mutation", () => {
  it("asks for the recommended list only on a first run", () => {
    const first = resolveSetup(ALL, NOTHING_INSTALLED, null);
    expect(first.applyRecommendedExcludedApps).toBe(true);
    expect(first.excludedApps).toEqual([]);
  });

  it("leaves a returning user's list alone, even when empty", () => {
    const kept = resolveSetup(ALL, NOTHING_INSTALLED, {
      excludedApps: ["com.example.vault"],
    });
    expect(kept.applyRecommendedExcludedApps).toBe(false);
    expect(kept.excludedApps).toEqual(["com.example.vault"]);

    const emptied = resolveSetup(ALL, NOTHING_INSTALLED, { excludedApps: [] });
    expect(emptied.applyRecommendedExcludedApps).toBe(false);
    expect(emptied.excludedApps).toEqual([]);
  });
});

// ── Test 8: re-entry ───────────────────────────────────────────────────────

describe("resolveSetup — re-entry", () => {
  it("keeps a deliberately disabled feature disabled", () => {
    const result = resolveSetup(ALL, NOTHING_INSTALLED, {
      features: { semanticSearch: false, ocr: false },
    });
    expect(result.features.semanticSearch).toBe(false);
    expect(result.features.ocr).toBe(false);
    // ...and its download is not queued.
    expect(ids(result)).toEqual([
      `speakerAnalysis:speakrs:${DEFAULT_SPEAKER_MODEL_ID}`,
      `audioTranscription:local_whisper:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
    ]);
  });

  it("does not re-enable a disabled capture source when its permission is granted", () => {
    const result = resolveSetup(ALL, NOTHING_INSTALLED, {
      features: { microphone: false },
    });
    expect(result.features.microphone).toBe(false);
    expect(result.features.transcribeMicrophone).toBe(false);
    // System audio is still on, so the audio chain survives.
    expect(result.features.transcription).toBe(true);
    expect(result.features.transcribeSystemAudio).toBe(true);
  });

  it("turns the audio chain off when the user disabled every source", () => {
    const result = resolveSetup(ALL, NOTHING_INSTALLED, {
      features: { microphone: false, systemAudio: false },
    });
    expect(result.features.transcription).toBe(false);
    expect(result.features.speakerSeparation).toBe(false);
    expect(ids(result)).toEqual([
      `semanticSearch:local:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
    ]);
  });

  it("is idempotent — re-resolving its own output changes nothing", () => {
    const saved = { features: { transcription: false, aiFeatures: true } };
    const first = resolveSetup(ALL, NOTHING_INSTALLED, saved);
    const second = resolveSetup(ALL, NOTHING_INSTALLED, {
      features: first.features,
      models: first.models,
      excludedApps: first.excludedApps,
    });
    expect(second.features).toEqual(first.features);
    expect(second.models).toEqual(first.models);
    expect(ids(second)).toEqual(ids(first));
  });

  it("never re-ticks AI features, but keeps them on once configured", () => {
    expect(resolveSetup(ALL, NOTHING_INSTALLED, {}).features.aiFeatures).toBe(false);
    expect(
      resolveSetup(ALL, NOTHING_INSTALLED, { features: { aiFeatures: true } })
        .features.aiFeatures,
    ).toBe(true);
  });
});

// ── ADR 0047: cloud transcription never appears in onboarding ───────────────

describe("resolveSetup — Deepgram is never queued for download", () => {
  // Deepgram's descriptors DO carry model ids (`nova-3`/`nova-2`) and their
  // `available` flag means "an API key is present", not "bytes are on disk" — so
  // with no key the model reads as not-installed and would land on the download
  // agenda, which `start_audio_transcription_model_download` rejects outright.
  // Keeping the saved provider is correct ("saved settings win"); queuing a
  // download for it is not.
  it("keeps the saved provider but queues no cloud download", () => {
    const resolved = resolveSetup(
      ALL,
      {
        ...NOTHING_INSTALLED,
        audioTranscription: {
          modelId: "nova-3",
          displayName: "Deepgram Nova-3",
          byteSize: null,
          installed: false,
        },
      },
      {
        features: { microphone: true, systemAudio: true, transcription: true },
        models: { transcriptionProvider: "deepgram", transcriptionModelId: "nova-3" },
        excludedApps: [],
      },
    );
    expect(resolved.models.transcriptionProvider).toBe("deepgram");
    expect(resolved.workList.map((item) => item.provider)).not.toContain("deepgram");
  });
});

describe("resolveSetup — a first run has no persisted settings to win with", () => {
  // The flow's own `savedChoices()` returns `null` whenever nothing is persisted,
  // which is EVERY genuine first run — so `next()`'s re-resolve on the way out of
  // Permissions would restore the default for every row. `OnboardingFlow` now
  // carries the rows the user actually flipped, so this is the shape it passes:
  // a features-only `SavedChoices` with no `excludedApps`, which must both win
  // AND leave the recommended privacy list to be applied.
  it("a features-only saved set wins and still seeds the recommended privacy apps", () => {
    const resolved = resolveSetup(ALL, NOTHING_INSTALLED, {
      features: { semanticSearch: false },
    });

    expect(resolved.features.semanticSearch).toBe(false);
    expect(resolved.workList.some((item) => item.subsystem === "semanticSearch")).toBe(false);
    // `excludedApps` absent => first run => the recommended list is still applied.
    expect(resolved.applyRecommendedExcludedApps).toBe(true);
  });
});

// `resolveSetup` honouring a SavedChoices object proves nothing about the flow
// BUILDING one, and the first-run branch is where "saved settings win" actually
// lives: a first run has no persisted settings to win with, so the only
// deliberate choices in existence are the rows flipped this session.
describe("savedChoicesFor — what the flow hands the resolver", () => {
  const DRAFTS = {
    screen: true,
    microphone: true,
    systemAudio: true,
    ocr: true,
    transcription: true,
    speakerSeparation: true,
    semanticSearch: true,
    aiFeatures: false,
    privacy: true,
  };
  const MODELS = {
    ocrProvider: "apple_vision",
    ocrModelId: null,
    transcriptionProvider: "local_whisper",
    transcriptionModelId: DEFAULT_TRANSCRIPTION_MODEL_ID,
    speakerProvider: "speakrs",
    speakerModelId: DEFAULT_SPEAKER_MODEL_ID,
    semanticSearchModelId: DEFAULT_SEMANTIC_SEARCH_MODEL_ID,
  };
  const base = (over) => ({
    everSaved: false,
    touched: {},
    drafts: DRAFTS,
    models: MODELS,
    excludedAppIds: [],
    ...over,
  });

  it("is null on a first run with nothing touched — every field is a gap", () => {
    expect(savedChoicesFor(base({}))).toBeNull();
  });

  it("carries ONLY the rows a first-run user flipped, and no excludedApps", () => {
    const saved = savedChoicesFor(base({ touched: { semanticSearch: false } }));
    expect(saved).toEqual({ features: { semanticSearch: false } });
    // Absent (not empty) — present-even-empty would suppress the recommended
    // privacy list on a run where the user has never seen it.
    expect(saved).not.toHaveProperty("excludedApps");
    expect(saved).not.toHaveProperty("models");
  });

  // The two halves composed. This is the invariant the PR states in prose:
  // "re-entry never re-enables something the user deliberately turned off".
  it("a first-run opt-out survives the round trip back through the resolver", () => {
    const saved = savedChoicesFor(base({ touched: { semanticSearch: false } }));
    const resolved = resolveSetup(ALL, NOTHING_INSTALLED, saved);
    expect(resolved.features.semanticSearch).toBe(false);
    expect(resolved.workList.some((item) => item.subsystem === "semanticSearch")).toBe(false);
    // Still a first run for privacy purposes.
    expect(resolved.applyRecommendedExcludedApps).toBe(true);
  });

  it("on re-entry passes every draft, and a touched row beats its draft", () => {
    const saved = savedChoicesFor(
      base({ everSaved: true, touched: { semanticSearch: false }, excludedAppIds: ["com.acme.app"] }),
    );
    expect(saved.features.semanticSearch).toBe(false);
    expect(saved.features.ocr).toBe(true);
    expect(saved.features.aiFeatures).toBe(false);
    expect(saved.models).toEqual(MODELS);
    expect(saved.excludedApps).toEqual(["com.acme.app"]);
  });

  it("a returning user with an empty privacy list is not re-seeded", () => {
    const saved = savedChoicesFor(base({ everSaved: true }));
    expect(saved.excludedApps).toEqual([]);
    expect(resolveSetup(ALL, NOTHING_INSTALLED, saved).applyRecommendedExcludedApps).toBe(false);
  });
});
