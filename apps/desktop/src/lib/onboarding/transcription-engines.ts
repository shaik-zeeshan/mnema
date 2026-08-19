// Onboarding provider choice (issue #195, slice 9) — the pure half.
//
// One RECOMMENDED engine that carries *why*, and alternatives that carry their
// price. The price is a real delta against the recommended engine and nothing
// else: download bytes straight from the manifest the backend already ships,
// plus memory from the measured table below. The three engines have never been
// benchmarked against each other in this build, so speed and accuracy are not
// ranked here and `Providers.svelte` does not draw them at all — an unmeasured
// axis is left off the screen, not drawn as an empty track.
//
// Pure: no Svelte, no `invoke`. `Providers.svelte` renders what this returns.

import { formatBytes } from "../settings/state/format";
import { defaultTranscriptionModelIdForProvider } from "../settings/state/models-format";

/** `default_audio_transcription_provider()` — `capture-types/src/recording.rs:304`. */
export const RECOMMENDED_ENGINE = "local_whisper";

// Resident memory with the engine's default model loaded. MEASURED on a running
// app — no manifest in the repo publishes a memory figure, which is why every
// readout built from this table says "about". Kept in bytes so the one app-wide
// `formatBytes` can print it; the underlying measurements are MB-granular
// (800 MB baseline / 1.4 GB Whisper base / 3 GB Parakeet).
const ENGINE_RAM_BYTES: Record<string, number> = {
  apple_speech_on_device: 800_000_000,
  local_whisper: 1_400_000_000,
  parakeet: 3_000_000_000,
};

// Apple Speech does its work in macOS's own process, so its figure is the app's
// no-model-resident baseline rather than a measurement of the engine.
const RAM_INFERRED = new Set(["apple_speech_on_device"]);

// Card copy. Each `line` restates a claim the manifest itself makes
// (`crates/audio-transcription/src/lib.rs`: base is "balanced speed, size, and
// quality"; Parakeet is "Highest memory use"; Apple Speech has "No app-managed
// download"). `name` shortens the backend's provider display name to fit a
// three-across card grid — same engines, fewer words.
const ENGINE_COPY: Record<string, { name: string; line: string; heavierLabel?: string }> = {
  apple_speech_on_device: {
    name: "Apple Speech",
    line:
      "Nothing to download and the app stays at its baseline — macOS does the work in its " +
      "own process. It transcribes the languages macOS has installed.",
  },
  local_whisper: {
    name: "Whisper",
    line: "Balanced speed, size and quality — the manifest's own reason for making Base the default.",
  },
  parakeet: {
    name: "Parakeet",
    line: "The heaviest of the three: the manifest calls it the highest memory use.",
    heavierLabel: "full-precision build",
  },
};

/** Structurally satisfied by `AudioTranscriptionModelStatus` / `OcrModelStatus`. */
export interface ModelSource {
  modelId: string | null;
  description?: string;
  available: boolean;
  download: { byteSize: number } | null;
}

/** Structurally satisfied by `AudioTranscriptionProviderStatus` / `OcrProviderStatus`. */
export interface ProviderSource {
  provider: string;
  displayName: string;
  models: readonly ModelSource[];
}

export interface Engine {
  id: string;
  name: string;
  /** Bytes still to fetch; 0 when there is nothing to fetch, null when unknown. */
  bytes: number | null;
  /** The default model is OS-managed — there is no download to have. */
  osManaged: boolean;
  installed: boolean;
  ramBytes: number | null;
  /** Memory is inferred rather than measured for this engine. */
  ramInferred: boolean;
  line: string;
  /** A bigger build of the same engine, reachable only in the model picker. */
  foot: string | null;
}

// The status command has not answered yet. The three engines are still named —
// the control must not be empty on a cold start — but no size is claimed for
// them until the manifest says one.
const LOADING_SOURCES: readonly ProviderSource[] = [
  { provider: "apple_speech_on_device", displayName: "Apple Speech", models: [] },
  { provider: "local_whisper", displayName: "Local Whisper", models: [] },
  { provider: "parakeet", displayName: "Parakeet", models: [] },
];

function defaultModel(source: ProviderSource): ModelSource | null {
  const wanted = defaultTranscriptionModelIdForProvider(source.provider);
  return source.models.find((m) => m.modelId === wanted) ?? source.models[0] ?? null;
}

/**
 * The engines onboarding may offer, in the order the backend listed them.
 *
 * Deepgram is dropped here as well as at the call site (ADR 0047): cloud
 * transcription is Settings-only behind a consent gate, and `Providers.svelte`
 * names it in a disclosure instead of ever offering it.
 */
export function buildEngines(sources: readonly ProviderSource[]): Engine[] {
  return (sources.length > 0 ? sources : LOADING_SOURCES)
    .filter((source) => source.provider !== "deepgram")
    .map((source) => {
      const copy = ENGINE_COPY[source.provider];
      const model = defaultModel(source);
      const osManaged = model !== null && model.download === null;
      const heavier = heavierBuild(source, model);
      return {
        id: source.provider,
        name: copy?.name ?? source.displayName,
        bytes: model === null ? null : model.available || osManaged ? 0 : model.download!.byteSize,
        osManaged,
        installed: model?.available ?? false,
        ramBytes: ENGINE_RAM_BYTES[source.provider] ?? null,
        ramInferred: RAM_INFERRED.has(source.provider),
        line: copy?.line ?? model?.description ?? "",
        foot:
          copy?.heavierLabel && heavier !== null
            ? `${copy.heavierLabel} ${formatBytes(heavier.bytes)} (+${formatBytes(
                heavier.extra,
              )}) — chosen in the model picker, not here`
            : null,
      };
    });
}

// The largest build of the same engine that the default is NOT. "Parakeet is
// huge" is only true of a build this screen does not select, so the card prints
// both figures instead of letting the smaller one carry the reputation.
function heavierBuild(
  source: ProviderSource,
  model: ModelSource | null,
): { bytes: number; extra: number } | null {
  const base = model?.download?.byteSize ?? 0;
  const biggest = source.models
    .filter((m) => m.modelId !== model?.modelId && m.download !== null)
    .reduce((max, m) => Math.max(max, m.download!.byteSize), 0);
  return biggest > base ? { bytes: biggest, extra: biggest - base } : null;
}

/** What the "download" metric on a card reads. */
export function downloadLabel(engine: Engine): string {
  if (engine.bytes === null) return "checking…";
  if (engine.osManaged) return "no download";
  if (engine.installed) return "already on this Mac";
  return formatBytes(engine.bytes);
}

/** What the "memory" metric on a card reads. Unmeasured stays unmeasured. */
export function memoryLabel(engine: Engine): string {
  return engine.ramBytes === null ? "not measured" : `about ${formatBytes(engine.ramBytes)}`;
}

/** 0..100 for a metric track. An unknown value fills nothing. */
export function trackWidth(value: number | null, max: number): number {
  if (value === null || max <= 0) return 0;
  return (value / max) * 100;
}

export interface EngineDelta {
  text: string;
  /** The choice costs more than the recommended engine on some axis. */
  up: boolean;
}

/**
 * What picking `id` costs against the recommended engine. An axis nobody can
 * put a number on is left out of the sentence rather than guessed at.
 */
export function engineDelta(engines: readonly Engine[], id: string): EngineDelta {
  const chosen = engines.find((e) => e.id === id);
  const recommended = engines.find((e) => e.id === RECOMMENDED_ENGINE);
  if (!chosen || !recommended) return { text: "", up: false };
  if (chosen.id === recommended.id) {
    return { text: "This is what Mnema picked. Nothing to decide unless you want to.", up: false };
  }

  const sign = (n: number) => (n > 0 ? "+" : "−");
  const parts: string[] = [];
  let up = false;

  const bytes = delta(chosen.bytes, recommended.bytes);
  if (bytes !== null && bytes !== 0) {
    parts.push(`${sign(bytes)}${formatBytes(Math.abs(bytes))} to download`);
    up ||= bytes > 0;
  }
  const ram = delta(chosen.ramBytes, recommended.ramBytes);
  if (ram !== null && ram !== 0) {
    parts.push(`about ${sign(ram)}${formatBytes(Math.abs(ram))} of memory`);
    up ||= ram > 0;
  }

  if (parts.length === 0) {
    return { text: "Costs the same as the recommended engine, as far as anything measured says.", up: false };
  }
  return { text: `${parts.join(" · ")} vs the recommended engine`, up };
}

function delta(a: number | null, b: number | null): number | null {
  return a === null || b === null ? null : a - b;
}
