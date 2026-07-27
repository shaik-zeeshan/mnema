// Onboarding setup resolver (issue #195, slice 1).
//
// `resolveSetup(permissions, installed, saved)` turns what the OS granted, what
// is already on disk, and what the user previously decided into ONE resolved
// settings object plus an ordered download work-list. Pure: no Svelte, no
// `invoke`, no side effects. It replaces the welcome-screen "Use recommended
// defaults" preset entirely — including the preset's one eager mutation
// (recommended privacy-listed apps), which becomes resolved DATA here and is
// committed atomically at finish.
//
// Rules that are load-bearing, in one place:
//  · All capture sources default ON. The permission grant is what makes each
//    real — a source whose permission is missing stays in the state (the row
//    renders "not granted"); it does not vanish.
//  · Model-backed AUDIO features gate on a real grant, so denying the mic does
//    not cost the user a 148 MB Whisper download for audio that never arrives.
//  · Deepgram is NEVER selected here. Cloud transcription is Settings-only,
//    behind its consent gate (ADR 0047).
//  · AI features (Reasoning Engine) resolve OFF. Consent is never pre-ticked.
//  · SAVED SETTINGS WIN — the resolver only fills gaps. Re-entering must never
//    silently re-enable something the user deliberately turned off.
//  · The resolver never touches secret-vault contents or Reasoning Engine
//    provider config, and it never overwrites a saved privacy-listed app set.
import {
  type FeatureId,
  type FeatureState,
  type PermissionIntents,
  normalizeFeatures,
} from "./feature-rules";

export type { FeatureId, FeatureState, PermissionIntents };

// ── Default provider / model selections ────────────────────────────────────

/** Apple Vision: zero bytes, OS-managed, no download. */
export const DEFAULT_OCR_PROVIDER = "apple_vision";
/** Local Whisper — never `deepgram` (ADR 0047). */
export const DEFAULT_TRANSCRIPTION_PROVIDER = "local_whisper";
export const DEFAULT_TRANSCRIPTION_MODEL_ID = "base";
export const DEFAULT_SPEAKER_PROVIDER = "speakrs";
export const DEFAULT_SPEAKER_MODEL_ID = "pyannote-community-1-wespeaker";
export const DEFAULT_SEMANTIC_SEARCH_MODEL_ID = "nomic-embed-text-v1.5";
/** `semantic-search/src/models.rs:39` — `SEMANTIC_SEARCH_PROVIDER_ID`. */
export const SEMANTIC_SEARCH_PROVIDER = "local";

// Download sizes, mirrored from the Rust manifests that own them. Kept as
// constants (not fetched) so the resolver stays pure; each cites its source so
// a drift is a one-line fix.
/** `speaker-analysis/src/lib.rs:361` — the MultiFile artifact's declared size.
 *  NOTE: the plan's "729 MB total / 179 MB without Semantic Search" figures
 *  assumed a ~31 MB speakrs bundle; the real artifact is ~419 MB, so the full
 *  default set is ~1.12 GB. */
export const SPEAKRS_BYTES = 419_482_724;
/** `audio-transcription/src/lib.rs:268` — Whisper Base. */
export const WHISPER_BASE_BYTES = 147_951_465;
/**
 * `semantic-search/src/models.rs:343` — nomic-embed-text-v1.5. APPROXIMATE: the
 * Rust field is `approx_download_bytes` (weights + tokenizer + config at the
 * pinned revision, rounded up so the disk preflight never undercounts). There is
 * no exact figure anywhere in the repo — unlike speakrs and Whisper, whose sizes
 * are per-file verified. So any total containing this item is approximate too.
 */
export const NOMIC_BYTES = 548_000_000;

// ── Inputs ─────────────────────────────────────────────────────────────────

/** Which default models are already on disk. Installed models are omitted from
 *  the work-list — that is what makes re-entry cheap. */
export interface ModelInventory {
  /** speakrs `pyannote-community-1-wespeaker` diarization bundle. */
  speakerAnalysis: boolean;
  /** local Whisper `base`. */
  whisperBase: boolean;
  /** `nomic-embed-text-v1.5`. */
  semanticSearch: boolean;
}

/** Provider/model selections. Every field is resolved (never undefined) on the
 *  way out; the saved half is a Partial because absent = a gap to fill. */
export interface ModelSelections {
  ocrProvider: string;
  ocrModelId: string | null;
  transcriptionProvider: string;
  transcriptionModelId: string | null;
  speakerProvider: string;
  speakerModelId: string | null;
  semanticSearchModelId: string | null;
}

/**
 * What the user has already decided. Any field PRESENT here is a deliberate
 * choice and is copied through untouched; absent fields are gaps the resolver
 * fills. Pass `null` on a first run.
 */
export interface SavedChoices {
  features?: Partial<Record<FeatureId, boolean>>;
  models?: Partial<ModelSelections>;
  /** Privacy-listed apps the user already has. Present (even empty) means the
   *  user has been here — the recommended list is NOT re-applied over it. */
  excludedApps?: string[];
}

// ── Output ─────────────────────────────────────────────────────────────────

/** Which per-subsystem download command drives this item. */
export type DownloadSubsystem =
  | "ocr"
  | "audioTranscription"
  | "speakerAnalysis"
  | "semanticSearch";

/** One model to fetch. The work-list is the Setup screen's whole agenda. */
export interface DownloadWorkItem {
  /** Stable id: `${subsystem}:${provider}:${modelId}`. */
  id: string;
  subsystem: DownloadSubsystem;
  provider: string;
  modelId: string;
  /** Model display name — what the Setup screen's current-item label shows. */
  label: string;
  bytes: number;
  /** The feature turned off if this download is cancelled. The capture source
   *  stays on: cancelling Whisper turns transcription off, not the microphone. */
  feature: FeatureId;
}

export interface ResolvedSettings {
  features: FeatureState;
  models: ModelSelections;
  /** The user's existing privacy-listed apps, verbatim. Never rewritten here. */
  excludedApps: string[];
  /**
   * True only on a first run: the caller should apply the backend's recommended
   * privacy exclusions (`applyAllRecommendedPrivacyApps`) at commit. The old
   * welcome preset did this eagerly as a side effect; here it is resolved DATA,
   * so a returning user's list is never re-seeded over.
   */
  applyRecommendedExcludedApps: boolean;
  /** Fixed order: speakrs → Whisper base → nomic. Speakrs is first because the
   *  Voice step cannot run its embedder without it. */
  workList: DownloadWorkItem[];
}

// ── Resolver ───────────────────────────────────────────────────────────────

export function resolveSetup(
  permissions: PermissionIntents,
  installed: ModelInventory,
  saved: SavedChoices | null,
): ResolvedSettings {
  const savedFeatures = saved?.features ?? {};
  const pick = (id: FeatureId, fallback: boolean) => savedFeatures[id] ?? fallback;

  // Only a GRANTED source can produce audio, so the model-backed audio chain
  // defaults off when nothing was granted (story 6: one "no" must not dead-end
  // setup, and must not queue a download for audio that never arrives).
  const grantedAudio = permissions.microphone || permissions.systemAudio;

  const features = normalizeFeatures({
    permissions,
    // Capture sources default ON regardless of grant — they stay listed.
    screen: pick("screen", true),
    microphone: pick("microphone", true),
    systemAudio: pick("systemAudio", true),
    // Apple Vision costs zero bytes and OCR over no frames is inert, so it is
    // not grant-gated: it self-heals the moment Screen Recording is granted.
    ocr: pick("ocr", true),
    transcription: pick("transcription", grantedAudio),
    speakerSeparation: pick("speakerSeparation", grantedAudio),
    semanticSearch: pick("semanticSearch", true),
    aiFeatures: pick("aiFeatures", false),
    privacy: pick("privacy", true),
    transcribeMicrophone: false,
    transcribeSystemAudio: false,
    recognizeSavedPeople: false,
  });

  const savedModels = saved?.models ?? {};
  const models: ModelSelections = {
    ocrProvider: savedModels.ocrProvider ?? DEFAULT_OCR_PROVIDER,
    ocrModelId: savedModels.ocrModelId ?? null,
    transcriptionProvider:
      savedModels.transcriptionProvider ?? DEFAULT_TRANSCRIPTION_PROVIDER,
    transcriptionModelId:
      savedModels.transcriptionModelId ?? DEFAULT_TRANSCRIPTION_MODEL_ID,
    speakerProvider: savedModels.speakerProvider ?? DEFAULT_SPEAKER_PROVIDER,
    speakerModelId: savedModels.speakerModelId ?? DEFAULT_SPEAKER_MODEL_ID,
    semanticSearchModelId:
      savedModels.semanticSearchModelId ?? DEFAULT_SEMANTIC_SEARCH_MODEL_ID,
  };

  return {
    features,
    models,
    excludedApps: [...(saved?.excludedApps ?? [])],
    applyRecommendedExcludedApps: saved?.excludedApps === undefined,
    workList: buildWorkList(features, models, installed),
  };
}

/**
 * Only what is (a) needed by an enabled feature, (b) an app-managed default,
 * and (c) not already installed. Order is fixed, never sorted by size.
 */
function buildWorkList(
  features: FeatureState,
  models: ModelSelections,
  installed: ModelInventory,
): DownloadWorkItem[] {
  const items: DownloadWorkItem[] = [];

  if (
    features.speakerSeparation &&
    !installed.speakerAnalysis &&
    models.speakerProvider === DEFAULT_SPEAKER_PROVIDER &&
    models.speakerModelId === DEFAULT_SPEAKER_MODEL_ID
  ) {
    items.push({
      id: `speakerAnalysis:${DEFAULT_SPEAKER_PROVIDER}:${DEFAULT_SPEAKER_MODEL_ID}`,
      subsystem: "speakerAnalysis",
      provider: DEFAULT_SPEAKER_PROVIDER,
      modelId: DEFAULT_SPEAKER_MODEL_ID,
      label: "pyannote community-1 + WeSpeaker (CoreML)",
      bytes: SPEAKRS_BYTES,
      feature: "speakerSeparation",
    });
  }

  if (
    features.transcription &&
    !installed.whisperBase &&
    models.transcriptionProvider === DEFAULT_TRANSCRIPTION_PROVIDER &&
    models.transcriptionModelId === DEFAULT_TRANSCRIPTION_MODEL_ID
  ) {
    items.push({
      id: `audioTranscription:${DEFAULT_TRANSCRIPTION_PROVIDER}:${DEFAULT_TRANSCRIPTION_MODEL_ID}`,
      subsystem: "audioTranscription",
      provider: DEFAULT_TRANSCRIPTION_PROVIDER,
      modelId: DEFAULT_TRANSCRIPTION_MODEL_ID,
      label: "Whisper Base",
      bytes: WHISPER_BASE_BYTES,
      feature: "transcription",
    });
  }

  if (
    features.semanticSearch &&
    !installed.semanticSearch &&
    models.semanticSearchModelId === DEFAULT_SEMANTIC_SEARCH_MODEL_ID
  ) {
    items.push({
      id: `semanticSearch:${SEMANTIC_SEARCH_PROVIDER}:${DEFAULT_SEMANTIC_SEARCH_MODEL_ID}`,
      subsystem: "semanticSearch",
      provider: SEMANTIC_SEARCH_PROVIDER,
      modelId: DEFAULT_SEMANTIC_SEARCH_MODEL_ID,
      label: "Nomic Embed Text v1.5",
      bytes: NOMIC_BYTES,
      feature: "semanticSearch",
    });
  }

  return items;
}

/**
 * Total bytes the work-list will fetch — the free-disk preflight's input, and
 * the only source for the download total any screen shows. Per-item sizes are
 * on `DownloadWorkItem.bytes`, so *Change settings* needs nothing else to render
 * a per-row size next to a running total.
 *
 * APPROXIMATE by one component: `NOMIC_BYTES` is the Rust `approx_download_bytes`
 * (rounded up). Round the display; never present this as exact.
 */
export function workListBytes(workList: readonly DownloadWorkItem[]): number {
  return workList.reduce((sum, item) => sum + item.bytes, 0);
}
