// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { deriveMcpConnectorRow } from "./mcp-connector-row";

const row = (over) =>
  deriveMcpConnectorRow({
    authMode: undefined,
    enabled: true,
    hasSecret: false,
    oauthState: undefined,
    ...over,
  });

describe("bearer rows", () => {
  it("with a saved secret → secret badge, Configure only, not dimmed", () => {
    const v = row({ authMode: "bearer", hasSecret: true });
    expect(v.badge).toBe("secret");
    expect(v.actions).toEqual({ connect: false, reconnect: false, disconnect: false, configure: true });
    expect(v.dimmed).toBe(false);
  });

  it("without a secret → no badge (none), still Configure only", () => {
    const v = row({ authMode: "bearer", hasSecret: false });
    expect(v.badge).toBe("none");
    expect(v.actions.configure).toBe(true);
    expect(v.actions.connect).toBe(false);
  });

  it("undefined authMode behaves as bearer", () => {
    expect(row({ authMode: undefined, hasSecret: true }).badge).toBe("secret");
  });

  it("dims when switched off", () => {
    expect(row({ authMode: "bearer", hasSecret: true, enabled: false }).dimmed).toBe(true);
    expect(row({ authMode: "bearer", hasSecret: true, enabled: true }).dimmed).toBe(false);
  });
});

describe("oauth rows", () => {
  it("none → not-connected badge, Connect only", () => {
    const v = row({ authMode: "oauth", oauthState: "none" });
    expect(v.badge).toBe("not-connected");
    expect(v.actions).toEqual({ connect: true, reconnect: false, disconnect: false, configure: false });
    expect(v.dimmed).toBe(false);
  });

  it("undefined oauthState reads as none (Connect offered)", () => {
    const v = row({ authMode: "oauth", oauthState: undefined });
    expect(v.badge).toBe("not-connected");
    expect(v.actions.connect).toBe(true);
  });

  it("authorizing → authorizing badge, no action buttons", () => {
    const v = row({ authMode: "oauth", oauthState: "authorizing" });
    expect(v.badge).toBe("authorizing");
    expect(v.actions).toEqual({ connect: false, reconnect: false, disconnect: false, configure: false });
  });

  it("reconnect → reconnect badge, Reconnect + Disconnect", () => {
    const v = row({ authMode: "oauth", oauthState: "reconnect" });
    expect(v.badge).toBe("reconnect");
    expect(v.actions.reconnect).toBe(true);
    expect(v.actions.disconnect).toBe(true);
    expect(v.actions.connect).toBe(false);
    expect(v.actions.configure).toBe(false);
  });

  it("authorized + enabled → authorized badge, Disconnect + Configure, not dimmed", () => {
    const v = row({ authMode: "oauth", oauthState: "authorized", enabled: true });
    expect(v.badge).toBe("authorized");
    expect(v.actions).toEqual({ connect: false, reconnect: false, disconnect: true, configure: true });
    expect(v.dimmed).toBe(false);
  });

  it("authorized + disabled → authorized-muted, Configure only, dimmed", () => {
    const v = row({ authMode: "oauth", oauthState: "authorized", enabled: false });
    expect(v.badge).toBe("authorized-muted");
    expect(v.actions).toEqual({ connect: false, reconnect: false, disconnect: false, configure: true });
    expect(v.dimmed).toBe(true);
  });
});
