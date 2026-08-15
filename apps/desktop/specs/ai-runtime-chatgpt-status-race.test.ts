// Regression: `handleChatgptConnectionChange` is the first caller of
// `loadAiRuntimeStatus` that is NOT serialised behind the `aiProviderKeyInFlight`
// latch — it is driven by the `chatgpt_login_update` event, which lands whenever
// the browser approval happens (up to 15 minutes after the click). So two of its
// refresh sequences can be in flight at once: the event's, and the one the user's
// Disconnect fires from `ChatgptConnect`'s `onchange`.
//
// `refreshAiProviderKeyPresence` was given a monotonic ticket for exactly this
// reason. `loadAiRuntimeStatus` was not: it assigns whatever answer arrives last.
// The older call read the runtime BEFORE the disconnect, so when it lands second
// the Intelligence panel reports the engine as available against a vault slot
// that no longer holds a token — the presence badge (sequenced) and the runtime
// status (unsequenced) disagree, and only a manual reload clears it.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL store with Svelte's compiler under node (same harness as
// ai-runtime-chatgpt-presence-race.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

// The vault as the backend sees it, flipped by the "disconnect" mid-test.
let connected = true;
// Parks the FIRST `get_ai_runtime_status` round trip so it can land last.
let releaseStatus: (() => void) | null = null;
let statusCalls = 0;

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: { request?: { provider?: string } }) => {
    if (cmd === "ai_runtime_has_provider_key") return connected;
    if (cmd === "get_ai_runtime_status") {
      statusCalls += 1;
      // The backend reads the world NOW; the answer travels back afterwards.
      const answer = connected
        ? { available: true, reason: null }
        : { available: false, reason: "no_provider_key:chatgpt" };
      if (statusCalls === 1) {
        await new Promise<void>((res) => {
          releaseStatus = res;
        });
      }
      return answer;
    }
    void args;
    return undefined;
  },
  convertFileSrc: (p: string) => p,
}));
// NOTE: `@tauri-apps/plugin-dialog` is left UNMOCKED on purpose — bun's module
// mocks fix the export-name set process-wide (see onboarding-ai-verify-race).

beforeAll(() => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], { stdio: "inherit" });
  if (built.status !== 0) throw new Error("precompile failed");
});

const flush = () => new Promise((r) => setTimeout(r, 0));

const providers = [{ id: "chatgpt", kind: "chatgpt", label: "", baseUrl: "" }];

test("a login-event status load cannot land over a newer disconnect's", async () => {
  const { createAiRuntimeStore } = (await import("./_reactivity/gen/ai-runtime.js")) as {
    createAiRuntimeStore: (deps: unknown) => {
      handleChatgptConnectionChange: () => Promise<void>;
      aiRuntimeStatus: { available: boolean; reason: string | null } | null;
    };
  };

  connected = true;
  releaseStatus = null;
  statusCalls = 0;

  const store = createAiRuntimeStore({
    getProviders: () => providers,
    getMcpServers: () => [],
    isCloudProviderKind: (kind: string) => kind !== "ollama" && kind !== "llamafile",
    labelForProvider: (id: string) => id,
    loadAskAiAvailability: () => {},
  });

  // A: the `chatgpt_login_update` listener's refresh. Its status round trip
  // parks with the "available" answer already in hand.
  const landed = store.handleChatgptConnectionChange();
  await flush();
  await flush();
  expect(releaseStatus).not.toBeNull();

  // B: meanwhile the user hits Disconnect. Its own refresh runs to completion
  // and records the truth: nothing is connected any more.
  connected = false;
  await store.handleChatgptConnectionChange();
  expect(store.aiRuntimeStatus?.available).toBe(false);

  // A finally lands, carrying the pre-disconnect world.
  releaseStatus!();
  await landed;
  await flush();

  expect(store.aiRuntimeStatus?.available).toBe(false);
});
