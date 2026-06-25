// Pure, runtime-free core of the Settings autosave engine.
//
// Everything here is plain data + pure functions so it is unit-testable without
// Svelte's reactive runtime or Tauri's `invoke`. The reactive engine
// (`autosave.svelte.ts`) layers debouncing and effect tracking on top of these
// primitives. Keep this file free of `$state`/`$derived`/`$effect` and of any
// IPC — that is the whole point of the split.

import type { SettingsOwnershipDomain } from "$lib/types";

// The recording-settings domains that autosave (each maps to one update
// command). `app_privacy_exclusion` is a *draft* domain (it has a snapshot and a
// baseline) but it is NOT autosaved through a generic command — it is committed
// through the dedicated app-privacy-exclusion controller — so it is excluded
// here and added back only to the draft-domain list below.
export type AutosaveRecordingDomain = Extract<
  SettingsOwnershipDomain,
  | "capture_sources"
  | "capture_timing"
  | "video"
  | "storage"
  | "display"
  | "metadata"
  | "inactivity"
  | "processing"
  | "developer"
  | "access"
  | "ai_runtime"
  | "user_context"
>;

// Every domain that owns a draft snapshot + baseline. The autosave domains plus
// the privacy-exclusion domain (which diffs/baselines but commits separately).
export type RecordingSettingsDraftDomain =
  | AutosaveRecordingDomain
  | "app_privacy_exclusion";

export const RECORDING_AUTOSAVE_DOMAINS: readonly AutosaveRecordingDomain[] = [
  "capture_sources",
  "capture_timing",
  "video",
  "storage",
  "display",
  "metadata",
  "inactivity",
  "processing",
  "developer",
  "access",
  "ai_runtime",
  "user_context",
];

export const RECORDING_DRAFT_DOMAINS: readonly RecordingSettingsDraftDomain[] = [
  ...RECORDING_AUTOSAVE_DOMAINS,
  "app_privacy_exclusion",
];

// The Tauri command each autosave domain persists through. The engine looks the
// command up here when a dirty domain's debounce fires.
export const RECORDING_DOMAIN_COMMANDS: Record<AutosaveRecordingDomain, string> = {
  capture_sources: "update_capture_source_settings",
  capture_timing: "update_capture_timing_settings",
  video: "update_video_settings",
  storage: "update_storage_settings",
  display: "update_display_settings",
  metadata: "update_metadata_settings",
  inactivity: "update_inactivity_settings",
  processing: "update_processing_settings",
  developer: "update_developer_settings",
  access: "update_access_settings",
  ai_runtime: "update_ai_runtime_settings",
  user_context: "update_user_context_settings",
};

// Debounce windows (ms). Recording domains share one window; the microphone
// controller uses a shorter one because its applies are cheaper/snappier.
export const RECORDING_AUTOSAVE_DEBOUNCE_MS = 450;
export const MIC_AUTOSAVE_DEBOUNCE_MS = 250;

// Build the initial per-domain record (e.g. saving flags `false`, baselines
// `null`). Pure: callers supply the seed value.
export function makeRecordingDomainState<T>(value: T): Record<RecordingSettingsDraftDomain, T> {
  return Object.fromEntries(
    RECORDING_DRAFT_DOMAINS.map((domain) => [domain, value]),
  ) as Record<RecordingSettingsDraftDomain, T>;
}

// Is `domain` one of the draft domains? (Narrows an arbitrary ownership domain
// arriving from a backend event.)
export function isRecordingDraftDomain(
  domain: SettingsOwnershipDomain | string,
): domain is RecordingSettingsDraftDomain {
  return (RECORDING_DRAFT_DOMAINS as readonly string[]).includes(domain);
}

// The autosave command for a domain (never undefined for an autosave domain).
export function domainCommand(domain: AutosaveRecordingDomain): string {
  return RECORDING_DOMAIN_COMMANDS[domain];
}

// A draft is dirty when it has an established baseline and its current snapshot
// diverges from it. A `null` baseline means "not yet loaded" — never dirty, so
// the engine stays quiet until the first sync establishes the baseline.
export function isDirty(current: string, baseline: string | null): boolean {
  return baseline !== null && current !== baseline;
}

// The single gate the engine consults before (re)scheduling or firing a save.
// A domain is allowed to save iff: it is dirty, its domain-specific validation
// does not block it, no privacy command is mid-flight, and it is not already
// saving. Pure — every input is passed in, nothing is read from runtime state.
export function shouldSaveDomain(args: {
  current: string;
  baseline: string | null;
  blocked: boolean;
  privacyCommandInFlight: boolean;
  saving: boolean;
}): boolean {
  if (!isDirty(args.current, args.baseline)) return false;
  if (args.blocked) return false;
  if (args.privacyCommandInFlight) return false;
  if (args.saving) return false;
  return true;
}
