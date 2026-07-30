/**
 * The copy of the "Never recorded" sentence-as-control (issue #195, slice 7).
 *
 * Ported from `docs/onboarding/mockups/input-components/parts/excluded.part.html`
 * — that mockup is the design of record. Everything here is pure so the grammar
 * can be tested without a DOM; `ExcludedApps.svelte` only renders it.
 *
 * The one rule the whole control rests on: a struck app KEEPS ITS SLOT. Striking
 * is `set_privacy_excluded_app_enabled(sourceId, false)`, so the entry stays in
 * `PrivacySettings.excluded_apps` and the sentence never reflows.
 */
import { activeExclusionBundleIds, isPendingExclusion, normalizedSearchValue } from "../app-privacy-exclusion";
import type { PrivacyAppCandidate } from "../app-privacy-exclusion";
import type { ExcludedAppEntry } from "../types";

/**
 * The audio half is gated on Settings → Privacy → filter system audio:
 * `system_audio_privacy_excluded_bundle_ids()`
 * (`apps/desktop/src-tauri/src/native_capture/privacy.rs:89`) withholds the whole
 * privacy list when that toggle is off. Never claim parity without the qualifier.
 */
export const AUDIO_CLAIM =
  "Nothing from them is recorded — not the screen, and not system audio while that filter is on.";

/**
 * ponytail: the sentence reads well to 8 apps (measured in the mockup at 640px
 * and 560px: 3–8 is two lines and still reads as prose; 9–11 is three lines and
 * has become a list wearing sentence punctuation; 12+ should not be a sentence).
 * Onboarding seeds three, so the shipped range is inside the comfortable half.
 * If the seeded catalog ever passes eight, the upgrade path is a chip list —
 * NOT a smaller font, and not a "+N more" truncation, which would hide what the
 * app decided on the user's behalf.
 */
export const SENTENCE_APP_CEILING = 8;

export function entryLabel(entry: ExcludedAppEntry): string {
  return entry.displayName.trim() || entry.bundleId.trim();
}

/**
 * "A" · "A or B" · "A, B, or C" — the separator that goes BEFORE the name at
 * `index`. Split out from the names because each name is a button, so the list
 * cannot be joined into one string. If this stops reading like English the whole
 * premise of the control is gone.
 */
export function separatorBefore(index: number, count: number): string {
  if (index === 0) return "";
  if (index === count - 1) return count === 2 ? " or " : ", or ";
  return ", ";
}

/** The words around the names: what comes before the first and after the last. */
export function excludedSentence(entries: ExcludedAppEntry[]): { lead: string; tail: string } {
  if (entries.length === 0) {
    return { lead: "Mnema records everything on your screen.", tail: "" };
  }
  if (entries.some((entry) => entry.enabled)) {
    return { lead: "Mnema never records ", tail: "." };
  }
  return {
    lead: "Mnema records everything on your screen. ",
    tail: entries.length === 1 ? " is back on." : " are back on.",
  };
}

/**
 * The one explanatory line under the sentence. `hint` is the faint half.
 *
 * The audio claim is only made when something is ACTUALLY being filtered —
 * `activeExclusionBundleIds` drops pending rules, so a set that is nothing but
 * "apps you haven't installed yet" says the rule waits instead of claiming
 * protection it does not have.
 */
export function excludedNote(entries: ExcludedAppEntry[]): { text: string; hint: string } {
  if (entries.length === 0) {
    return {
      text: "Nothing is hidden. Everything on your screen is recorded.",
      hint: "Add an app it should never see.",
    };
  }
  if (!entries.some((entry) => entry.enabled)) {
    return { text: "", hint: "Click a crossed-out name to hide it again." };
  }
  const waiting = entries.filter((entry) => entry.enabled && isPendingExclusion(entry)).length;
  const parts: string[] = [];
  if (activeExclusionBundleIds(entries).length > 0) parts.push(AUDIO_CLAIM);
  if (waiting > 0) {
    parts.push(`${waiting === 1 ? "One of them isn't" : `${waiting} of them aren't`} installed yet — the rule waits.`);
  }
  return { text: parts.join(" "), hint: "Click a name to record it after all." };
}

/** What the live region says when a name is clicked. */
export function strikeAnnouncement(entry: ExcludedAppEntry, enabled: boolean): string {
  const name = entryLabel(entry);
  return enabled ? `${name} is never recorded again.` : `${name} is recorded like everything else.`;
}

/** What the live region says after the add field commits. */
export function addAnnouncement(displayName: string, installed: boolean): string {
  return installed
    ? `${displayName} is never recorded.`
    : `${displayName} will never be recorded once you install it.`;
}

/**
 * The typed name → an installed app, exact match first, then unique-enough
 * prefix. A name that matches nothing installed is still a real rule: the caller
 * stores it with an empty bundle id and it resolves on first sighting
 * (`pendingExclusionResolutions`).
 */
export function resolveTypedApp(
  text: string,
  candidates: PrivacyAppCandidate[],
): PrivacyAppCandidate | null {
  const needle = normalizedSearchValue(text);
  if (!needle) return null;
  return (
    candidates.find((candidate) => normalizedSearchValue(candidate.displayName) === needle) ??
    candidates.find((candidate) => normalizedSearchValue(candidate.displayName).startsWith(needle)) ??
    null
  );
}
