// Privacy-slice sync for the onboarding draft. Kept in its OWN module with only
// `import type` (no $lib/rune runtime imports) so it is unit-testable under
// `bun test` without dragging in theme.svelte / Svelte runes.
//
// The onboarding privacy row has NO backend `privacy.enabled` field; excluded
// apps are ALWAYS persisted from `draftExcludedApps` regardless of the toggle.
// `privacyEnabled` is a frontend-only flag (drives the row toggle, dim-when-off,
// and the footer "features on" count). Derive it from the excluded-apps list so a
// returning user (or "Apply recommended") with >=1 excluded app lights the row.
// MONOTONIC: the row is auto-turned ON only via the load-derive (a returning
// user with >=1 excluded app) and "Apply recommended" (which issues the privacy
// command directly); the in-editor add can't flip it ON because the editor is
// dimmed until the user enables the row first. Removing the last app does NOT
// auto-turn-OFF (that would dim the editor mid-edit) — the user toggles off
// explicitly via the row switch.
import type { ExcludedAppEntry, RecordingSettings } from "$lib/types";

export interface PrivacyDraftTarget {
  draftExcludedApps: ExcludedAppEntry[];
  privacyEnabled: boolean;
}

export function syncPrivacyDraftInto(
  draft: PrivacyDraftTarget,
  next: RecordingSettings,
): void {
  draft.draftExcludedApps = [...(next.privacy?.excludedApps ?? [])];
  draft.privacyEnabled = draft.privacyEnabled || draft.draftExcludedApps.length > 0;
}
