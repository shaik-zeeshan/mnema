import { describe, expect, test } from "bun:test";
import {
  canonicalBundleIdForComparison,
  filteredPrivacyAppCandidates,
  normalizedSearchValue,
  recommendationActionFor,
  unresolvedIconBundleIds,
  visibleBrowserDisclosureApps,
  type BrowserDisclosureApp,
  type PrivacyAppCandidate,
  type RecommendedAppExclusion,
  type SensitiveCaptureRecommendations,
} from "../src/lib/app-privacy-exclusion";
import type { ExcludedAppEntry } from "../src/lib/types";

function excluded(bundleId: string, enabled = true): ExcludedAppEntry {
  return {
    id: `excluded-${bundleId}`,
    enabled,
    bundleId,
    displayName: bundleId,
  };
}

function candidate(bundleId: string, displayName: string): PrivacyAppCandidate {
  return {
    id: `candidate-${bundleId}`,
    enabled: true,
    bundleId,
    displayName,
    running: false,
    iconPath: null,
  };
}

function recommendation(
  bundleId: string,
  exclusionState: RecommendedAppExclusion["exclusionState"],
): RecommendedAppExclusion {
  return {
    bundleId,
    displayName: "Apple Passwords",
    category: "apple_passwords",
    categoryLabel: "Apple passwords",
    running: false,
    iconPath: null,
    exclusionState,
  };
}

describe("App Privacy Exclusion helpers", () => {
  test("canonical bundle IDs use locale-invariant casing", () => {
    expect(canonicalBundleIdForComparison(" COM.EXAMPLE.ID ")).toBe("com.example.id");
  });

  test("search values use locale-invariant casing", () => {
    expect(normalizedSearchValue(" COM.EXAMPLE.ID ")).toBe("com.example.id");
  });

  test("filters installed app candidates using normalized bundle IDs and search text", () => {
    const candidates = [
      candidate("com.apple.Passwords", "Apple Passwords"),
      candidate("com.bitwarden.desktop", "Bitwarden"),
      candidate("com.agilebits.onepassword7", "1Password 7"),
    ];
    const excludedApps = [excluded("COM.APPLE.PASSWORDS")];

    expect(filteredPrivacyAppCandidates(candidates, excludedApps, "")).toEqual([
      candidates[1],
      candidates[2],
    ]);
    expect(filteredPrivacyAppCandidates(candidates, excludedApps, "warden")).toEqual([
      candidates[1],
    ]);
    expect(filteredPrivacyAppCandidates(candidates, excludedApps, "AGILEBITS")).toEqual([
      candidates[2],
    ]);
  });

  test("deduplicates icon requests by normalized bundle ID and skips cached or in-flight IDs", () => {
    const requested = new Set(["com.bitwarden.desktop"]);

    expect(unresolvedIconBundleIds(
      [
        " com.apple.Passwords ",
        "COM.APPLE.PASSWORDS",
        "com.bitwarden.desktop",
        "com.1password.1password",
      ],
      { "com.apple.passwords": "/tmp/passwords.icns" },
      requested,
    )).toEqual(["com.1password.1password"]);
  });

  test("selects add, re-enable, or no-op recommendation actions from current exclusions", () => {
    expect(recommendationActionFor(recommendation("com.apple.Passwords", "missing"), [])).toEqual({
      kind: "add",
      command: "add_privacy_excluded_app",
      args: {
        bundleId: "com.apple.Passwords",
        displayName: "Apple Passwords",
      },
    });

    expect(recommendationActionFor(
      recommendation("com.apple.Passwords", "disabled"),
      [excluded("COM.APPLE.PASSWORDS", false)],
    )).toEqual({
      kind: "reenable",
      command: "set_privacy_excluded_app_enabled",
      args: {
        sourceId: "excluded-COM.APPLE.PASSWORDS",
        enabled: true,
      },
    });

    expect(recommendationActionFor(
      recommendation("com.apple.Passwords", "missing"),
      [excluded("COM.APPLE.PASSWORDS", true)],
    )).toEqual({
      kind: "none",
      command: null,
      args: null,
    });
  });

  test("shows browser disclosure rows only when running or already configured", () => {
    const browser = (
      bundleId: string,
      running: boolean,
      exclusionState: BrowserDisclosureApp["exclusionState"],
    ): BrowserDisclosureApp => ({
      bundleId,
      displayName: bundleId,
      running,
      iconPath: null,
      exclusionState,
    });
    const recommendations: SensitiveCaptureRecommendations = {
      promptId: "prompt",
      recommendedApps: [],
      actionableRecommendationCount: 0,
      shouldShowExistingUserPrompt: false,
      browserDisclosures: [
        browser("com.apple.Safari", false, "missing"),
        browser("com.google.Chrome", true, "missing"),
        browser("org.mozilla.firefox", false, "disabled"),
      ],
    };

    expect(visibleBrowserDisclosureApps(recommendations).map((app) => app.bundleId)).toEqual([
      "com.google.Chrome",
      "org.mozilla.firefox",
    ]);
  });
});
