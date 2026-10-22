// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { providerRowState, runtimeReasonTone } from "./provider-row-state";

const row = (kind, over = {}) => ({
  provider: { id: kind, kind, baseUrl: "" },
  cloud: !["ollama", "llamafile"].includes(kind),
  keySaved: false,
  runtimeReason: null,
  listFailed: false,
  listed: true,
  ...over,
});

describe("providerRowState", () => {
  it("cloud key presence, then the verify check", () => {
    expect(providerRowState(row("anthropic", { keySaved: true, listed: false }))).toEqual({ text: "key saved", tone: "ok" });
    expect(providerRowState(row("anthropic", { keySaved: true }))).toEqual({ text: "reachable", tone: "ok" });
    expect(providerRowState(row("anthropic", { keySaved: true, listFailed: true }))).toEqual({ text: "not reachable", tone: "warn" });
    expect(providerRowState(row("anthropic"))).toEqual({ text: "no key", tone: "" });
  });

  it("chatgpt splits reconnect (rejected) from unreachable (no verdict)", () => {
    expect(providerRowState(row("chatgpt", { keySaved: true })).text).toBe("connected");
    expect(providerRowState(row("chatgpt", { keySaved: true, runtimeReason: "needs_reconnect:chatgpt" })).tone).toBe("danger");
    expect(providerRowState(row("chatgpt", { keySaved: true, runtimeReason: "provider_unreachable:chatgpt" }))).toEqual({ text: "unreachable", tone: "warn" });
  });

  it("a reason only applies to the instance it names (ADR 0035)", () => {
    const second = row("chatgpt", { keySaved: true, runtimeReason: "needs_reconnect:chatgpt-2" });
    expect(providerRowState(second).text).toBe("connected");
  });

  it("local reachability follows the model-list probe", () => {
    expect(providerRowState(row("ollama"))).toEqual({ text: "reachable", tone: "ok" });
    expect(providerRowState(row("ollama", { listFailed: true }))).toEqual({ text: "not reachable", tone: "warn" });
    expect(providerRowState(row("ollama", { listed: false }))).toEqual({ text: "on this Mac", tone: "" });
  });

  it("openai-compatible without a base URL", () => {
    expect(providerRowState(row("openai_compatible", { keySaved: true })).text).toBe("needs base URL");
  });
});

describe("runtimeReasonTone", () => {
  it("rejections are danger, the rest warn", () => {
    expect(runtimeReasonTone("needs_reconnect:chatgpt")).toBe("danger");
    expect(runtimeReasonTone("no_default_model")).toBe("warn");
    expect(runtimeReasonTone("provider_unreachable:chatgpt")).toBe("warn");
  });
});
