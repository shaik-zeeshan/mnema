// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  activeExclusionBundleIds,
  isPendingExclusion,
  pendingExclusionResolutions,
  recommendationActionFor,
} from "./app-privacy-exclusion";

function entry(overrides = {}) {
  return {
    id: "rule-1",
    enabled: true,
    bundleId: "com.example.installed",
    displayName: "Installed",
    ...overrides,
  };
}

const pendingFigma = entry({ id: "rule-pending", bundleId: "", displayName: "Figma" });

describe("isPendingExclusion", () => {
  it("is true for a name-only rule and false once a bundle id lands", () => {
    expect(isPendingExclusion(pendingFigma)).toBe(true);
    expect(isPendingExclusion({ ...pendingFigma, bundleId: "  " })).toBe(true);
    expect(isPendingExclusion({ ...pendingFigma, bundleId: "com.figma.Desktop" })).toBe(false);
    expect(isPendingExclusion(entry())).toBe(false);
  });

  it("is false for a rule with neither a bundle id nor a name — that is not a rule", () => {
    expect(isPendingExclusion({ ...pendingFigma, displayName: "  " })).toBe(false);
  });
});

describe("activeExclusionBundleIds", () => {
  // The failure mode this whole slice guards: a pending rule that looks like it
  // protects something. It must not appear in the ids handed to the screen
  // filter or the system-audio tap while it is unresolved.
  it("omits a pending rule, and keeps it omitted even when enabled", () => {
    expect(activeExclusionBundleIds([entry(), pendingFigma])).toEqual(["com.example.installed"]);
    expect(activeExclusionBundleIds([pendingFigma])).toEqual([]);
  });

  it("omits a struck rule and dedupes by canonical bundle id", () => {
    const ids = activeExclusionBundleIds([
      entry(),
      entry({ id: "rule-2", bundleId: "COM.EXAMPLE.INSTALLED" }),
      entry({ id: "rule-3", bundleId: "com.example.struck", enabled: false }),
    ]);
    expect(ids).toEqual(["com.example.installed"]);
  });
});

describe("pendingExclusionResolutions", () => {
  const figmaCandidate = { bundleId: "com.figma.Desktop", displayName: "figma" };

  it("resolves a pending rule on first sighting of a matching display name", () => {
    expect(pendingExclusionResolutions([entry(), pendingFigma], [figmaCandidate])).toEqual([
      {
        kind: "add",
        command: "add_privacy_excluded_app",
        args: { bundleId: "com.figma.Desktop", displayName: "Figma" },
      },
    ]);
  });

  it("stays quiet when nothing matches, when nothing is pending, or on a resolved rule", () => {
    expect(pendingExclusionResolutions([pendingFigma], [])).toEqual([]);
    expect(pendingExclusionResolutions([pendingFigma], [
      { bundleId: "com.apple.Safari", displayName: "Safari" },
    ])).toEqual([]);
    expect(pendingExclusionResolutions([entry()], [figmaCandidate])).toEqual([]);
  });

  it("never resolves against a candidate with no bundle id of its own", () => {
    expect(pendingExclusionResolutions([pendingFigma], [
      { bundleId: "  ", displayName: "Figma" },
    ])).toEqual([]);
  });

  it("resolves a struck pending rule too — the backend keeps it struck", () => {
    const struck = { ...pendingFigma, enabled: false };
    expect(pendingExclusionResolutions([struck], [figmaCandidate])).toHaveLength(1);
  });
});

describe("recommendationActionFor with a not-yet-installed app", () => {
  const notInstalled = { bundleId: "", displayName: "Bitwarden", exclusionState: "missing" };

  // Every blank bundle id equals every other one, so identity for a pending row
  // has to be the display name. Without that, adding a second not-yet-installed
  // app re-enabled the FIRST pending row and the second app was never stored.
  it("adds a second pending rule instead of re-enabling the first", () => {
    expect(recommendationActionFor(notInstalled, [pendingFigma])).toEqual({
      kind: "add",
      command: "add_privacy_excluded_app",
      args: { bundleId: "", displayName: "Bitwarden" },
    });
  });

  it("re-enables the pending rule with the same name rather than duplicating it", () => {
    const struckPending = { ...pendingFigma, enabled: false };
    expect(recommendationActionFor({ ...notInstalled, displayName: "figma" }, [struckPending])).toEqual({
      kind: "reenable",
      command: "set_privacy_excluded_app_enabled",
      args: { sourceId: "rule-pending", enabled: true },
    });
  });

  it("does nothing when that pending rule is already live", () => {
    expect(recommendationActionFor({ ...notInstalled, displayName: "Figma" }, [pendingFigma]).kind)
      .toBe("none");
  });
});
