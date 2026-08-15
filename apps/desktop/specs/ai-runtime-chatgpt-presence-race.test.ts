// Regression: `handleChatgptConnectionChange` (new with the ChatGPT provider) is
// driven by the `chatgpt_login_update` Tauri event, which lands whenever the
// browser approval happens — up to 15 minutes after the click. Unlike
// `saveAiProviderKey` / `clearAiProviderKey` it is NOT serialised behind the
// `aiProviderKeyInFlight` latch, so the event's refresh can overlap the refresh
// a user-driven Disconnect fires, and both call
// `refreshAiProviderKeyPresence`.
//
// That function merges only the ids it probed, but the merge happens at the END
// of a SEQUENTIAL probe loop. So an older pass that read `chatgpt: true` before
// the token was cleared, and is still parked on a later provider's probe, merges
// its stale `true` AFTER the newer pass wrote `false` — the "connected" badge
// comes back for a provider whose token set is gone from the vault, and
// Disconnect/Reconnect render against a credential that no longer exists.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL store with Svelte's compiler under node (same harness as
// onboarding-ai-verify-race.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

// Fake vault, flipped by the "disconnect" mid-test. The second provider's probe
// is deferred so the first refresh is still mid-loop while the second completes.
let vault: Record<string, boolean> = {};
let releaseAnthropic: (() => void) | null = null;

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: { request?: { provider?: string } }) => {
    if (cmd === "ai_runtime_has_provider_key") {
      const id = args?.request?.provider ?? "";
      if (id === "anthropic" && releaseAnthropic === null) {
        await new Promise<void>((res) => {
          releaseAnthropic = res;
        });
      }
      return vault[id] ?? false;
    }
    if (cmd === "get_ai_runtime_status") return { available: true, reason: null };
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

// chatgpt is probed FIRST and anthropic second, so the parked probe sits AFTER
// the id under test — the ordering that lets a stale value land last.
const providers = [
  { id: "chatgpt", kind: "chatgpt", label: "", baseUrl: "" },
  { id: "anthropic", kind: "anthropic", label: "", baseUrl: "" },
];

test("a login-event refresh cannot resurrect a token the user just disconnected", async () => {
  const { createAiRuntimeStore } = (await import("./_reactivity/gen/ai-runtime.js")) as {
    createAiRuntimeStore: (deps: unknown) => {
      handleChatgptConnectionChange: () => Promise<void>;
      aiProviderKeySavedByProvider: Record<string, boolean>;
    };
  };

  vault = { chatgpt: true, anthropic: true };
  releaseAnthropic = null;

  const store = createAiRuntimeStore({
    getProviders: () => providers,
    getMcpServers: () => [],
    isCloudProviderKind: (kind: string) => kind !== "ollama" && kind !== "llamafile",
    labelForProvider: (id: string) => id,
    loadAskAiAvailability: () => {},
  });

  // A: the `chatgpt_login_update` listener's refresh. Reads chatgpt = true,
  // then parks on the anthropic probe.
  const landed = store.handleChatgptConnectionChange();
  await flush();
  expect(releaseAnthropic).not.toBeNull();

  // B: meanwhile the user hits Disconnect. The token is gone and the
  // disconnect's own refresh runs to completion.
  vault = { chatgpt: false, anthropic: true };
  await store.handleChatgptConnectionChange();
  expect(store.aiProviderKeySavedByProvider.chatgpt).toBe(false);

  // A finally finishes and merges the snapshot it took before the disconnect.
  releaseAnthropic!();
  await landed;
  await flush();

  // The newest observation of the vault is "no token" — a stale pass must not
  // flip the badge back to connected.
  expect(store.aiProviderKeySavedByProvider.chatgpt).toBe(false);
});
