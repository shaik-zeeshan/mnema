// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  MCP_PRESETS,
  presetDisplayLabel,
  presetToDraft,
  presetForServer,
  presetOverrides,
  buildCustomMcpDraft,
} from "./mcp-presets";

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

  it("Notion is hosted OAuth (ADR 0051) — no local npx variant, no pasted secret", () => {
    const notion = byId("notion");
    expect(notion.kind).toBe("hosted");
    expect(notion.authMode).toBe("oauth");
    expect(notion.url).toBe("https://mcp.notion.com/mcp");
    // OAuth has no pasted token and no local process.
    expect(notion.command).toBeUndefined();
    expect(notion.args).toBeUndefined();
    expect(notion.secretEnvName).toBeUndefined();
    expect(notion.secretLabel).toBeUndefined();
  });

  it("bearer presets leave authMode absent or bearer", () => {
    for (const p of MCP_PRESETS.filter((p) => p.kind === "hosted" && p.id !== "notion")) {
      expect([undefined, "bearer"]).toContain(p.authMode);
    }
  });
});

describe("presetToDraft", () => {
  it("hosted bearer → http draft with url set, authMode bearer, stdio fields cleared", () => {
    const draft = presetToDraft(byId("github"), []);
    expect(draft).toEqual({
      id: "github",
      label: "GitHub",
      enabled: true,
      transport: "http",
      authMode: "bearer",
      command: null,
      args: [],
      env: [],
      url: "https://api.githubcopilot.com/mcp/",
      secretEnvName: null,
      enabledTools: null,
    });
  });

  it("hosted OAuth (Notion) → http draft with authMode oauth, no bearer secret path", () => {
    const draft = presetToDraft(byId("notion"), []);
    expect(draft.transport).toBe("http");
    expect(draft.authMode).toBe("oauth");
    expect(draft.url).toBe("https://mcp.notion.com/mcp");
    expect(draft.command).toBeNull();
    expect(draft.secretEnvName).toBeNull();
  });

  it("local → stdio draft with command/args, url cleared", () => {
    const draft = presetToDraft(byId("filesystem"), []);
    expect(draft.transport).toBe("stdio");
    expect(draft.command).toBe("npx");
    expect(draft.args).toEqual(["-y", "@modelcontextprotocol/server-filesystem", "~/Documents"]);
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

describe("presetForServer", () => {
  const server = (over) => ({ id: "github", transport: "http", ...over });

  it("matches a preset by exact id + transport", () => {
    expect(presetForServer(server())?.id).toBe("github");
  });

  it("matches a slugger-suffixed id (github-2)", () => {
    expect(presetForServer(server({ id: "github-2" }))?.id).toBe("github");
  });

  it("requires the transport to match — a stdio connector on a hosted preset id is Custom", () => {
    // An old local-Notion connector (stdio) must NOT match the now-hosted Notion
    // preset — it falls through to Custom.
    expect(presetForServer({ id: "notion", transport: "stdio" })).toBeNull();
    expect(presetForServer(server({ transport: "stdio" }))).toBeNull();
  });

  it("no id match → null (a custom/legacy connector)", () => {
    expect(presetForServer({ id: "my-thing", transport: "http" })).toBeNull();
    // A prefix that isn't the `-<n>` slug shape does not match.
    expect(presetForServer({ id: "githubbed", transport: "http" })).toBeNull();
  });
});

describe("presetOverrides", () => {
  const adv = (over) => ({ name: "", url: "", command: "", args: "", nodeMissing: false, ...over });

  it("no edits → empty override (preset defaults win)", () => {
    expect(presetOverrides(byId("github"), adv())).toEqual({});
  });

  it("a local preset with Node missing lands disabled", () => {
    expect(presetOverrides(byId("filesystem"), adv({ nodeMissing: true })).enabled).toBe(false);
    // Node missing is irrelevant to a hosted preset.
    expect(presetOverrides(byId("github"), adv({ nodeMissing: true })).enabled).toBeUndefined();
  });

  it("only a changed name/url is carried for a hosted preset", () => {
    expect(presetOverrides(byId("github"), adv({ name: "GitHub" }))).toEqual({});
    expect(presetOverrides(byId("github"), adv({ name: "Work GH", url: "https://gh.example/mcp" }))).toEqual({
      label: "Work GH",
      url: "https://gh.example/mcp",
    });
  });

  it("a changed command/args is carried for a local preset (whitespace split)", () => {
    const o = presetOverrides(byId("filesystem"), adv({ command: "node", args: "server.js ~/Docs" }));
    expect(o.command).toBe("node");
    expect(o.args).toEqual(["server.js", "~/Docs"]);
    // Emptying args yields an explicit empty list (a real change from the default).
    expect(presetOverrides(byId("filesystem"), adv({ args: "" })).args).toEqual([]);
  });
});

describe("buildCustomMcpDraft", () => {
  const model = (over) => ({
    id: "",
    label: "My API",
    enabled: true,
    transport: "http",
    command: "",
    args: [],
    env: [],
    url: "https://api.example/mcp",
    secretEnvName: "",
    enabledTools: null,
    ...over,
  });

  it("http model → http draft, bearer default, stdio fields nulled", () => {
    const d = buildCustomMcpDraft(model(), []);
    expect(d.transport).toBe("http");
    expect(d.authMode).toBe("bearer");
    expect(d.url).toBe("https://api.example/mcp");
    expect(d.command).toBeNull();
    expect(d.args).toEqual([]);
    expect(d.secretEnvName).toBeNull();
  });

  it("http OAuth model keeps authMode oauth (else its Connect flow is unreachable)", () => {
    expect(buildCustomMcpDraft(model({ authMode: "oauth" }), []).authMode).toBe("oauth");
  });

  it("stdio model → stdio draft, authMode undefined, url nulled, blank args/env dropped", () => {
    const d = buildCustomMcpDraft(
      model({
        transport: "stdio",
        command: " my-server ",
        args: ["--flag", "  ", ""],
        env: [{ name: "TOKEN", value: "x" }, { name: "  ", value: "" }],
        url: "https://ignored",
      }),
      [],
    );
    expect(d.transport).toBe("stdio");
    expect(d.authMode).toBeUndefined();
    expect(d.command).toBe("my-server");
    expect(d.args).toEqual(["--flag"]);
    expect(d.env).toEqual([{ name: "TOKEN", value: "x" }]);
    expect(d.url).toBeNull();
  });

  it("slugs the id off the trimmed label, avoiding existing ids", () => {
    expect(buildCustomMcpDraft(model({ label: "My API" }), ["my-api"]).id).toBe("my-api-2");
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
