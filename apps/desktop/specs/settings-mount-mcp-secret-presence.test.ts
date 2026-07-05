import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Regression: the MCP connectors card renders a "secret in keychain" badge, a
// context-dependent password placeholder, and a Clear button that are ALL gated
// on `mcpSecretSavedById[server.id]` (populated by refreshMcpServerSecretPresence).
// The provider-key equivalent is chained off loadRecordingSettings in the mount
// untrack block so its badges reflect saved keys on first Settings open. The MCP
// refresh must be chained the same way — it reads the draft MCP server list,
// which loadRecordingSettings only populates after its async fetch resolves.
// Without it, a returning user with a saved MCP secret opens Settings and sees no
// badge, the wrong placeholder, and a permanently-disabled Clear button until an
// unrelated ai_runtime autosave happens to fire.
//
// A full component test needs a Svelte runtime harness not wired into this
// bun:test setup, so we assert the STRUCTURAL guarantee against the source, the
// same faithful check settings-mount-untrack.test.ts uses.

const pagePath = fileURLToPath(
  new URL("../src/routes/settings/+page.svelte", import.meta.url),
);
const source = readFileSync(pagePath, "utf8");

function mountUntrackBlock(src: string): string {
  let searchFrom = 0;
  for (;;) {
    const start = src.indexOf("untrack(() => {", searchFrom);
    expect(start).toBeGreaterThan(-1);
    const open = src.indexOf("{", start);
    let depth = 0;
    let body = "";
    for (let i = open; i < src.length; i++) {
      if (src[i] === "{") depth++;
      else if (src[i] === "}") {
        depth--;
        if (depth === 0) {
          body = src.slice(open + 1, i);
          break;
        }
      }
    }
    if (body.includes("loadCaptureSupport(")) return body;
    searchFrom = start + "untrack(() => {".length;
  }
}

describe("settings mount: MCP secret presence refreshed on load", () => {
  const block = mountUntrackBlock(source);

  test("refreshMcpServerSecretPresence is invoked in the mount untrack block", () => {
    expect(block).toContain("refreshMcpServerSecretPresence(");
  });

  test("it is chained AFTER loadRecordingSettings (reads the draft MCP list)", () => {
    const loadIdx = block.indexOf("rec.loadRecordingSettings()");
    const refreshIdx = block.indexOf("refreshMcpServerSecretPresence(");
    expect(loadIdx).toBeGreaterThan(-1);
    expect(refreshIdx).toBeGreaterThan(loadIdx);
  });
});
