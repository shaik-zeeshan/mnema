import type { ExcludedAppEntry } from "./types";

export type RecommendedExclusionState = "missing" | "disabled" | "enabled";

export type PrivacyAppCandidateDto = {
  bundleId: string;
  displayName: string;
  running: boolean;
  iconPath: string | null;
};

export type PrivacyAppCandidate = ExcludedAppEntry & {
  running: boolean;
  iconPath: string | null;
};

export type AppIconResolution = {
  bundleId: string;
  iconPath: string | null;
};

export type RecommendedAppExclusion = {
  bundleId: string;
  displayName: string;
  category: string;
  categoryLabel: string;
  running: boolean;
  iconPath: string | null;
  exclusionState: RecommendedExclusionState;
};

export type BrowserDisclosureApp = {
  bundleId: string;
  displayName: string;
  running: boolean;
  iconPath: string | null;
  exclusionState: RecommendedExclusionState;
};

export type AppPrivacyRecommendation = RecommendedAppExclusion | BrowserDisclosureApp;

export type SensitiveCaptureRecommendations = {
  promptId: string;
  recommendedApps: RecommendedAppExclusion[];
  actionableRecommendationCount: number;
  shouldShowExistingUserPrompt: boolean;
  browserDisclosures: BrowserDisclosureApp[];
};

export type PrivacyRecommendationCommand =
  | {
      kind: "add";
      command: "add_privacy_excluded_app";
      args: {
        bundleId: string;
        displayName: string;
      };
    }
  | {
      kind: "reenable";
      command: "set_privacy_excluded_app_enabled";
      args: {
        sourceId: string;
        enabled: true;
      };
    }
  | {
      kind: "none";
      command: null;
      args: null;
    };

export function canonicalBundleIdForComparison(value: string): string {
  return value.trim().toLowerCase();
}

export function sameBundleId(left: string, right: string): boolean {
  return canonicalBundleIdForComparison(left) === canonicalBundleIdForComparison(right);
}

export function uniqueBundleIds(bundleIds: Array<string | null | undefined>): string[] {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const rawBundleId of bundleIds) {
    const bundleId = rawBundleId?.trim() ?? "";
    if (!bundleId) continue;
    const canonical = canonicalBundleIdForComparison(bundleId);
    if (seen.has(canonical)) continue;
    seen.add(canonical);
    unique.push(bundleId);
  }
  return unique;
}

export function normalizedSearchValue(value: string): string {
  return value.trim().toLowerCase();
}

export function sameDisplayName(left: string, right: string): boolean {
  return normalizedSearchValue(left) === normalizedSearchValue(right);
}

/**
 * A rule the user typed before the app was installed: a display name and no
 * bundle id yet. It is stored, but it excludes nothing until it resolves —
 * `evaluate_privacy` skips an empty bundle id, so a pending rule reaches
 * neither the screen content filter nor the system-audio tap's exclude list
 * (both derive from `decision.excluded_bundle_ids`, `native_capture/privacy.rs`).
 * Render it as pending, never as protecting.
 */
export function isPendingExclusion(entry: ExcludedAppEntry): boolean {
  return !entry.bundleId.trim() && Boolean(entry.displayName.trim());
}

/**
 * The bundle ids a rule set actually excludes — the frontend mirror of
 * `evaluate_privacy`: enabled, resolved, deduped.
 */
export function activeExclusionBundleIds(excludedApps: ExcludedAppEntry[]): string[] {
  return uniqueBundleIds(excludedApps.filter((app) => app.enabled).map((app) => app.bundleId));
}

/**
 * First sighting of an installed app whose name matches a pending rule. Each
 * result is the existing `add_privacy_excluded_app` command; the backend fills
 * the bundle id into the pending row in place, keeping its id and enabled state.
 */
export function pendingExclusionResolutions(
  excludedApps: ExcludedAppEntry[],
  candidates: Array<{ bundleId: string; displayName: string }>,
): Array<Extract<PrivacyRecommendationCommand, { kind: "add" }>> {
  return excludedApps.filter(isPendingExclusion).flatMap((entry) => {
    const sighting = candidates.find((candidate) => (
      Boolean(candidate.bundleId.trim()) && sameDisplayName(candidate.displayName, entry.displayName)
    ));
    if (!sighting) return [];
    return [{
      kind: "add" as const,
      command: "add_privacy_excluded_app" as const,
      args: { bundleId: sighting.bundleId, displayName: entry.displayName },
    }];
  });
}

export function privacyAppCandidateSearchText(candidate: PrivacyAppCandidate): string {
  return normalizedSearchValue(`${candidate.displayName} ${candidate.bundleId}`);
}

export function makePrivacyAppCandidate(
  candidate: PrivacyAppCandidateDto,
  id: string,
): PrivacyAppCandidate {
  return {
    id,
    enabled: true,
    bundleId: candidate.bundleId,
    displayName: candidate.displayName,
    running: candidate.running,
    iconPath: candidate.iconPath,
  };
}

export function availablePrivacyAppCandidates(
  candidates: PrivacyAppCandidate[],
  excludedApps: ExcludedAppEntry[],
): PrivacyAppCandidate[] {
  return candidates.filter((candidate) => (
    !excludedApps.some((item) => sameBundleId(item.bundleId, candidate.bundleId))
  ));
}

export function filteredPrivacyAppCandidates(
  candidates: PrivacyAppCandidate[],
  excludedApps: ExcludedAppEntry[],
  query: string,
  limit = 12,
): PrivacyAppCandidate[] {
  const available = availablePrivacyAppCandidates(candidates, excludedApps);
  const normalizedQuery = normalizedSearchValue(query);
  if (!normalizedQuery) return available.slice(0, limit);
  return available
    .filter((candidate) => privacyAppCandidateSearchText(candidate).includes(normalizedQuery))
    .slice(0, limit);
}

export function pendingRecommendedApps(
  recommendations: SensitiveCaptureRecommendations | null,
): RecommendedAppExclusion[] {
  return (recommendations?.recommendedApps ?? []).filter((app) => app.exclusionState !== "enabled");
}

export function visibleBrowserDisclosureApps(
  recommendations: SensitiveCaptureRecommendations | null,
): BrowserDisclosureApp[] {
  return (recommendations?.browserDisclosures ?? []).filter((app) => (
    app.running || app.exclusionState !== "missing"
  ));
}

export function shouldShowSensitiveRecommendationPrompt(
  recommendations: SensitiveCaptureRecommendations | null,
): boolean {
  return Boolean(
    recommendations?.shouldShowExistingUserPrompt &&
    pendingRecommendedApps(recommendations).length > 0,
  );
}

export function recommendationBundleIds(
  recommendations: SensitiveCaptureRecommendations | null,
): string[] {
  return [
    ...(recommendations?.recommendedApps ?? []).map((app) => app.bundleId),
    ...(recommendations?.browserDisclosures ?? []).map((app) => app.bundleId),
  ];
}

export function appIconFallback(
  displayName: string | null | undefined,
  bundleId: string | null | undefined,
): string {
  return ((displayName ?? "").trim() || (bundleId ?? "").trim() || "?").slice(0, 1).toUpperCase();
}

export function iconPathForBundleId(
  bundleId: string,
  iconPathsByBundleId: Record<string, string>,
): string | null {
  const exact = iconPathsByBundleId[bundleId];
  if (exact) return exact;
  const canonical = canonicalBundleIdForComparison(bundleId);
  const matchingKey = Object.keys(iconPathsByBundleId).find((key) => (
    canonicalBundleIdForComparison(key) === canonical
  ));
  return matchingKey ? iconPathsByBundleId[matchingKey] ?? null : null;
}

export function unresolvedIconBundleIds(
  bundleIds: Array<string | null | undefined>,
  iconPathsByBundleId: Record<string, string>,
  requestedCanonicalBundleIds: ReadonlySet<string>,
): string[] {
  return uniqueBundleIds(bundleIds).filter((bundleId) => {
    const canonical = canonicalBundleIdForComparison(bundleId);
    return !iconPathForBundleId(bundleId, iconPathsByBundleId) &&
      !requestedCanonicalBundleIds.has(canonical);
  });
}

export function mergeIconResolutions(
  currentIconPaths: Record<string, string>,
  resolutions: AppIconResolution[],
): { iconPathsByBundleId: Record<string, string>; changed: boolean } {
  const nextIconPaths = { ...currentIconPaths };
  let changed = false;
  for (const icon of resolutions) {
    if (!icon.iconPath || iconPathForBundleId(icon.bundleId, nextIconPaths) === icon.iconPath) continue;
    nextIconPaths[icon.bundleId] = icon.iconPath;
    changed = true;
  }
  return { iconPathsByBundleId: nextIconPaths, changed };
}

export function recommendationActionLabel(state: RecommendedExclusionState): string {
  return state === "disabled" ? "Re-enable" : "Exclude";
}

export function recommendationActionFor(
  app: AppPrivacyRecommendation,
  excludedApps: ExcludedAppEntry[],
): PrivacyRecommendationCommand {
  // A rule for an app that is not installed carries no bundle id, and every
  // blank bundle id equals every other one — so identity for those rows is the
  // display name. Without this, adding a second not-yet-installed app re-enables
  // the FIRST pending row and the second app is silently never stored.
  const existing = excludedApps.find((entry) => (
    app.bundleId.trim()
      ? sameBundleId(entry.bundleId, app.bundleId)
      : isPendingExclusion(entry) && sameDisplayName(entry.displayName, app.displayName)
  ));
  if (app.exclusionState === "enabled" || existing?.enabled) {
    return { kind: "none", command: null, args: null };
  }
  if (existing) {
    return {
      kind: "reenable",
      command: "set_privacy_excluded_app_enabled",
      args: {
        sourceId: existing.id,
        enabled: true,
      },
    };
  }
  return {
    kind: "add",
    command: "add_privacy_excluded_app",
    args: {
      bundleId: app.bundleId,
      displayName: app.displayName,
    },
  };
}
