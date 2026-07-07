// The onboarding accordion's feature catalog. One entry per capability row,
// in display order. Slice 3's controller keys all per-feature state off the
// `FeatureId` union; the row chrome (FeatureRow) renders the static metadata.
import type { IconName } from "$lib/settings/groups";

export type FeatureId =
  | "permissions"
  | "screen"
  | "storage"
  | "mic"
  | "sysaudio"
  | "ocr"
  | "transcribe"
  | "speakers"
  | "privacy"
  | "askai"
  | "semanticSearch"
  | "licensing";

export interface FeatureMeta {
  id: FeatureId;
  icon: IconName;
  name: string;
  eyebrow: string;
  sub: string;
  required: boolean;
}

// Display order is the array order. Required features are always-on and locked
// (no toggle); optional ones flip the controller's backing draft field.
export const FEATURES: FeatureMeta[] = [
  {
    id: "permissions",
    icon: "access",
    name: "Permissions",
    eyebrow: "Foundation",
    sub: "Mnema needs OS access to capture your screen and audio.",
    required: true,
  },
  {
    id: "screen",
    icon: "capture",
    name: "Screen capture",
    eyebrow: "What gets recorded",
    sub: "Resolution, bitrate, frame rate and segment length for the screen video stream.",
    required: true,
  },
  {
    id: "storage",
    icon: "storage",
    name: "Storage & retention",
    eyebrow: "Where it lives",
    sub: "Pick a save location and how long captures stick around before cleanup.",
    required: true,
  },
  {
    id: "mic",
    icon: "audio",
    name: "Microphone capture",
    eyebrow: "Optional",
    sub: "Record and transcribe your microphone alongside the screen.",
    required: false,
  },
  {
    id: "sysaudio",
    icon: "speakers",
    name: "System audio",
    eyebrow: "Optional · needs screen",
    sub: "Capture sound coming out of your speakers (macOS).",
    required: false,
  },
  {
    id: "ocr",
    icon: "ocr",
    name: "OCR — read on-screen text",
    eyebrow: "Optional · recommended",
    sub: "Make every frame searchable by reading the text it shows.",
    required: false,
  },
  {
    id: "transcribe",
    icon: "transcription",
    name: "Audio transcription",
    eyebrow: "Optional · recommended",
    sub: "Turn captured speech into searchable transcripts.",
    required: false,
  },
  {
    id: "speakers",
    icon: "speakers",
    name: "Speaker separation",
    eyebrow: "Optional · needs transcription",
    sub: "Split a transcript by who's talking — all on-device.",
    required: false,
  },
  {
    id: "privacy",
    icon: "privacy",
    name: "Privacy — excluded apps",
    eyebrow: "Optional",
    sub: "Apps on this list are never captured — windows, audio, or text.",
    required: false,
  },
  {
    id: "askai",
    icon: "askAi",
    name: "Ask AI — Reasoning Engine",
    eyebrow: "Optional · advanced",
    sub: "Ask questions about your recorded history in natural language.",
    required: false,
  },
  {
    id: "semanticSearch",
    icon: "semanticSearch",
    name: "Semantic Search",
    eyebrow: "Optional · advanced",
    sub: "Meaning-based search fused with keyword search — runs fully on-device. Pick a model to activate.",
    required: false,
  },
  {
    // Purely explanatory: the trial starts at first Capture (not here), so this
    // row takes no key and starts nothing. Modeled `required` — an always-on,
    // no-toggle info row (like permissions/screen/storage) — so it never carries
    // an attention item and can't block the finale. No backing draft field.
    id: "licensing",
    icon: "about",
    name: "Trial & ownership",
    eyebrow: "Included · one-time purchase",
    sub: "A 30-day free trial that starts when you first record — buy once, own it.",
    required: true,
  },
];

// ── Feature dependency relations ───────────────────────────────────────────
// A feature can only be ENABLED once its prerequisite is met (turning a feature
// OFF is always allowed — that gating lives in the controller). This module owns
// the pure relation logic so it stays testable and the controller just supplies
// the live context.
export interface FeatureLockContext {
  micGranted: boolean;
  systemAudioGranted: boolean;
  transcriptionEnabled: boolean;
}

// Why an optional feature can't be enabled yet (unmet prerequisite), or null if
// it can be enabled. Required features and features with no prerequisite → null.
export function featureLockReason(id: FeatureId, ctx: FeatureLockContext): string | null {
  switch (id) {
    case "mic":
      return ctx.micGranted ? null : "Needs Microphone permission";
    case "sysaudio":
      return ctx.systemAudioGranted ? null : "Needs System audio permission";
    case "speakers":
      return ctx.transcriptionEnabled ? null : "Needs Audio transcription on";
    default:
      return null;
  }
}
