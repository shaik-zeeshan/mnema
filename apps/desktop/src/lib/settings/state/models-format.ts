// Pure status/label/format helpers for the Processing panel's model surfaces
// (OCR, transcription, speakers, semantic search). No runes, no `invoke`.

import type {
  AppleSpeechOnDeviceAvailabilityStatus,
  AudioTranscriptionModelStatus,
  OcrModelStatus,
  SemanticSearchModelDownloadProgress,
  SpeakerAnalysisModelStatus,
} from "$lib/types";

export function ocrStatusLabel(model: OcrModelStatus): string {
  if (model.available) return "Available";
  if (model.status === "os_managed") return "OS managed";
  if (model.status === "installed") return "Installed";
  if (model.status === "downloading") return "Downloading";
  if (model.status === "failed") return "Failed";
  return "Missing";
}

export function appleSpeechPermissionLabel(status: AppleSpeechOnDeviceAvailabilityStatus): string {
  switch (status) {
    case "available": return "Permission granted";
    case "permission_not_determined": return "Permission not requested";
    case "permission_denied": return "Permission denied";
    case "permission_restricted": return "Permission restricted";
    case "unsupported_platform": return "Unsupported platform";
    case "framework_unavailable": return "Speech framework unavailable";
    case "recognizer_unavailable": return "Recognizer unavailable";
    case "on_device_recognition_unavailable": return "On-device recognition unavailable";
  }
}

export function appleSpeechPermissionHint(status: AppleSpeechOnDeviceAvailabilityStatus): string {
  switch (status) {
    case "available":
      return "macOS has granted Speech Recognition permission for Mnema.";
    case "permission_not_determined":
      return "Mnema has not asked macOS for Speech Recognition permission yet. Request it before recording with Apple Speech selected.";
    case "permission_denied":
      return "macOS denied Speech Recognition permission. Enable it in System Settings → Privacy & Security → Speech Recognition, then refresh.";
    case "permission_restricted":
      return "macOS reports Speech Recognition permission is restricted by policy or parental controls.";
    default:
      return "Apple Speech cannot be used until this macOS availability check passes.";
  }
}

export function transcriptionStatusLabel(model: AudioTranscriptionModelStatus): string {
  if (model.provider === "apple_speech_on_device" && model.availabilityStatus) {
    return appleSpeechPermissionLabel(model.availabilityStatus);
  }
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

export function semanticSearchTierLabel(tier: string): string {
  if (tier === "english") return "English";
  if (tier === "multilingual") return "Multilingual";
  return "Custom";
}

export function semanticSearchProgressPercent(p: SemanticSearchModelDownloadProgress): number {
  if (!p.totalBytes || p.totalBytes <= 0) return 0;
  return Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100));
}

// Default model id for an OCR provider (tesseract has a bundled default;
// apple_vision is OS-managed → null).
export function defaultOcrModelIdForProvider(provider: string): string | null {
  if (provider === "tesseract") return "tesseract-5.5.2";
  return null;
}

export function defaultOcrLanguageForProvider(provider: string): string | null {
  if (provider === "tesseract") return "eng";
  return null;
}

export function defaultTranscriptionModelIdForProvider(provider: string): string | null {
  if (provider === "local_whisper") return "base";
  if (provider === "parakeet") return "parakeet-tdt-0.6b-v3-onnx-int8";
  return null;
}

// A Speaker Model Preset is keyed by (provider, modelId); `__os_managed__`
// stands in for a null modelId.
export function speakerPresetKey(provider: string, modelId: string | null): string {
  return `${provider}::${modelId ?? "__os_managed__"}`;
}
