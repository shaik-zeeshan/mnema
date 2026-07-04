// Pure curation helpers for the MCP tool-list modal (Workstream C, C3).
//
// `enabledTools` semantics MIRROR the Rust `offered_tools` (ADR 0048):
//   • null/undefined → default-offer: the FIRST MCP_DEFAULT_TOOL_CAP tools (in
//     server order) are active.
//   • an array       → EXACTLY those names that still exist are active; anything
//     else (including tools that newly appeared on the server) stays inactive —
//     the "new tools stay unchecked" drift rule. An empty array is a valid
//     curated "offer nothing" state, distinct from null.

// Keep in lockstep with the Rust `MCP_DEFAULT_TOOL_CAP` in `ask_ai/mcp/mod.rs`.
export const MCP_DEFAULT_TOOL_CAP = 32;

/** The tool names shown CHECKED for a given curation state. */
export function activeToolNames(
  allNames: readonly string[],
  enabledTools: readonly string[] | null | undefined,
): string[] {
  if (enabledTools == null) {
    return allNames.slice(0, MCP_DEFAULT_TOOL_CAP);
  }
  // Server order (mirrors the Rust `offered_tools`), dropping drifted names.
  const wanted = new Set(enabledTools);
  return allNames.filter((name) => wanted.has(name));
}

/**
 * Toggle one tool's active state, MATERIALIZING an uncurated (null) server into
 * an explicit list first — so any toggle turns it curated `Some(list)`, matching
 * the Rust default→curated boundary. Returns the new `enabledTools` in SERVER
 * order (drifted names dropped); may be empty (the valid "offer nothing" state).
 */
export function toggleTool(
  allNames: readonly string[],
  enabledTools: readonly string[] | null | undefined,
  toolName: string,
  active: boolean,
): string[] {
  const chosen = new Set(activeToolNames(allNames, enabledTools));
  if (active) chosen.add(toolName);
  else chosen.delete(toolName);
  // Preserve server order and drop anything not on the server (drift).
  return allNames.filter((name) => chosen.has(name));
}

/**
 * The active count for the "N tools · M active" caption. `enabledTools?.length`
 * for a curated server, else `min(cap, N)` for the default-offer — matching the
 * task's explicit formula. `toolCount` is the discovered N.
 */
export function activeToolCount(
  toolCount: number,
  enabledTools: readonly string[] | null | undefined,
): number {
  return enabledTools?.length ?? Math.min(MCP_DEFAULT_TOOL_CAP, toolCount);
}
