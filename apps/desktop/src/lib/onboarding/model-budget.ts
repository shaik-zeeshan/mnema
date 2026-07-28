// Model-picker catalog + download budget arithmetic (issue #195, slice 10).
//
// Ported from `docs/onboarding/mockups/input-components/parts/models.part.html`
// — that mockup is the design of record. Pure: no Svelte, no `invoke`, so the
// arithmetic the footer prints is unit-testable on its own.
//
// Every byte figure comes from the backend at runtime (`download.byteSize`,
// `approxDownloadBytes`), never from a constant here. `model-budget.test.ts`
// re-derives the Rust manifest sums from the Rust sources and fails if they
// drift. The one exception is the speaker model's fallback, which already lives
// in `resolve-setup.ts` as `SPEAKRS_BYTES`.
import { formatBytes } from "../settings/state/format";
import { RESERVE_FLOOR_BYTES, storageNeedBytes } from "./gates";
import { estimateDailyStorageMb } from "./disk-estimate";
import type {
  AudioTranscriptionModelStatus,
  SemanticSearchModelStatus,
  SemanticSearchSupportedModel,
} from "../types";

/** Sentinel option value for a model with no id (Apple Speech is OS-managed).
 *  Mirrors `routes/onboarding/onboarding-models.svelte.ts`, which owns it —
 *  duplicated here only so this module stays free of a `routes/` import. */
export const OS_MANAGED_VALUE = "__os_managed__";

/** The family a disabled Semantic Search resolves to. No segment carries it —
 *  that is how "off" reads as no active segment rather than as a wrong one. */
export const SEMANTIC_OFF = "off";
export const SEMANTIC_ENGLISH = "en";
export const SEMANTIC_MULTILINGUAL = "multi";

/** One selectable build, render-ready for the variant sub-group + detail strip. */
export interface PickerModel {
  /** Option value — the model id, or `OS_MANAGED_VALUE`. */
  id: string;
  /** Family key: a transcription provider id, or a Semantic Search language group. */
  family: string;
  /** Full name, for the strip's Model cell. */
  name: string;
  /** Segment label — short enough for a sub-group pill. */
  short: string;
  /** Bytes to fetch. 0 for OS-managed or already installed. */
  bytes: number;
  /** True when `bytes` is an `approx_download_bytes` figure. */
  approx: boolean;
  installed: boolean;
  osManaged: boolean;
  /** The strip's third cell: memory while running / language coverage. */
  detail: string;
}

// ── Editorial copy ─────────────────────────────────────────────────────────
// Verbatim from the mockup. These are facts the manifests do NOT carry, so they
// live as copy: no manifest has a memory figure, and Parakeet int8 has no honest
// number anywhere, so it says what is true rather than inventing one.

const SHORT_LABELS: Record<string, string> = {
  tiny: "Tiny",
  base: "Base",
  small: "Small",
  medium: "Medium",
  "parakeet-tdt-0.6b-v3-onnx": "Full precision",
  "parakeet-tdt-0.6b-v3-onnx-int8": "int8",
  "nomic-embed-text-v1.5": "Nomic",
  stella_en_400M_v5: "Stella",
  "multilingual-e5-small": "E5 Small",
  "bge-m3": "BGE-M3",
  "snowflake-arctic-embed-l-v2.0": "Snowflake",
};

/** Memory while running, keyed by model id first, then provider. */
const MEMORY_NOTES: Record<string, string> = {
  local_whisper: "about 1.4 GB while running",
  apple_speech_on_device: "macOS handles it",
  parakeet: "about 3 GB while running",
  "parakeet-tdt-0.6b-v3-onnx-int8": "below Parakeet full precision",
};

/** The sentence that fills the height-reserved variant row. A family with one
 *  build still owes the reader a fact; that is what keeps the row from reading
 *  as empty. */
export const FAMILY_NOTES: Record<string, string> = {
  apple_speech_on_device: "One OS model. Nothing to choose, nothing to download.",
  local_whisper: "Larger is more accurate and slower.",
  parakeet: "Multilingual, CC-BY. int8 trades accuracy for memory.",
  [SEMANTIC_OFF]:
    "Keyword search still works. This is the only real saving on this row.",
  [SEMANTIC_ENGLISH]: "Bigger indexes retrieve better; none are cheap.",
  [SEMANTIC_MULTILINGUAL]: "Bigger indexes retrieve better; none are cheap.",
};

/** Which language coverage — NOT on/off. On/off is the switch chain's, so with
 *  the feature off `SEMANTIC_OFF` matches no option and no segment is active. */
export const SEMANTIC_FAMILIES = [
  { value: SEMANTIC_ENGLISH, label: "English" },
  { value: SEMANTIC_MULTILINGUAL, label: "Multilingual" },
];

/** "Nomic Embed Text v1.5 (English)" → "Nomic Embed Text v1.5". The parenthetical
 *  repeats the family group the user just clicked. */
function stripTierSuffix(name: string): string {
  return name.replace(/\s*\([^()]*\)\s*$/, "").trim();
}

function shortLabel(id: string, name: string): string {
  return SHORT_LABELS[id] ?? stripTierSuffix(name);
}

// ── Transcription ──────────────────────────────────────────────────────────

/**
 * The builds of ONE transcription family — the models the picked provider
 * carries. The family group itself is the provider list, which the screen
 * already renders.
 */
export function transcriptionPicks(
  models: readonly AudioTranscriptionModelStatus[],
): PickerModel[] {
  return models.map((model) => {
    const id = model.modelId ?? OS_MANAGED_VALUE;
    const osManaged = model.management === "os_managed";
    return {
      id,
      family: model.provider,
      name: model.displayName,
      short: shortLabel(id, model.displayName),
      bytes: osManaged ? 0 : (model.download?.byteSize ?? 0),
      approx: false,
      installed: model.available,
      osManaged,
      detail:
        MEMORY_NOTES[id] ?? MEMORY_NOTES[model.provider] ?? "not measured",
    };
  });
}

// ── Semantic Search ────────────────────────────────────────────────────────

/**
 * Every embedding model, grouped by language coverage rather than by tier — the
 * tier names ("custom") describe the app's recommendation, not what the user is
 * choosing between. `status` carries install state and the app's ordering;
 * `catalog` carries the multilingual flag, which the status rows do not.
 */
export function semanticPicks(
  status: readonly SemanticSearchModelStatus[],
  catalog: readonly SemanticSearchSupportedModel[],
): PickerModel[] {
  const byId = new Map<string, SemanticSearchSupportedModel>();
  for (const model of catalog) byId.set(model.modelId, model);

  const seen = new Set<string>();
  const out: PickerModel[] = [];
  const push = (
    id: string,
    displayName: string,
    bytes: number | null,
    installed: boolean,
    multilingual: boolean,
  ) => {
    if (seen.has(id)) return;
    seen.add(id);
    const name = stripTierSuffix(displayName);
    out.push({
      id,
      family: multilingual ? SEMANTIC_MULTILINGUAL : SEMANTIC_ENGLISH,
      name,
      short: shortLabel(id, displayName),
      bytes: bytes ?? 0,
      // Every semantic figure is `approx_download_bytes` — so is every total
      // containing one.
      approx: (bytes ?? 0) > 0,
      installed,
      osManaged: false,
      detail: multilingual ? "100+ languages" : "English",
    });
  };

  for (const model of status) {
    push(
      model.modelId,
      model.displayName,
      model.approxDownloadBytes,
      model.available,
      // The catalog's flag is the Rust heuristic (tier or architecture); the
      // tier alone only classifies the two guided rows.
      byId.get(model.modelId)?.multilingual ?? model.tier === "multilingual",
    );
  }
  for (const model of catalog) {
    push(
      model.modelId,
      model.displayName,
      model.approxDownloadBytes,
      false,
      model.multilingual,
    );
  }
  return out;
}

// ── Budget ─────────────────────────────────────────────────────────────────

export interface BudgetParts {
  /** speakrs. No picker anywhere — the budget bar is the only place it shows. */
  speakerBytes: number;
  transcriptionBytes: number;
  semanticBytes: number;
  /** True when the semantic figure is an `approx_download_bytes` one. */
  semanticApprox: boolean;
}

export interface DownloadBudget extends BudgetParts {
  bytes: number;
  approx: boolean;
}

/** What a model contributes to the download: nothing, once it is on disk. */
export function pickBytes(model: PickerModel | null, enabled: boolean): number {
  if (!model || !enabled || model.installed || model.osManaged) return 0;
  return model.bytes;
}

export function downloadBudget(parts: BudgetParts): DownloadBudget {
  return {
    ...parts,
    bytes: parts.speakerBytes + parts.transcriptionBytes + parts.semanticBytes,
    // "about" is not decoration: a total carrying an approximate part IS
    // approximate. It disappears the moment the approximate part does.
    approx: parts.semanticApprox && parts.semanticBytes > 0,
  };
}

export function totalLabel(bytes: number, approx: boolean): string {
  if (bytes <= 0) return "Nothing to download";
  return `${approx ? "about " : ""}${formatBytes(bytes)}`;
}

// ── Disk ───────────────────────────────────────────────────────────────────

export interface DiskVerdict {
  /** Reserve + downloads + one day of capture (slice 2's gate term). */
  needBytes: number;
  /** What is left for downloads once the reserve and a day are set aside. */
  roomForDownloadsBytes: number;
  /** The whole sentence: figures first, an action only when it works. */
  message: string;
  /** Non-null only when turning Semantic Search off actually clears the need —
   *  since the capture term landed, it usually does not. */
  escapeSavingBytes: number | null;
}

/**
 * `null` when the downloads fit, or when free space could not be measured — an
 * inability to MEASURE never acts (ADR 0040, same discipline as `gates.ts`).
 */
export function diskVerdict(input: {
  budget: DownloadBudget;
  freeBytes: number | null;
  captureIntervalSeconds: number;
}): DiskVerdict | null {
  const { budget, freeBytes, captureIntervalSeconds } = input;
  if (freeBytes === null) return null;
  const needBytes = storageNeedBytes(budget.bytes, captureIntervalSeconds);
  const dayBytes = estimateDailyStorageMb(captureIntervalSeconds) * 1e6;
  const roomForDownloadsBytes = Math.max(
    0,
    freeBytes - RESERVE_FLOOR_BYTES - dayBytes,
  );
  if (freeBytes >= needBytes) return null;

  // The figures are the message. The old "turn Semantic Search off — 548 MB"
  // escape stopped being a reliable escape when the gate grew its capture term,
  // so the action is offered only when it genuinely clears the whole need.
  const withoutSemantic = storageNeedBytes(
    budget.bytes - budget.semanticBytes,
    captureIntervalSeconds,
  );
  const clears = budget.semanticBytes > 0 && freeBytes >= withoutSemantic;
  const tail =
    budget.semanticBytes > 0
      ? ` Turning Semantic Search off frees about ${formatBytes(budget.semanticBytes)}, which ${
          clears ? "clears it" : "is still not enough"
        }.`
      : " Every remaining download is required by a feature you turned on.";
  return {
    needBytes,
    roomForDownloadsBytes,
    message:
      `Not enough room. ${formatBytes(freeBytes)} free · ${formatBytes(needBytes)} needed —` +
      ` that includes a day of recording (${formatBytes(dayBytes)}) and the ${formatBytes(RESERVE_FLOOR_BYTES)} kept free.` +
      tail,
    escapeSavingBytes: clears ? budget.semanticBytes : null,
  };
}
