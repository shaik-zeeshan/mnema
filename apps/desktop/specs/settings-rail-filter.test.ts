import { describe, expect, test } from "bun:test";
import { SETTINGS_GROUPS } from "../src/lib/settings/groups";
import { filterGroups, flattenSections } from "../src/lib/settings/rail-filter";

// Section ids in full rail order — the canonical sequence the keyboard model
// must preserve.
const ALL_SECTION_IDS = SETTINGS_GROUPS.flatMap((g) => g.sections.map((s) => s.id));

describe("rail-filter: filterGroups", () => {
  test("matches a section by label, case-insensitive substring", () => {
    const groups = filterGroups(SETTINGS_GROUPS, "speak");
    const ids = flattenSections(groups).map((s) => s.id);
    expect(ids).toEqual(["speakers"]);
    // The surviving group is Intelligence (Speakers' owner), not Capture etc.
    expect(groups.map((g) => g.id)).toEqual(["intelligence"]);
  });

  test("matches case-insensitively regardless of query casing (PRIV → Privacy)", () => {
    const groups = filterGroups(SETTINGS_GROUPS, "PRIV");
    const ids = flattenSections(groups).map((s) => s.id);
    expect(ids).toEqual(["privacy"]);
    expect(groups.map((g) => g.id)).toEqual(["capture"]);
  });

  test("empty query returns all five groups with all sections", () => {
    const groups = filterGroups(SETTINGS_GROUPS, "");
    expect(groups.map((g) => g.id)).toEqual([
      "general",
      "capture",
      "intelligence",
      "data",
      "about",
    ]);
    expect(flattenSections(groups).map((s) => s.id)).toEqual(ALL_SECTION_IDS);
  });

  test("whitespace-only query is a pass-through (all groups, all sections)", () => {
    const groups = filterGroups(SETTINGS_GROUPS, "   \t  ");
    expect(groups.map((g) => g.id)).toEqual([
      "general",
      "capture",
      "intelligence",
      "data",
      "about",
    ]);
    expect(flattenSections(groups).map((s) => s.id)).toEqual(ALL_SECTION_IDS);
  });

  test("a partial match keeps only matching sections and drops empty groups", () => {
    // "a" matches several sections across multiple groups; assert the surviving
    // groups carry ONLY their matching sections, and no all-miss group survives.
    const groups = filterGroups(SETTINGS_GROUPS, "video");
    expect(groups.map((g) => g.id)).toEqual(["capture"]);
    // Within Capture, only "video" matches — the siblings are dropped.
    expect(groups[0].sections.map((s) => s.id)).toEqual(["video"]);
  });

  test("a query that hits sections in several groups keeps each, narrowed", () => {
    // "st" → "startup" (general) + "storage" (data) by label, and "capture"
    // (capture) via its "system audio" keyword — keyword matches surface a
    // section just like label matches do.
    const groups = filterGroups(SETTINGS_GROUPS, "st");
    const byGroup = Object.fromEntries(
      groups.map((g) => [g.id, g.sections.map((s) => s.id)]),
    );
    expect(byGroup.general).toEqual(["startup"]);
    expect(byGroup.capture).toEqual(["capture"]);
    expect(byGroup.data).toEqual(["storage"]);
    // Intelligence/About have no "st" label or keyword match → dropped.
    expect(groups.map((g) => g.id)).toEqual(["general", "capture", "data"]);
  });

  test("a keyword match surfaces a section whose label does not match", () => {
    // "retention" lives only in Storage's keywords, not its label.
    const groups = filterGroups(SETTINGS_GROUPS, "retention");
    expect(groups.map((g) => g.id)).toEqual(["data"]);
    expect(groups[0].sections.map((s) => s.id)).toEqual(["storage"]);
  });

  test("a group-label match surfaces the whole group", () => {
    // "data" matches the Data group label → keep all its sections even though
    // neither "Storage" nor "Access" contains "data".
    const groups = filterGroups(SETTINGS_GROUPS, "data");
    const data = groups.find((g) => g.id === "data");
    expect(data?.sections.map((s) => s.id)).toEqual(["storage", "access"]);
  });

  test("no-match query returns an empty array", () => {
    const groups = filterGroups(SETTINGS_GROUPS, "zzz-nothing-here");
    expect(groups).toEqual([]);
    expect(flattenSections(groups)).toEqual([]);
  });

  test("does not mutate the input groups", () => {
    const before = SETTINGS_GROUPS.map((g) => g.sections.length);
    filterGroups(SETTINGS_GROUPS, "video");
    const after = SETTINGS_GROUPS.map((g) => g.sections.length);
    expect(after).toEqual(before);
  });
});

describe("rail-filter: flattenSections", () => {
  test("preserves rail order across all groups", () => {
    expect(flattenSections(SETTINGS_GROUPS).map((s) => s.id)).toEqual(ALL_SECTION_IDS);
  });

  test("of a filtered set preserves the relative rail order of survivors", () => {
    const groups = filterGroups(SETTINGS_GROUPS, "a"); // broad match across groups
    const ids = flattenSections(groups).map((s) => s.id);
    // The flattened ids must be a subsequence of the full rail order.
    let cursor = 0;
    for (const id of ids) {
      const found = ALL_SECTION_IDS.indexOf(id, cursor);
      expect(found).toBeGreaterThanOrEqual(cursor);
      cursor = found + 1;
    }
    expect(ids.length).toBeGreaterThan(0);
  });

  test("of an empty group list is empty", () => {
    expect(flattenSections([])).toEqual([]);
  });
});
