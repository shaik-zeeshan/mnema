// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { MCP_PRESETS, presetDisplayLabel, presetToDraft } from "./mcp-presets";

const byId = (id) => MCP_PRESETS.find((p) => p.id === id);

describe("catalog invariants", () => {
  it("every preset has non-empty label/tagline/lede/brandSvg and a valid kind", () => {
    for (const p of MCP_PRESETS) {
      expect(p.id.length).toBeGreaterThan(0);
      expect(p.label.length).toBeGreaterThan(0);
      expect(p.tagline.length).toBeGreaterThan(0);
      expect(p.lede.length).toBeGreaterThan(0);
      expect(p.brandSvg).toContain("<svg");
      expect(["hosted", "local"]).toContain(p.kind);
    }
  });

  it("every preset with a secret has non-empty secretLabel + helpUrl", () => {
    for (const p of MCP_PRESETS) {
      if (p.secretLabel === undefined && p.secretEnvName === undefined) continue;
      expect(p.secretLabel.length).toBeGreaterThan(0);
      expect(p.helpUrl.startsWith("https://")).toBe(true);
    }
  });

  it("hosted presets carry an https url and no command/args/secretEnvName", () => {
    for (const p of MCP_PRESETS.filter((p) => p.kind === "hosted")) {
      expect(p.url.startsWith("https://")).toBe(true);
      expect(p.command).toBeUndefined();
      expect(p.args).toBeUndefined();
      expect(p.secretEnvName).toBeUndefined();
    }
  });

  it("local presets carry command+args and no url", () => {
    for (const p of MCP_PRESETS.filter((p) => p.kind === "local")) {
      expect(p.command.length).toBeGreaterThan(0);
      expect(Array.isArray(p.args)).toBe(true);
      expect(p.args.length).toBeGreaterThan(0);
      expect(p.url).toBeUndefined();
    }
  });
});

describe("presetToDraft", () => {
  it("hosted → http draft with url set, stdio fields cleared", () => {
    const draft = presetToDraft(byId("github"), []);
    expect(draft).toEqual({
      id: "github",
      label: "GitHub",
      enabled: true,
      transport: "http",
      command: null,
      args: [],
      env: [],
      url: "https://api.githubcopilot.com/mcp/",
      secretEnvName: null,
      enabledTools: null,
    });
  });

  it("local → stdio draft with command/args/secretEnvName, url cleared", () => {
    const draft = presetToDraft(byId("notion"), []);
    expect(draft.transport).toBe("stdio");
    expect(draft.command).toBe("npx");
    expect(draft.args).toEqual(["-y", "@notionhq/notion-mcp-server"]);
    expect(draft.secretEnvName).toBe("NOTION_TOKEN");
    expect(draft.url).toBeNull();
  });

  it("local without a secret → secretEnvName null", () => {
    expect(presetToDraft(byId("filesystem"), []).secretEnvName).toBeNull();
  });

  it("args are copied, not shared with the catalog", () => {
    const draft = presetToDraft(byId("filesystem"), []);
    draft.args.push("mutated");
    expect(byId("filesystem").args).not.toContain("mutated");
  });

  it("duplicate add suffixes both id (slugger) and label", () => {
    const first = presetToDraft(byId("github"), []);
    const second = presetToDraft(byId("github"), [first]);
    expect(second.id).toBe("github-2");
    expect(second.label).toBe("GitHub (2)");
    const third = presetToDraft(byId("github"), [first, second]);
    expect(third.id).toBe("github-3");
    expect(third.label).toBe("GitHub (3)");
  });

  it("label suffix keys off labels, id off ids — independently", () => {
    // A hand-made connector already took the "github" id but not the label.
    const existing = [{ id: "github", label: "My server" }];
    const draft = presetToDraft(byId("github"), existing);
    expect(draft.id).toBe("github-2");
    expect(draft.label).toBe("GitHub");
  });
});

describe("presetDisplayLabel", () => {
  it("returns the plain label when free, first free suffix otherwise", () => {
    expect(presetDisplayLabel("Linear", [])).toBe("Linear");
    expect(presetDisplayLabel("Linear", ["Linear"])).toBe("Linear (2)");
    expect(presetDisplayLabel("Linear", ["Linear", "Linear (2)"])).toBe("Linear (3)");
    // A gap is reused.
    expect(presetDisplayLabel("Linear", ["Linear", "Linear (3)"])).toBe("Linear (2)");
  });
});
