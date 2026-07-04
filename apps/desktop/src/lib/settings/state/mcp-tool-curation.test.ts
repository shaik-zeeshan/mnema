// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  MCP_DEFAULT_TOOL_CAP,
  activeToolNames,
  activeToolCount,
  toggleTool,
} from "./mcp-tool-curation";

const many = (n: number) => Array.from({ length: n }, (_, i) => `t${i}`);

describe("activeToolNames", () => {
  it("uncurated (null) → the first cap tools in server order", () => {
    const names = activeToolNames(many(40), null);
    expect(names.length).toBe(MCP_DEFAULT_TOOL_CAP);
    expect(names[0]).toBe("t0");
    expect(names[MCP_DEFAULT_TOOL_CAP - 1]).toBe(`t${MCP_DEFAULT_TOOL_CAP - 1}`);
  });

  it("under the cap → all tools", () => {
    expect(activeToolNames(["a", "b"], null)).toEqual(["a", "b"]);
  });

  it("curated → only selected names that still exist, in server order", () => {
    // 'gone' drifted off the server; 'a' precedes 'c' in server order.
    expect(activeToolNames(["a", "b", "c"], ["c", "a", "gone"])).toEqual(["a", "c"]);
  });

  it("curated empty → nothing active", () => {
    expect(activeToolNames(["a", "b"], [])).toEqual([]);
  });
});

describe("toggleTool", () => {
  it("materializes null into an explicit list on first toggle (default→curated)", () => {
    // Unchecking one of the first-32 defaults turns the server curated.
    const next = toggleTool(many(40), null, "t5", false);
    expect(next).not.toContain("t5");
    // The other 31 defaults survive; nothing beyond the cap is added.
    expect(next.length).toBe(MCP_DEFAULT_TOOL_CAP - 1);
    expect(next).toContain("t0");
    expect(next).not.toContain("t32");
  });

  it("checking a tool beyond the default cap adds it (curated has no cap)", () => {
    const next = toggleTool(many(40), null, "t35", true);
    expect(next).toContain("t35");
    expect(next.length).toBe(MCP_DEFAULT_TOOL_CAP + 1);
  });

  it("adds/removes from a curated list, keeping server order + dropping drift", () => {
    expect(toggleTool(["a", "b", "c"], ["a"], "c", true)).toEqual(["a", "c"]);
    expect(toggleTool(["a", "b", "c"], ["a", "c"], "a", false)).toEqual(["c"]);
    // Toggling down to empty is allowed (offer nothing).
    expect(toggleTool(["a"], ["a"], "a", false)).toEqual([]);
    // A drifted selected name never reappears in the result.
    expect(toggleTool(["a", "b"], ["a", "gone"], "b", true)).toEqual(["a", "b"]);
  });
});

describe("activeToolCount", () => {
  it("uncurated → min(cap, N)", () => {
    expect(activeToolCount(40, null)).toBe(MCP_DEFAULT_TOOL_CAP);
    expect(activeToolCount(5, null)).toBe(5);
  });

  it("curated → the list length (even an empty offer-nothing state)", () => {
    expect(activeToolCount(40, ["a", "b"])).toBe(2);
    expect(activeToolCount(40, [])).toBe(0);
  });
});
