// @ts-nocheck — run under `bun test` (bun:test types aren't in svelte-check's tsconfig).
import { describe, expect, it } from "bun:test";
import { contextWindowForModel, modelDisplayName, shortModelLabel } from "./conversation";

describe("modelDisplayName", () => {
  it("names Claude models by family and version, dropping the date", () => {
    expect(modelDisplayName("claude-sonnet-4-5-20250929")).toBe("Claude Sonnet 4.5");
    expect(modelDisplayName("anthropic/claude-haiku-4-5")).toBe("Claude Haiku 4.5");
    expect(modelDisplayName("claude-opus-4-latest")).toBe("Claude Opus 4");
    expect(modelDisplayName("claude-3-5-sonnet-20241022")).toBe("Claude Sonnet 3.5");
    expect(modelDisplayName("claude-3-opus-latest")).toBe("Claude Opus 3");
  });

  it("names GPT models with the variant in lower case", () => {
    expect(modelDisplayName("gpt-5-mini")).toBe("GPT-5 mini");
    expect(modelDisplayName("gpt-5")).toBe("GPT-5");
    expect(modelDisplayName("gpt-4o")).toBe("GPT-4o");
    expect(modelDisplayName("gpt-4.1-nano-2025-04-14")).toBe("GPT-4.1 nano");
  });

  it("leaves unknown ids as their short label", () => {
    expect(modelDisplayName("accounts/fireworks/models/deepseek-v4-pro")).toBe("deepseek-v4-pro");
    expect(modelDisplayName("llama3.2:3b")).toBe("llama3.2:3b");
    expect(modelDisplayName("claude-instant")).toBe("claude-instant");
  });

  it("does not change what contextWindowForModel matches on", () => {
    expect(shortModelLabel("anthropic/claude-sonnet-4-5")).toBe("claude-sonnet-4-5");
    expect(contextWindowForModel("claude-sonnet-4-5")).toBe(200_000);
  });
});
