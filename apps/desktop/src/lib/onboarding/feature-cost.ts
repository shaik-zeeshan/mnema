// What a feature set costs (issue #195, slice 8).
//
// `feature-rules.ts` owns the dependency graph and deliberately owns no
// arithmetic. This module is the other half: given a `FeatureState`, what it
// costs per day on disk and what it still has to download. It is the ONE place
// the per-row and total figures next to the switches come from, so a preview
// (`preview(state, id).after`) and a commit are priced by the same code.
//
// Two existing models are reused rather than re-derived:
//  · DOWNLOAD — `resolveSetup(...).workList`. Already the app's only download
//    agenda: it knows the manifest byte sizes, skips models already installed,
//    and drops an item the moment its feature goes off. A row's figure is what
//    the Setup screen would actually fetch, never a pasted constant.
//  · DISK — `estimateDailyStorageMb`, the one MEASURED anchor (270 MB/day at a
//    snapshot every 3 s, both audio sources on, no embeddings). This module only
//    DECOMPOSES that anchor into per-row shares; the shares sum back to it
//    exactly, so the totals here and the capture-rate sentence never disagree.
//
// Semantic Search sits ON TOP of the anchor: the measured machine shipped no
// vectors, so its cost is computed (one int8 vector per frame-document) rather
// than carved out of the 270.
import { DEFAULT_CAPTURE_INTERVAL_S } from "../components/capture-rate";
import { ANCHOR_MB_PER_DAY, estimateDailyStorageMb } from "./disk-estimate";
import { FEATURE_ORDER, type FeatureId, type FeatureState } from "./feature-rules";
import {
  resolveSetup,
  workListBytes,
  type ModelInventory,
  type ModelSelections,
} from "./resolve-setup";

/**
 * The measured 270 MB/day anchor, decomposed. Recordings 168 + index 47 + two
 * audio sources at 23 + a transcript per source at 4.5 = 270 exactly (pinned by
 * `feature-cost.test.ts`). Shares scale with the anchor, so any capture rate
 * keeps summing to `estimateDailyStorageMb(interval)`.
 */
export const ANCHOR_SHARE_MB = {
  /** Video frames. */
  screen: 168,
  /** The OCR half of the index — one pass per frame. */
  ocr: 47,
  /** Per audio source (microphone, system audio). */
  audioSource: 23,
  /** Per transcribed source. */
  transcript: 4.5,
} as const;

/** Hours of activity a day, from the anchor's pause-on-inactivity measurement. */
const ACTIVE_HOURS = 8;
/**
 * Output width of the default Semantic Search Model (nomic-embed-text-v1.5).
 * One byte per dimension at rest: migration `0039` stores every vector through
 * `vec_quantize_int8(?, 'unit')`, so the f32 the embedder produces is NOT what
 * lands on disk. `frameVectorMb` takes this as an argument so a model tier with
 * another width (issue #190) moves one call, not this constant.
 */
export const DEFAULT_EMBED_DIMS = 768;
/** Vectors over a day of transcripts — small and flat next to the frame side. */
const TRANSCRIPT_VECTOR_MB = 2;

/** Everything outside `FeatureState` that moves a cost. All optional. */
export interface CostContext {
  /** The live provider/model drafts. Defaults to the resolver's own defaults. */
  models?: ModelSelections | null;
  /** Live facts for each subsystem's selected model — an installed one costs 0. */
  installed?: Partial<ModelInventory> | null;
  /** Seconds between snapshots. Defaults to the ladder default (2 s). */
  captureIntervalSeconds?: number;
}

export interface FeatureCost {
  /** MB/day per row, in chain order. Rows with no disk cost are 0. */
  diskByFeature: Record<FeatureId, number>;
  /** MB/day for the whole set. */
  diskMbPerDay: number;
  /** Bytes still to fetch per row — 0 once the model is installed. */
  downloadByFeature: Record<FeatureId, number>;
  /** Bytes still to fetch for the whole set. */
  downloadBytes: number;
  /**
   * True when a figure includes nomic's approximate size, so the caller prints
   * "about". (`NOMIC_BYTES` is the Rust `approx_download_bytes`, and the vector
   * term is derived from it.)
   */
  approximate: boolean;
}

function zeroByFeature(): Record<FeatureId, number> {
  return Object.fromEntries(FEATURE_ORDER.map((id) => [id, 0])) as Record<
    FeatureId,
    number
  >;
}

/** Frames captured in one active day at `intervalSeconds`. */
export function framesPerDay(intervalSeconds: number): number {
  const interval = intervalSeconds > 0 ? intervalSeconds : DEFAULT_CAPTURE_INTERVAL_S;
  return (ACTIVE_HOURS * 3600) / interval;
}

/**
 * Stored vector width per Semantic Search model id, mirroring each descriptor's
 * `dimension` in `crates/semantic-search/src/models.rs`.
 *
 * A hand-mirrored table rather than a new field threaded through
 * `SelectedModelFacts`/`ModelInventory`: the disk estimate is the only consumer,
 * and `model-budget.test.ts` already re-derives the sizes straight out of the Rust
 * catalog, so the same drift guard covers this (see
 * `every_semantic_model_dimension_matches_the_rust_catalog`).
 *
 * The widths are NOT all equal, which is the whole point — this catalog ships a
 * 384-dim model, so pricing every tier at 768 overstates its disk by 2x on the
 * screen where the user decides whether to keep the feature on.
 */
export const EMBED_DIMS_BY_MODEL: Record<string, number> = {
  "nomic-embed-text-v1.5": 768,
  "multilingual-e5-small": 384,
  "bge-m3": 1024,
  // Stella's STORED width is its 2048-dim dense head, not the 1024 backbone; Arctic
  // stores 256 (Matryoshka-truncated from 1024). Both are easy to get wrong by
  // reading the backbone instead of the descriptor — hence the drift guard.
  stella_en_400M_v5: 2048,
  "snowflake-arctic-embed-l-v2.0": 256,
  "gte-modernbert-base": 768,
  "granite-embedding-english-r2": 768,
  "granite-embedding-small-english-r2": 384,
};

/** The stored vector width for `modelId`, or the default tier's when unknown. */
export function embedDimsFor(modelId: string | null | undefined): number {
  if (!modelId) return DEFAULT_EMBED_DIMS;
  return EMBED_DIMS_BY_MODEL[modelId] ?? DEFAULT_EMBED_DIMS;
}

/**
 * MB/day of embedding vectors over the captured frames — a CEILING.
 *
 * One int8 vector per frame-document. It is an over-estimate on purpose: an
 * `equivalent_reuse` anchor (a frame whose screen did not change) is never
 * embedded and has no `vec0` row at all, and how much of a real day dedups away
 * is a habit, not a constant. Erring high on the screen where the user decides
 * whether to keep the feature is the right direction to be wrong in.
 */
export function frameVectorMb(
  intervalSeconds: number,
  dims: number = DEFAULT_EMBED_DIMS,
): number {
  return (framesPerDay(intervalSeconds) * dims) / 1e6;
}

/** What `state` costs: MB/day on disk, bytes still to download, per row and total. */
export function featureCost(state: FeatureState, ctx: CostContext = {}): FeatureCost {
  const interval = ctx.captureIntervalSeconds ?? DEFAULT_CAPTURE_INTERVAL_S;
  // Scale the decomposition by the ladder stop, so the shares always sum back to
  // the anchor-derived total for this capture rate.
  const scale = estimateDailyStorageMb(interval) / ANCHOR_MB_PER_DAY;
  const sources = (state.microphone ? 1 : 0) + (state.systemAudio ? 1 : 0);
  const reads = state.screen && state.ocr;

  const diskByFeature = zeroByFeature();
  diskByFeature.screen = state.screen ? ANCHOR_SHARE_MB.screen * scale : 0;
  diskByFeature.ocr = reads ? ANCHOR_SHARE_MB.ocr * scale : 0;
  diskByFeature.microphone = state.microphone ? ANCHOR_SHARE_MB.audioSource * scale : 0;
  diskByFeature.systemAudio = state.systemAudio ? ANCHOR_SHARE_MB.audioSource * scale : 0;
  diskByFeature.transcription = state.transcription
    ? ANCHOR_SHARE_MB.transcript * sources * scale
    : 0;
  // Diarization writes speaker labels onto turns that already exist, and AI
  // features and privacy write nothing at all.
  // Priced at the SELECTED model's width, not a fixed 768: the catalog ships a
  // 384-dim tier, and quoting it at 768 doubles the disk figure the user is deciding
  // against. `frameVectorMb`'s `dims` argument existed for exactly this and had no
  // caller passing it.
  const embedDims = embedDimsFor(ctx.models?.semanticSearchModelId);
  diskByFeature.semanticSearch = state.semanticSearch
    ? (reads ? frameVectorMb(interval, embedDims) : 0) +
      (state.transcription ? TRANSCRIPT_VECTOR_MB : 0)
    : 0;

  const workList = resolveSetup(
    state.permissions,
    {
      speakerAnalysis: ctx.installed?.speakerAnalysis ?? null,
      audioTranscription: ctx.installed?.audioTranscription ?? null,
      semanticSearch: ctx.installed?.semanticSearch ?? null,
    },
    // Every feature is PRESENT, so the resolver copies the state through
    // untouched and only prices it. `excludedApps: []` keeps it from reporting
    // a recommended-privacy apply we are not asking for.
    { features: state, models: ctx.models ?? undefined, excludedApps: [] },
  ).workList;

  const downloadByFeature = zeroByFeature();
  for (const item of workList) downloadByFeature[item.feature] += item.bytes;

  return {
    diskByFeature,
    diskMbPerDay: sum(diskByFeature),
    downloadByFeature,
    downloadBytes: workListBytes(workList),
    approximate: state.semanticSearch,
  };
}

/** How much a move from `before` to `after` costs (positive) or frees (negative). */
export function costDelta(
  before: FeatureState,
  after: FeatureState,
  ctx: CostContext = {},
): { diskMbPerDay: number; downloadBytes: number } {
  const a = featureCost(before, ctx);
  const b = featureCost(after, ctx);
  return {
    diskMbPerDay: b.diskMbPerDay - a.diskMbPerDay,
    downloadBytes: b.downloadBytes - a.downloadBytes,
  };
}

function sum(by: Record<FeatureId, number>): number {
  return FEATURE_ORDER.reduce((total, id) => total + by[id], 0);
}
