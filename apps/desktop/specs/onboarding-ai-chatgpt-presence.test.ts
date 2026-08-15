// Regression: a ChatGPT sign-in lands OUTSIDE the key-input flow. The device-code
// login writes the token set straight into the vault slot and the backend emits
// `chatgpt_login_update`; onboarding's listener then re-proves the instance.
//
// Re-proving is not enough. The onboarding card's "✓ signed in" pill and
// ChatgptConnect's Reconnect/Disconnect affordances all read the vault-presence
// map, and nothing re-probes it after the login — `init()` and `addProvider` are
// its only other callers and both ran before the login started. So the engine
// verifies live (Finish unblocks) while the card still says "Connect ChatGPT"
// and Disconnect is unreachable for the rest of onboarding.
//
// Settings already gets this right via `handleChatgptConnectionChange`; this is
// the onboarding half of the same contract.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL store with Svelte's compiler under node (same harness as
// onboarding-ai-verify-race.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

// Fake backend keyed off one bit: does the vault hold a ChatGPT token set?
// `verify_ai_provider` lists the static model set only while it does, mirroring
// `list_models_for_provider`'s `needs_reconnect:<id>` gate.
let tokenInVault = false;

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: { request?: { provider?: string } }) => {
    if (cmd === "ai_runtime_has_provider_key") return tokenInVault;
    if (cmd === "verify_ai_provider") {
      if (!tokenInVault) throw `needs_reconnect:${args?.request?.provider}`;
      return { models: ["gpt-5.6-sol", "gpt-5.5"], latencyMs: 12 };
    }
    return undefined;
  },
  convertFileSrc: (p: string) => p,
}));

beforeAll(() => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], { stdio: "inherit" });
  if (built.status !== 0) throw new Error("precompile failed");
});

type Store = {
  addProvider: (kind: string, baseUrl?: string) => string | null;
  handleChatgptChanged: (id: string) => Promise<void>;
  aiVerifications: Record<string, { status: string }>;
  aiProviderKeySaved: Record<string, boolean>;
};

test("a landed ChatGPT sign-in flips the card to signed in, not just verified", async () => {
  tokenInVault = false;
  const { createOnboardingAiStore } = (await import("./_reactivity/gen/onboarding-ai.js")) as {
    createOnboardingAiStore: () => Store;
  };
  const ai = createOnboardingAiStore();
  const id = ai.addProvider("chatgpt");
  expect(id).toBeTruthy();

  // The user approved the device code in the browser: the token set is in the
  // vault and `chatgpt_login_update { connected: true }` just arrived.
  tokenInVault = true;
  await ai.handleChatgptChanged(id!);

  expect(ai.aiVerifications[id!].status).toBe("live");
  expect(ai.aiProviderKeySaved[id!]).toBe(true);
});

test("a disconnect drops the instance back to not signed in", async () => {
  tokenInVault = false;
  const { createOnboardingAiStore } = (await import("./_reactivity/gen/onboarding-ai.js")) as {
    createOnboardingAiStore: () => Store;
  };
  const ai = createOnboardingAiStore();
  const id = ai.addProvider("chatgpt");

  tokenInVault = true;
  await ai.handleChatgptChanged(id!);
  expect(ai.aiProviderKeySaved[id!]).toBe(true);

  // "Disconnect" cleared the vault slot.
  tokenInVault = false;
  await ai.handleChatgptChanged(id!);
  expect(ai.aiProviderKeySaved[id!]).toBe(false);
  expect(ai.aiVerifications[id!].status).toBe("error");
});
