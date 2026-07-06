// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { deriveMcpOAuthStage, showMcpOAuthConnectPanel } from "./mcp-oauth-stage";

const stage = (over) =>
  deriveMcpOAuthStage({
    state: undefined,
    attempted: false,
    hasError: false,
    sawAuthorizing: false,
    ...over,
  });

describe("mcp oauth connect stage", () => {
  it("resting → idle (Connect button)", () => {
    expect(stage({ state: "none" })).toBe("idle");
    expect(stage({ state: "reconnect" })).toBe("idle");
    expect(stage({ state: undefined })).toBe("idle");
  });

  it("just clicked, begin in flight → optimistic authorizing", () => {
    // attempted, state still none (add/begin not resolved), never saw authorizing.
    expect(stage({ state: "none", attempted: true })).toBe("authorizing");
    expect(stage({ state: undefined, attempted: true })).toBe("authorizing");
  });

  it("live backend authorizing → authorizing, even before an attempt", () => {
    expect(stage({ state: "authorizing" })).toBe("authorizing");
    expect(stage({ state: "authorizing", attempted: true })).toBe("authorizing");
  });

  it("authorized → authorized (terminal, ignores flags)", () => {
    expect(stage({ state: "authorized" })).toBe("authorized");
    expect(stage({ state: "authorized", attempted: true, sawAuthorizing: true })).toBe("authorized");
  });

  it("begin threw → denied", () => {
    expect(stage({ state: "none", attempted: true, hasError: true })).toBe("denied");
  });

  it("browser round-trip fell back after authorizing → denied", () => {
    // saw authorizing, then state dropped to none/reconnect (cancel/deny/lapse).
    expect(stage({ state: "none", attempted: true, sawAuthorizing: true })).toBe("denied");
    expect(stage({ state: "reconnect", attempted: true, sawAuthorizing: true })).toBe("denied");
  });

  it("retry resets attempt → back to idle", () => {
    // Retry clears attempted/sawAuthorizing even if a stale error lingers.
    expect(stage({ state: "none", attempted: false, hasError: true })).toBe("idle");
  });
});

describe("mcp oauth connect-panel routing", () => {
  const panel = (over) =>
    showMcpOAuthConnectPanel({
      hasConnectServer: false,
      step: "connect",
      isOAuthPreset: false,
      edit: false,
      ...over,
    });

  it("row Connect/Reconnect always shows the connect panel", () => {
    expect(panel({ hasConnectServer: true })).toBe(true);
  });

  it("catalog-add of a hosted-OAuth preset shows the connect panel", () => {
    expect(panel({ isOAuthPreset: true })).toBe(true);
  });

  it("Configure/edit of an existing OAuth connector shows the EDIT form, not the connect panel", () => {
    // Regression: edit mode carries no connect id, so the connect panel's Connect
    // button takes the catalog-add branch and re-adds a DUPLICATE connector.
    expect(panel({ isOAuthPreset: true, edit: true })).toBe(false);
  });

  it("catalog grid / non-oauth preset never shows the connect panel", () => {
    expect(panel({ step: "catalog", isOAuthPreset: true })).toBe(false);
    expect(panel({ isOAuthPreset: false })).toBe(false);
  });
});
