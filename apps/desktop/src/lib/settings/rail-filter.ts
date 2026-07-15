// Pure Settings rail search/filter — framework-free so it is unit-testable and
// reusable. The rail (`SettingsRail.svelte`) narrows its visible nav as the user
// types in the search field; this module is the single source of truth for both
//   • which (group, section) pairs survive a query, and
//   • the flattened sub-section order used by the keyboard roving-tabindex model.
//
// Matching is a case-insensitive substring against each section's label and its
// optional `keywords` (search terms for settings that live inside the section
// but aren't in its label — e.g. "retention" → Storage, "bitrate" → Video), and
// (as an affordance) the owning group's label — so typing a category name like
// "intel" keeps that whole group. A group is kept only if ≥1 of its sections
// match (group-label matches keep ALL of that group's sections). An empty or
// whitespace-only query is a pass-through (returns the input groups unchanged).

import type { KeyboardPlatform } from "$lib/keyboard";
import type { SettingsGroup, SettingsSection } from "./groups";

// Normalize a query the same way for both the haystack and needle: trim + lower.
function norm(value: string): string {
  return value.trim().toLowerCase();
}

function sectionMatches(section: SettingsSection, needle: string): boolean {
  if (section.label.toLowerCase().includes(needle)) return true;
  return (section.keywords ?? []).some((keyword) =>
    keyword.toLowerCase().includes(needle),
  );
}

function groupLabelMatches(group: SettingsGroup, needle: string): boolean {
  return group.label.toLowerCase().includes(needle);
}

/**
 * Filter the rail groups by a search query.
 *
 * - Empty / whitespace-only query → returns `groups` unchanged (same reference
 *   contents; a pass-through so callers can render the full rail).
 * - Otherwise returns a new array of groups, each carrying only the sections
 *   that match (case-insensitive substring on the section label or its
 *   `keywords`). A group whose own label matches keeps ALL its sections. Groups
 *   that end up with zero sections are dropped, so a no-match query returns `[]`.
 *
 * Group order and within-group section order are preserved.
 */
export function filterGroups(
  groups: readonly SettingsGroup[],
  query: string,
): SettingsGroup[] {
  const needle = norm(query);
  if (needle === "") return [...groups];

  const result: SettingsGroup[] = [];
  for (const group of groups) {
    // A group-label hit surfaces the whole group; otherwise keep only the
    // sub-sections whose own label matches.
    const sections = groupLabelMatches(group, needle)
      ? [...group.sections]
      : group.sections.filter((s) => sectionMatches(s, needle));
    if (sections.length > 0) {
      result.push({ ...group, sections });
    }
  }
  return result;
}

/**
 * Drop platform-gated sections that don't apply to the current platform, so the
 * rail (and its search) never surface them.
 *
 * Today only `windowsOnly` sections exist — the Windows-only GPU Acceleration
 * (NVIDIA CUDA backend) panel. On any non-Windows platform they are removed, and a
 * group left with zero sections is dropped; Windows keeps everything. Pure and
 * order-preserving (mirrors `filterGroups`), and applied BEFORE the search filter
 * so a macOS user can never reach a Windows-only section by typing its name. This is
 * the rail-side half of the gate; the panel mirrors the same `detectKeyboardPlatform()`
 * check and renders nothing off Windows.
 */
export function filterPlatform(
  groups: readonly SettingsGroup[],
  platform: KeyboardPlatform,
): SettingsGroup[] {
  if (platform === "windows") return [...groups];
  const result: SettingsGroup[] = [];
  for (const group of groups) {
    const sections = group.sections.filter((s) => !s.windowsOnly);
    if (sections.length > 0) result.push({ ...group, sections });
  }
  return result;
}

/**
 * Flatten groups into their sections in rail order (group order, then
 * within-group order). This is the keyboard roving-tabindex model — when the
 * rail is filtered, callers flatten the *filtered* groups so Arrow/Home/End only
 * traverse the currently-visible items.
 */
export function flattenSections(
  groups: readonly SettingsGroup[],
): SettingsSection[] {
  return groups.flatMap((g) => g.sections);
}
