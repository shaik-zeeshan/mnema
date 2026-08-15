// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { aiListingFailureCopy, newMcpServerId } from "./ai-providers";

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

describe("aiListingFailureCopy", () => {
  it("turns the chatgpt reconnect code into copy a user can act on", () => {
    // `ai_runtime_list_models` reports a per-provider failure whose `reason` is
    // rendered verbatim by ModelPickerMenu ("{label} — {reason}") and by the
    // aggregated `modelsError` line in Settings and onboarding. A chatgpt
    // instance with no token set fails with the machine code
    // `needs_reconnect:<id>`, so without this translation the picker reads
    // "ChatGPT — needs_reconnect:chatgpt".
    expect(aiListingFailureCopy("needs_reconnect:chatgpt")).not.toContain("needs_reconnect");
    expect(aiListingFailureCopy("needs_reconnect:chatgpt-2")).toContain("sign in");
  });

  it("distinguishes an unreachable endpoint from a spent login", () => {
    // Two codes, two different fixes. Collapsing them is what makes a user
    // disconnect a healthy account because their network blipped.
    expect(aiListingFailureCopy("provider_unreachable:chatgpt")).toBe("unreachable");
    expect(aiListingFailureCopy("needs_reconnect:chatgpt")).toContain("sign in");
  });

  it("passes every already-human listing reason through untouched", () => {
    // The other `classify_listing_failure` outputs are already the at-a-glance
    // label; re-wording them here would drift from the backend contract.
    expect(aiListingFailureCopy("unreachable")).toBe("unreachable");
    expect(aiListingFailureCopy("missing API key")).toBe("missing API key");
    expect(aiListingFailureCopy("authentication failed")).toBe("authentication failed");
  });
});
