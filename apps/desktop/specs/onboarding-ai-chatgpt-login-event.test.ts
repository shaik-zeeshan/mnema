// Regression: the terminal `chatgpt_login_update` must be owned by something
// that outlives the onboarding SCREENS.
//
// The backend poll runs for up to 15 minutes and emits exactly one terminal
// event; there is no snapshot command to re-read it afterwards. `AiSetup` used
// to own the subscription, but it is mounted inside the tab fork of
// ChangeSettingsScreen — clicking Screen capture / Engines / Models, or
// stepping off the screen, unmounts it and its listener. The user is in the
// browser approving a code precisely when that is most likely, so the outcome
// would land in the vault while the card stayed "not tested" and Finish stayed
// blocked, with no error and no retry affordance.
//
// Owning it in the store (which hangs off OnboardingController) is what makes
// the outcome unmissable. These tests drive the store directly — the same thing
// that survives the unmount.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL store with Svelte's compiler under node (same harness as
// onboarding-ai-verify-race.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

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

// Stand in for the Tauri event bus: capture the handler the store registers so
// the test can deliver the one terminal event the backend would.
type LoginUpdate = { providerId: string; connected: boolean; error?: string };
let deliver: ((payload: LoginUpdate) => void) | null = null;
let unlistenCalls = 0;

mock.module("@tauri-apps/api/event", () => ({
  listen: async (event: string, handler: (e: { payload: LoginUpdate }) => void) => {
    if (event === "chatgpt_login_update") {
      deliver = (payload) => handler({ payload });
    }
    return () => {
      unlistenCalls += 1;
    };
  },
}));

beforeAll(() => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], { stdio: "inherit" });
  if (built.status !== 0) throw new Error("precompile failed");
});

type Store = {
  init: () => void;
  addProvider: (kind: string, baseUrl?: string) => string | null;
  disposeChatgptLoginUpdates: () => void;
  aiVerifications: Record<string, { status: string; reason?: string }>;
  aiProviderKeySaved: Record<string, boolean>;
  draftAiDefaultModel: { provider: string; model: string } | null;
};

async function freshStore(): Promise<Store> {
  deliver = null;
  const { createOnboardingAiStore } = (await import(
    "./_reactivity/gen/onboarding-ai.js"
  )) as { createOnboardingAiStore: () => Store };
  const ai = createOnboardingAiStore();
  ai.init();
  return ai;
}

const settled = () => new Promise((r) => setTimeout(r, 0));

test("the store subscribes at init, so no screen has to be mounted to catch the outcome", async () => {
  tokenInVault = false;
  const ai = await freshStore();
  expect(deliver).not.toBeNull();
  ai.disposeChatgptLoginUpdates();
});

test("a landed sign-in verifies the instance and seeds the default model", async () => {
  tokenInVault = false;
  const ai = await freshStore();
  const id = ai.addProvider("chatgpt");
  expect(id).toBeTruthy();

  // The user approved the code in the browser, long after AiSetup was unmounted.
  tokenInVault = true;
  deliver!({ providerId: id!, connected: true });
  await settled();
  await settled();

  expect(ai.aiVerifications[id!].status).toBe("live");
  expect(ai.aiProviderKeySaved[id!]).toBe(true);
  // First engine to answer seeds the default model — the step that used to live
  // in AiSetup.reportVerification, which is exactly what is not mounted here.
  expect(ai.draftAiDefaultModel).toEqual({ provider: id!, model: "gpt-5.6-sol" });
  ai.disposeChatgptLoginUpdates();
});

test("a failed sign-in surfaces instead of vanishing", async () => {
  tokenInVault = false;
  const ai = await freshStore();
  const id = ai.addProvider("chatgpt");

  // The old handler started with `if (!connected) return`, so a timed-out or
  // refused login was dropped entirely — leaving the user watching a spinner
  // that would never resolve.
  deliver!({ providerId: id!, connected: false, error: "the code expired" });
  await settled();

  expect(ai.aiVerifications[id!].status).toBe("error");
  expect(ai.aiVerifications[id!].reason).toContain("expired");
  ai.disposeChatgptLoginUpdates();
});

test("an event for a provider this run never created is ignored", async () => {
  tokenInVault = true;
  const ai = await freshStore();
  const id = ai.addProvider("chatgpt");

  deliver!({ providerId: "chatgpt-from-another-window", connected: true });
  await settled();

  expect(ai.aiVerifications["chatgpt-from-another-window"]).toBeUndefined();
  expect(ai.aiVerifications[id!]).toBeUndefined();
  ai.disposeChatgptLoginUpdates();
});
