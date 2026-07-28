// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
// Slice 11 (AI setup): the readiness rule, ported from the mockup's
// `console.assert` self-check in
// `docs/onboarding/mockups/input-components/parts/aisetup.part.html`.
import { describe, expect, test } from "bun:test";
import {
  aiReadinessMissing,
  aiReadinessReady,
  aiVerificationWord,
  type AiReadinessInput,
} from "./ai-readiness";
import { newAiProviderId } from "$lib/settings/state/ai-providers";

const OLLAMA_MODELS = ["llama3.1:8b", "qwen2.5:14b", "gemma3:12b"];
const ANTHROPIC_MODELS = ["claude-opus-4-1", "claude-sonnet-4-5", "claude-haiku-4-5"];

function input(overrides: Partial<AiReadinessInput> = {}): AiReadinessInput {
  return {
    providers: [{ id: "ollama", label: "Ollama · localhost:11434" }],
    verifications: {},
    defaultModel: { provider: "ollama", model: "llama3.1:8b" },
    ...overrides,
  };
}

describe("slice 11 — AI readiness is live verification, not a stored string", () => {
  test("no connected provider blocks", () => {
    expect(aiReadinessMissing(input({ providers: [], defaultModel: null }))).toBe(
      "Connect a reasoning engine to use Ask AI.",
    );
  });

  test("an unverified provider is never ready", () => {
    // Production's whole test is "the key reached the keychain" — here a
    // provider that has not answered this session cannot be ready, cloud or not.
    expect(aiReadinessReady(input())).toBe(false);
    expect(aiReadinessMissing(input())).toBe("Verify Ollama · localhost:11434 before finishing.");
  });

  test("a local kind gets the same endpoint check as a cloud kind", () => {
    // The old rule exempted non-cloud kinds from any check at all.
    const local = input({
      providers: [{ id: "llamafile", label: "Llamafile · localhost:8080" }],
      defaultModel: { provider: "llamafile", model: "tinyllama" },
    });
    expect(aiReadinessReady(local)).toBe(false);
  });

  test("a live provider listing the model is ready", () => {
    const ready = input({
      verifications: { ollama: { status: "live", models: OLLAMA_MODELS, latencyMs: 412 } },
    });
    expect(aiReadinessMissing(ready)).toBeNull();
    expect(aiReadinessReady(ready)).toBe(true);
  });

  test("a default model absent from the live listing blocks, naming it", () => {
    const stale = input({
      verifications: {
        ollama: {
          status: "live",
          models: OLLAMA_MODELS.filter((m) => m !== "llama3.1:8b"),
          latencyMs: 412,
        },
      },
    });
    expect(aiReadinessMissing(stale)).toBe(
      "Ollama · localhost:11434 no longer lists llama3.1:8b — pick another.",
    );
  });

  test("a rejected key blocks and carries the provider's reason", () => {
    const rejected = input({
      providers: [{ id: "anthropic", label: "Anthropic" }],
      verifications: { anthropic: { status: "error", reason: "authentication failed" } },
      defaultModel: { provider: "anthropic", model: "claude-opus-4-1" },
    });
    expect(aiReadinessMissing(rejected)).toBe(
      "Anthropic is not answering — authentication failed.",
    );
  });

  test("an unreachable endpoint blocks", () => {
    const dead = input({
      verifications: { ollama: { status: "error", reason: "unreachable" } },
    });
    expect(aiReadinessMissing(dead)).toBe(
      "Ollama · localhost:11434 is not answering — unreachable.",
    );
  });

  test("no model chosen reads differently before and after any engine verifies", () => {
    expect(aiReadinessMissing(input({ defaultModel: null }))).toBe(
      "No engine has verified yet — nothing can answer.",
    );
    expect(
      aiReadinessMissing(
        input({
          defaultModel: null,
          verifications: { ollama: { status: "live", models: OLLAMA_MODELS, latencyMs: 9 } },
        }),
      ),
    ).toBe("Choose a default model.");
  });

  test("a model pointing at a removed provider blocks", () => {
    expect(
      aiReadinessMissing(
        input({
          providers: [{ id: "anthropic", label: "Anthropic" }],
          verifications: { anthropic: { status: "live", models: ANTHROPIC_MODELS, latencyMs: 8 } },
        }),
      ),
    ).toBe("Pick a default model from a connected engine.");
  });

  test("two same-kind instances verify independently (ADR 0035)", () => {
    const first = newAiProviderId("ollama", []);
    const second = newAiProviderId("ollama", [first]);
    expect([first, second]).toEqual(["ollama", "ollama-2"]);

    const rack: AiReadinessInput = {
      providers: [
        { id: first, label: "Ollama · localhost:11434" },
        { id: second, label: "Ollama · studio.local:11434" },
      ],
      // Only the second box answered.
      verifications: { [second]: { status: "live", models: ["llama3.3:70b"], latencyMs: 631 } },
      defaultModel: { provider: first, model: "llama3.1:8b" },
    };
    expect(aiReadinessReady(rack)).toBe(false);
    expect(aiReadinessReady({ ...rack, defaultModel: { provider: second, model: "llama3.3:70b" } })).toBe(
      true,
    );
  });

  test("a verification in flight is not yet ready", () => {
    const checking = input({ verifications: { ollama: { status: "checking" } } });
    expect(aiReadinessReady(checking)).toBe(false);
    expect(aiReadinessMissing(checking)).toBe("Verifying Ollama · localhost:11434…");
  });
});

describe("slice 11 — the status word is counted, not pasted", () => {
  test("a live listing reports its own count and measured latency", () => {
    expect(aiVerificationWord({ status: "live", models: OLLAMA_MODELS, latencyMs: 412 })).toBe(
      "3 models · 412 ms",
    );
    expect(aiVerificationWord({ status: "live", models: ["only-one"], latencyMs: 7 })).toBe(
      "1 model · 7 ms",
    );
  });

  test("the other three states", () => {
    expect(aiVerificationWord(undefined)).toBe("not tested");
    expect(aiVerificationWord({ status: "checking" })).toBe("checking…");
    expect(aiVerificationWord({ status: "error", reason: "unreachable" })).toBe("unreachable");
  });
});
