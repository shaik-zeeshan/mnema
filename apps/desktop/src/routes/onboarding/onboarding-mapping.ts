// Pure mapping/parsing helpers for the onboarding flow. Lifted 1:1 from the
// legacy `routes/onboarding/+page.svelte` so the settings round-trip
// (syncDrafts ⇄ buildSettingsRequest) stays byte-identical. No reactive state
// lives here — only stateless transforms the controller composes.
import type {
  AudioTranscriptionModelStatus,
  AudioTranscriptionProvider,
  OcrModelStatus,
  OcrProvider,
  SpeakerAnalysisModelStatus,
} from "$lib/types";

export const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
export const AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT =
  "audio_transcription_model_download_progress";
export const OCR_MODEL_DOWNLOAD_PROGRESS_EVENT = "ocr_model_download_progress";
export const SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT =
  "speaker_analysis_model_download_progress";
export const SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT =
  "semantic_search_model_download_progress";

// speakrs is the sole on-device diarization provider; pyannote-community-1 +
// WeSpeaker is its default preset (mirrors recording.svelte.ts defaults).
export const DEFAULT_SPEAKER_PROVIDER = "speakrs";
export const DEFAULT_SPEAKER_MODEL_ID = "pyannote-community-1-wespeaker";

export function serializeError(err: unknown): string {
  return typeof err === "string" ? err : (JSON.stringify(err) ?? "Unknown error");
}

export function parsePositiveInteger(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number.parseInt(trimmed, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

export function defaultOcrModelIdForProvider(provider: OcrProvider): string | null {
  if (provider === "tesseract") return "tesseract-5.5.2";
  return null;
}

export function defaultOcrLanguageForProvider(provider: OcrProvider): string | null {
  if (provider === "tesseract") return "eng";
  return null;
}

export function defaultTranscriptionModelIdForProvider(
  provider: AudioTranscriptionProvider,
): string | null {
  if (provider === "local_whisper") return "base";
  if (provider === "parakeet") return "parakeet-tdt-0.6b-v3-onnx-int8";
  return null;
}

export function ocrStatusLabel(model: OcrModelStatus): string {
  if (model.available) return "Available";
  if (model.status === "os_managed") return "OS managed";
  if (model.status === "installed") return "Installed";
  if (model.status === "downloading") return "Downloading";
  if (model.status === "failed") return "Failed";
  return "Missing";
}

export function transcriptionStatusLabel(
  model: AudioTranscriptionModelStatus,
): string {
  if (model.status === "os_managed") return "OS managed";
  if (model.status === "installed") return "Installed";
  if (model.status === "downloading") return "Downloading";
  if (model.status === "failed") return "Failed";
  return "Missing";
}

export function speakerStatusLabel(model: SpeakerAnalysisModelStatus): string {
  if (model.status === "installed") return "Installed";
  if (model.status === "downloading") return "Downloading";
  if (model.status === "failed") return "Failed";
  if (model.status === "incomplete") return "Incomplete";
  return "Missing";
}

// A speaker model preset is keyed by (provider, modelId); `__os_managed__`
// stands in for a null modelId. Mirrors models-format.ts so onboarding's
// preset picker round-trips the same way the Settings panel does.
export function speakerPresetKey(provider: string, modelId: string | null): string {
  return `${provider}::${modelId ?? "__os_managed__"}`;
}

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "unknown size";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatDuration(v: number): string {
  if (v >= 60) {
    const m = Math.floor(v / 60);
    const s = v % 60;
    return s ? `${m}m ${s}s` : `${m}m`;
  }
  return `${v}s`;
}
