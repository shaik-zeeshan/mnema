// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
//
// Slice 10 (model pickers). This is the mockup's `console.assert` self-check
// (`docs/onboarding/mockups/input-components/parts/models.part.html`) promoted
// to a real test, plus the drift guard the mockup could not have: every byte
// figure is RE-DERIVED from the Rust manifest that owns it, so a manifest edit
// fails here instead of silently making the footer lie.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  FAMILY_NOTES,
  OS_MANAGED_VALUE,
  SEMANTIC_ENGLISH,
  SEMANTIC_FAMILIES,
  SEMANTIC_MULTILINGUAL,
  SEMANTIC_OFF,
  diskVerdict,
  downloadBudget,
  pickBytes,
  semanticPicks,
  totalLabel,
  transcriptionPicks,
} from "./model-budget";
import { NOMIC_BYTES, SPEAKRS_BYTES, WHISPER_BASE_BYTES } from "./resolve-setup";
import { RESERVE_FLOOR_BYTES, storageNeedBytes } from "./gates";

// ── The manifests, re-derived ──────────────────────────────────────────────

const rustFile = (path: string) =>
  readFileSync(new URL(`../../../../../${path}`, import.meta.url), "utf8");

const TRANSCRIPTION_RS = rustFile("crates/audio-transcription/src/lib.rs");
const SPEAKER_RS = rustFile("crates/speaker-analysis/src/lib.rs");
const SEMANTIC_RS = rustFile("crates/semantic-search/src/models.rs");

const num = (raw: string) => Number(raw.replace(/_/g, ""));

/** `whisper_model("base", "Whisper Base", "…", 147_951_465, "…")`. */
function whisperBytes(id: string): number {
  const match = TRANSCRIPTION_RS.match(
    new RegExp(`whisper_model\\(\\s*"${id}",[\\s\\S]{0,600}?(\\d[\\d_]{6,}),`),
  );
  if (!match) throw new Error(`no whisper_model("${id}") in the manifest`);
  return num(match[1]);
}

/** The Parakeet artifacts declare `byte_size: files.iter()…sum()` — so sum the
 *  file list, exactly as the Rust does. */
function artifactFileSum(fnName: string): number {
  const start = TRANSCRIPTION_RS.indexOf(`fn ${fnName}()`);
  if (start < 0) throw new Error(`no fn ${fnName}() in the manifest`);
  const end = TRANSCRIPTION_RS.indexOf("\nfn ", start + 4);
  const body = TRANSCRIPTION_RS.slice(start, end < 0 ? undefined : end);
  const sizes = [...body.matchAll(/byte_size:\s*([\d_]+),/g)].map((m) => num(m[1]));
  expect(sizes.length).toBeGreaterThan(1);
  return sizes.reduce((sum, size) => sum + size, 0);
}

function semanticApproxBytes(modelId: string): number {
  const start = SEMANTIC_RS.indexOf(`model_id: "${modelId}"`);
  if (start < 0) throw new Error(`no descriptor for ${modelId}`);
  const match = SEMANTIC_RS.slice(start).match(/approx_download_bytes:\s*([\d_]+)/);
  return num(match[1]);
}

const WHISPER = {
  tiny: whisperBytes("tiny"),
  base: whisperBytes("base"),
  small: whisperBytes("small"),
  medium: whisperBytes("medium"),
};
const PARAKEET_FULL = artifactFileSum("parakeet_v3_onnx_artifact");
const PARAKEET_INT8 = artifactFileSum("parakeet_v3_onnx_int8_artifact");
const SPEAKRS = num(SPEAKER_RS.match(/byte_size:\s*(419[\d_]+),/)[1]);
const NOMIC = semanticApproxBytes("nomic-embed-text-v1.5");
const E5_SMALL = semanticApproxBytes("multilingual-e5-small");

// ── Fixtures shaped from the real types ────────────────────────────────────

const whisperModel = (id: keyof typeof WHISPER, name: string) => ({
  provider: "local_whisper",
  modelId: id,
  displayName: name,
  description: "",
  management: "app_managed",
  status: "missing",
  available: false,
  availabilityStatus: null,
  installPath: null,
  missingFiles: [],
  failureMessage: null,
  licenseLabel: null,
  sourceUrl: null,
  download: { url: "", byteSize: WHISPER[id], sha256: "", shape: null },
});

const WHISPER_FAMILY = [
  whisperModel("tiny", "Whisper Tiny"),
  whisperModel("base", "Whisper Base"),
  whisperModel("small", "Whisper Small"),
  whisperModel("medium", "Whisper Medium"),
];

const APPLE_FAMILY = [
  {
    provider: "apple_speech_on_device",
    modelId: null,
    displayName: "Apple Speech (on-device)",
    description: "",
    management: "os_managed",
    status: "os_managed",
    available: true,
    availabilityStatus: "available",
    installPath: null,
    missingFiles: [],
    failureMessage: null,
    licenseLabel: null,
    sourceUrl: null,
    download: null,
  },
];

const parakeetModel = (id: string, name: string, bytes: number) => ({
  ...whisperModel("base", name),
  provider: "parakeet",
  modelId: id,
  download: { url: "", byteSize: bytes, sha256: "", shape: null },
});

const PARAKEET_FAMILY = [
  parakeetModel("parakeet-tdt-0.6b-v3-onnx", "Parakeet TDT 0.6B v3 ONNX", PARAKEET_FULL),
  parakeetModel(
    "parakeet-tdt-0.6b-v3-onnx-int8",
    "Parakeet TDT 0.6B v3 ONNX int8",
    PARAKEET_INT8,
  ),
];

const semanticStatus = (
  modelId: string,
  displayName: string,
  tier: string,
  bytes: number,
) => ({
  provider: "local",
  modelId,
  displayName,
  description: "",
  tier,
  dimension: 768,
  maxTokens: 512,
  modelCode: modelId,
  approxDownloadBytes: bytes,
  licenseLabel: null,
  status: "missing",
  available: false,
  installPath: "",
  missingFiles: [],
});

const SEMANTIC_STATUS = [
  semanticStatus("nomic-embed-text-v1.5", "Nomic Embed Text v1.5 (English)", "english", NOMIC),
  semanticStatus(
    "multilingual-e5-small",
    "Multilingual E5 Small (Multilingual)",
    "multilingual",
    E5_SMALL,
  ),
  semanticStatus(
    "bge-m3",
    "BGE-M3 (Multilingual, Custom)",
    "custom",
    semanticApproxBytes("bge-m3"),
  ),
  semanticStatus(
    "stella_en_400M_v5",
    "Stella 400M v5 (English, Custom)",
    "custom",
    semanticApproxBytes("stella_en_400M_v5"),
  ),
  semanticStatus(
    "snowflake-arctic-embed-l-v2.0",
    "Snowflake Arctic Embed L v2.0 (Multilingual, Custom)",
    "custom",
    semanticApproxBytes("snowflake-arctic-embed-l-v2.0"),
  ),
];

const SEMANTIC_CATALOG = [
  { modelId: "nomic-embed-text-v1.5", displayName: "Nomic Embed Text v1.5 (English)", modelCode: "", dimension: 768, description: "", multilingual: false, approxDownloadBytes: NOMIC },
  { modelId: "multilingual-e5-small", displayName: "Multilingual E5 Small (Multilingual)", modelCode: "", dimension: 384, description: "", multilingual: true, approxDownloadBytes: E5_SMALL },
  { modelId: "bge-m3", displayName: "BGE-M3 (Multilingual, Custom)", modelCode: "", dimension: 1024, description: "", multilingual: true, approxDownloadBytes: semanticApproxBytes("bge-m3") },
  { modelId: "stella_en_400M_v5", displayName: "Stella 400M v5 (English, Custom)", modelCode: "", dimension: 1024, description: "", multilingual: false, approxDownloadBytes: semanticApproxBytes("stella_en_400M_v5") },
  { modelId: "snowflake-arctic-embed-l-v2.0", displayName: "Snowflake Arctic Embed L v2.0 (Multilingual, Custom)", modelCode: "", dimension: 1024, description: "", multilingual: true, approxDownloadBytes: semanticApproxBytes("snowflake-arctic-embed-l-v2.0") },
];

// ── The manifest sums the mockup pinned ────────────────────────────────────

describe("slice 10 — the manifest figures the pickers print", () => {
  test("Whisper's four sizes match the manifest, and base is the default", () => {
    expect(WHISPER).toEqual({
      tiny: 77_691_713,
      base: 147_951_465,
      small: 487_601_967,
      medium: 1_533_763_059,
    });
    expect(WHISPER_BASE_BYTES).toBe(WHISPER.base);
  });

  test("both Parakeet artifact sums match, and int8 is +523 MB over Whisper base", () => {
    expect(PARAKEET_INT8).toBe(670_619_803);
    expect(PARAKEET_FULL).toBe(2_549_945_719);
    // The honest delta from the default: NOT the full-precision build's.
    expect(Math.round((PARAKEET_INT8 - WHISPER.base) / 1e6)).toBe(523);
  });

  test("speakrs and nomic match the constants the resolver already carries", () => {
    expect(SPEAKRS).toBe(SPEAKRS_BYTES);
    expect(SPEAKRS).toBe(419_482_724);
    expect(NOMIC).toBe(NOMIC_BYTES);
    expect(NOMIC).toBe(548_000_000);
  });

  test("Semantic Search has no cheap end — the smallest model saves 60 MB", () => {
    expect(NOMIC - E5_SMALL).toBe(60_000_000);
  });

  test("the default set is about 1.1 GB", () => {
    const total = WHISPER.base + NOMIC + SPEAKRS;
    expect(total).toBe(1_115_434_189);
    expect(totalLabel(total, true)).toBe("about 1.1 GB");
  });
});

// ── Catalog shape: every model has to be reachable ─────────────────────────

describe("slice 10 — every model is reachable", () => {
  test("all seven transcription builds appear across the three families", () => {
    const families = [APPLE_FAMILY, WHISPER_FAMILY, PARAKEET_FAMILY];
    const picks = families.flatMap((family) => transcriptionPicks(family));
    expect(picks).toHaveLength(7);
    expect(picks.map((m) => m.short)).toEqual([
      "Apple Speech",
      "Tiny",
      "Base",
      "Small",
      "Medium",
      "Full precision",
      "int8",
    ]);
    // Apple Speech is OS-managed: no id, no download, no invented memory figure.
    expect(picks[0].id).toBe(OS_MANAGED_VALUE);
    expect(picks[0].osManaged).toBe(true);
    expect(picks[0].bytes).toBe(0);
    expect(picks[0].detail).toBe("macOS handles it");
    // No manifest carries an int8 memory figure, so it says so.
    expect(picks[6].detail).toBe("below Parakeet full precision");
  });

  test("all five semantic models appear, grouped by language coverage", () => {
    const picks = semanticPicks(SEMANTIC_STATUS, SEMANTIC_CATALOG);
    expect(picks).toHaveLength(5);
    expect(picks.filter((m) => m.family === SEMANTIC_ENGLISH).map((m) => m.short)).toEqual([
      "Nomic",
      "Stella",
    ]);
    expect(
      picks.filter((m) => m.family === SEMANTIC_MULTILINGUAL).map((m) => m.short),
    ).toEqual(["E5 Small", "BGE-M3", "Snowflake"]);
    // The tier alone would misfile Stella (English) and the two custom
    // multilingual builds; the catalog's flag is what classifies them.
    expect(picks.find((m) => m.id === "stella_en_400M_v5").family).toBe(SEMANTIC_ENGLISH);
    expect(picks.find((m) => m.id === "bge-m3").family).toBe(SEMANTIC_MULTILINGUAL);
    // Every semantic size is approximate.
    expect(picks.every((m) => m.approx)).toBe(true);
  });

  test("the family group offers language coverage only, never Off", () => {
    expect(SEMANTIC_FAMILIES.map((f) => f.value)).toEqual([
      SEMANTIC_ENGLISH,
      SEMANTIC_MULTILINGUAL,
    ]);
  });

  test("a disabled feature resolves to a family no segment carries", () => {
    // That is how "off" reads as NO active segment rather than a wrong one —
    // and the height-reserved row still owes the reader its sentence.
    expect(SEMANTIC_FAMILIES.find((f) => f.value === SEMANTIC_OFF)).toBeUndefined();
    expect(FAMILY_NOTES[SEMANTIC_OFF]).toBeTruthy();
  });

  test("a catalog-only model still reaches a family", () => {
    const picks = semanticPicks([], SEMANTIC_CATALOG);
    expect(picks).toHaveLength(5);
    expect(picks.every((m) => m.installed)).toBe(false);
  });
});

// ── The budget ─────────────────────────────────────────────────────────────

const pick = (list, id) => list.find((m) => m.id === id) ?? null;

function budgetOf(transcriptionId: string, semanticId: string | null, over = {}) {
  const t = pick(transcriptionPicks(WHISPER_FAMILY.concat(PARAKEET_FAMILY, APPLE_FAMILY)), transcriptionId);
  const s = semanticId
    ? pick(semanticPicks(SEMANTIC_STATUS, SEMANTIC_CATALOG), semanticId)
    : null;
  return downloadBudget({
    speakerBytes: SPEAKRS,
    transcriptionBytes: pickBytes(t, true),
    semanticBytes: pickBytes(s, s !== null),
    semanticApprox: s?.approx ?? false,
    ...over,
  });
}

describe("slice 10 — the download budget", () => {
  test("the default set reads 'about 1.1 GB'", () => {
    const budget = budgetOf("base", "nomic-embed-text-v1.5");
    expect(budget.bytes).toBe(1_115_434_189);
    expect(totalLabel(budget.bytes, budget.approx)).toBe("about 1.1 GB");
  });

  test("'about' disappears with nomic — nothing approximate is left in the sum", () => {
    const budget = budgetOf("base", null);
    expect(budget.approx).toBe(false);
    expect(budget.bytes).toBe(WHISPER.base + SPEAKRS);
    expect(totalLabel(budget.bytes, budget.approx)).not.toContain("about");
  });

  test("an installed or OS-managed model contributes nothing", () => {
    const installed = { ...pick(transcriptionPicks(WHISPER_FAMILY), "base"), installed: true };
    expect(pickBytes(installed, true)).toBe(0);
    expect(pickBytes(pick(transcriptionPicks(APPLE_FAMILY), OS_MANAGED_VALUE), true)).toBe(0);
    // A feature that is off downloads nothing either.
    expect(pickBytes(pick(transcriptionPicks(WHISPER_FAMILY), "base"), false)).toBe(0);
  });

  test("the total moves with every pick", () => {
    const base = budgetOf("base", "nomic-embed-text-v1.5").bytes;
    expect(budgetOf("medium", "nomic-embed-text-v1.5").bytes).toBe(
      base - WHISPER.base + WHISPER.medium,
    );
    expect(budgetOf("parakeet-tdt-0.6b-v3-onnx-int8", "nomic-embed-text-v1.5").bytes).toBe(
      base - WHISPER.base + PARAKEET_INT8,
    );
    expect(budgetOf("base", "bge-m3").bytes).toBe(
      base - NOMIC + semanticApproxBytes("bge-m3"),
    );
    expect(budgetOf("base", "nomic-embed-text-v1.5", { speakerBytes: 0 }).bytes).toBe(
      base - SPEAKRS,
    );
  });

  test("nothing selected reads 'Nothing to download'", () => {
    expect(totalLabel(0, false)).toBe("Nothing to download");
  });
});

// ── The over-disk state ────────────────────────────────────────────────────

describe("slice 10 — the over-disk state", () => {
  const budget = budgetOf("base", "nomic-embed-text-v1.5");
  const INTERVAL_S = 2;
  const need = storageNeedBytes(budget.bytes, INTERVAL_S);

  test("a roomy volume, or an unmeasured one, says nothing", () => {
    expect(diskVerdict({ budget, freeBytes: need, captureIntervalSeconds: INTERVAL_S })).toBeNull();
    expect(
      diskVerdict({ budget, freeBytes: null, captureIntervalSeconds: INTERVAL_S }),
    ).toBeNull();
  });

  test("the shortfall states the figures, including the reserve and a day", () => {
    const verdict = diskVerdict({
      budget,
      freeBytes: 2.5e9,
      captureIntervalSeconds: INTERVAL_S,
    });
    expect(verdict.needBytes).toBe(need);
    expect(verdict.message).toContain("Not enough room.");
    // Consistent with the sibling copy the storage screen already prints.
    expect(verdict.message).toContain("a day of recording (405.0 MB)");
    expect(verdict.message).toContain("1.1 GB kept free");
    // The bar's marker is what is left for downloads, not raw free space —
    // otherwise the picture would say "fits" while the sentence says it doesn't.
    expect(verdict.roomForDownloadsBytes).toBe(2.5e9 - RESERVE_FLOOR_BYTES - 405e6);
    // A volume with less than the reserve has no room at all, never a negative.
    expect(
      diskVerdict({ budget, freeBytes: 620e6, captureIntervalSeconds: INTERVAL_S })
        .roomForDownloadsBytes,
    ).toBe(0);
  });

  // A MEASURED zero is the most determined free-space reading there is, but
  // `formatBytes` renders it with its can't-determine sentinel — so the picker's
  // shortfall on a full volume read "unknown size free", which `diskVerdict`
  // returns `null` for when the space genuinely could not be measured.
  test("a full volume states zero free space, not the can't-determine sentinel", () => {
    const verdict = diskVerdict({ budget, freeBytes: 0, captureIntervalSeconds: INTERVAL_S });
    expect(verdict).not.toBeNull();
    expect(verdict.message).not.toContain("unknown size");
    expect(verdict.message).toContain("0 B free");
  });

  test("the escape is offered ONLY when turning Semantic Search off clears it", () => {
    // Since the gate grew its capture term, dropping nomic usually does not
    // clear the check — so the copy states the fact instead of offering a
    // button that fails.
    const narrow = diskVerdict({
      budget,
      freeBytes: storageNeedBytes(budget.bytes - budget.semanticBytes, INTERVAL_S),
      captureIntervalSeconds: INTERVAL_S,
    });
    expect(narrow.escapeSavingBytes).toBe(budget.semanticBytes);
    expect(narrow.message).toContain("which clears it");
    // …and the escape genuinely lands under free disk.
    const escaped = downloadBudget({
      speakerBytes: budget.speakerBytes,
      transcriptionBytes: budget.transcriptionBytes,
      semanticBytes: 0,
      semanticApprox: false,
    });
    expect(
      diskVerdict({
        budget: escaped,
        freeBytes: storageNeedBytes(budget.bytes - budget.semanticBytes, INTERVAL_S),
        captureIntervalSeconds: INTERVAL_S,
      }),
    ).toBeNull();

    const hopeless = diskVerdict({
      budget,
      freeBytes: 620e6,
      captureIntervalSeconds: INTERVAL_S,
    });
    expect(hopeless.escapeSavingBytes).toBeNull();
    expect(hopeless.message).toContain("is still not enough");
  });

  test("with Semantic Search already off, no escape is invented", () => {
    const withoutSemantic = budgetOf("medium", null);
    const verdict = diskVerdict({
      budget: withoutSemantic,
      freeBytes: 620e6,
      captureIntervalSeconds: INTERVAL_S,
    });
    expect(verdict.escapeSavingBytes).toBeNull();
    expect(verdict.message).toContain(
      "Every remaining download is required by a feature you turned on.",
    );
  });
});
