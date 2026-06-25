// Pure attention / validation / permission-display helpers for the onboarding
// flow. Lifted 1:1 out of `OnboardingController` (only `this.x` → parameters) to
// keep that file under the size budget. No reactive state lives here — these are
// stateless transforms the controller composes from its `$derived`s and the
// per-feature bodies render. Behavior is byte-identical to the inlined versions.
import type { PermissionStatus } from "$lib/types";

// PermissionValue mirrors the legacy page: the backend may return statuses the
// `PermissionStatus` union doesn't model, plus the synthetic "unsupported". The
// controller re-exports both (its public name) so body components keep importing
// them from `onboarding.svelte`; they live here so the lifecycle/listener
// helpers can share the exact same types without circling back to the controller.
export type PermissionValue = PermissionStatus | "unsupported" | "unknown";
export type PermissionKey = "screen" | "microphone" | "systemAudio";

// Granted/unsupported need no action. macOS won't re-prompt once denied, so
// those rows deep-link to System Settings instead of re-requesting.
export function permissionActionFor(
  value: PermissionValue | undefined,
): { label: string; mode: "request" | "settings" } | null {
  if (value === "granted" || value === "unsupported") return null;
  if (value === "denied" || value === "restricted") return { label: "Open Settings", mode: "settings" };
  // The synthetic "unknown" is only ever emitted on Windows, where the per-app
  // microphone privacy toggle can't be read and can only be changed in Settings.
  // Routing through `request_capture_permission` would be a silent no-op (it just
  // re-checks for an endpoint), so deep-link to the privacy pane instead.
  if (value === "unknown") return { label: "Open Settings", mode: "settings" };
  return { label: "Grant access", mode: "request" };
}

export function permissionLabelFor(value: PermissionValue | undefined): string {
  switch (value) {
    case "granted": return "Granted";
    case "denied": return "Denied";
    case "not_determined": return "Not requested";
    case "restricted": return "Restricted";
    case "unsupported": return "Unsupported";
    // Windows can't read the per-app mic toggle, so the backend reports a
    // best-effort "unknown". Surfacing the raw "Unknown" reads as a fault; frame
    // it as system-controlled instead ("unknown" is Windows-only).
    case "unknown": return "Managed by Windows";
    default: return "Unknown";
  }
}

export function permissionToneFor(
  value: PermissionValue | undefined,
): "ok" | "pending" | "blocked" {
  if (value === "granted") return "ok";
  if (value === "not_determined") return "pending";
  // The Windows "unknown" mic state is informational, not a denial — keep it on
  // the neutral (amber) tone rather than the red "blocked" one.
  if (value === "unknown") return "pending";
  return "blocked";
}

// A capture source may start when its permission is granted, or when the OS
// reports the synthetic "unknown" — the Windows-only state for the per-app
// microphone toggle the backend cannot read. Windows enforces the real grant at
// the device level (a blocked mic surfaces gracefully at capture time), so
// "unknown" must not pre-emptively disable the source the way "denied" does. On
// macOS "unknown" never occurs, so this is identical to `=== "granted"` there.
export function permissionPermitsCapture(value: PermissionValue | undefined): boolean {
  return value === "granted" || value === "unknown";
}

// ── Custom resolution / bitrate validation ─────────────────────────────────
// The clamp ranges match the Settings page's `recording-validation`
// (width/height 16-8192, mbps 1-40) so the two surfaces agree on what a valid
// custom resolution/bitrate is. The controller already parses raw input into the
// nullable draft numbers; here we only translate "null while custom mode is on"
// into the user-facing error strings.
export function customResolutionErrors(
  resolutionMode: string,
  customWidth: number | null,
  customHeight: number | null,
): string[] {
  if (resolutionMode !== "custom") return [];
  const errors: string[] = [];
  if (customWidth === null) errors.push("Width must be between 16 and 8192 pixels.");
  if (customHeight === null) errors.push("Height must be between 16 and 8192 pixels.");
  return errors;
}

export function customBitrateErrors(
  bitrateMode: string,
  customMbps: number | null,
): string[] {
  if (bitrateMode !== "custom") return [];
  return customMbps === null
    ? ["Bitrate must be a whole number from 1 to 40 Mbps."]
    : [];
}

// ── Per-feature model "needs attention" predicates ─────────────────────────
// A model is "not available" for attention/finish purposes when its feature is
// on but the selected model isn't ready. Each predicate only reads `available`,
// so a minimal `{ available }` view keeps them decoupled from the four distinct
// model-status shapes.
type ModelAvailability = { available: boolean } | null | undefined;

// OCR: needs attention whenever the feature is on and the selected model isn't
// available — whether unselected, missing, downloading, or failed. A completed
// download flips `available` true on the next status reload (mirrors the
// transcription/speaker rules).
export function ocrModelNeedsAttention(
  enabled: boolean,
  model: ModelAvailability,
): boolean {
  if (!enabled) return false;
  if (!model) return true;
  return !model.available;
}

export function transcriptionModelNeedsAttention(
  enabled: boolean,
  model: ModelAvailability,
): boolean {
  if (!enabled) return false;
  if (!model) return true;
  return !model.available;
}

export function speakerModelNeedsAttention(
  enabled: boolean,
  model: ModelAvailability,
): boolean {
  if (!enabled) return false;
  if (!model) return true;
  return !model.available;
}

// Semantic search is inert until a model is installed: enabled but no model
// selected, or the selected model isn't downloaded yet (mirrors the
// transcription attention rule). A completed download flips `available` true on
// the next status reload.
export function semanticSearchModelNeedsAttention(
  enabled: boolean,
  model: ModelAvailability,
): boolean {
  if (!enabled) return false;
  if (!model) return true;
  return !model.available;
}
