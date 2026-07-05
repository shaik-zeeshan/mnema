// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { newMcpServerId } from "./ai-providers";

describe("newMcpServerId", () => {
  it("slugs a label to the load-bearing [a-z0-9-] charset", () => {
    expect(newMcpServerId("GitHub", [])).toBe("github");
    expect(newMcpServerId("My Cool Server!", [])).toBe("my-cool-server");
    expect(newMcpServerId("  spaced  ", [])).toBe("spaced");
  });

  it("falls back to `connector` when the label has no usable characters", () => {
    expect(newMcpServerId("", [])).toBe("connector");
    expect(newMcpServerId("!!!", [])).toBe("connector");
  });

  it("suffixes on collision", () => {
    expect(newMcpServerId("GitHub", ["github"])).toBe("github-2");
    expect(newMcpServerId("GitHub", ["github", "github-2"])).toBe("github-3");
    expect(newMcpServerId("", ["connector"])).toBe("connector-2");
  });
});
