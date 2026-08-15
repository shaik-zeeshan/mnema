// Regression: "Test connection" is the one reason-code surface in the AI
// settings store that renders the backend code verbatim.
//
// `ai_runtime_test_connection` resolves the default model's engine through
// `resolve_engine_config_live`, so for a `chatgpt` default model it fails with
// exactly the two codes this PR introduced: `needs_reconnect:<id>` (OpenAI
// rejected the grant — sign in again) and `provider_unreachable:<id>` (nothing
// rendered a verdict — do NOT disconnect a healthy account). Every other
// surface translates them (`aiRuntimeReasonLabel`, `aiListingFailureCopy`,
// `pendingReasonCopy`); this one pipes the rejection straight through
// `humanizeError`, which only upper-cases the first letter — so the banner
// reads "Needs_reconnect:chatgpt".
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL store with Svelte's compiler under node (same harness as
// ai-runtime-chatgpt-presence-race.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

// What `ai_runtime_test_connection` rejects with this run.
let testConnectionError = "needs_reconnect:chatgpt";

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string) => {
    if (cmd === "ai_runtime_test_connection") throw testConnectionError;
    if (cmd === "get_ai_runtime_status") return { available: false, reason: null };
    if (cmd === "ai_runtime_has_provider_key") return false;
    return undefined;
  },
  convertFileSrc: (p: string) => p,
}));

beforeAll(() => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], { stdio: "inherit" });
  if (built.status !== 0) throw new Error("precompile failed");
});

type Store = {
  runAiRuntimeTestConnection: () => Promise<void>;
  aiRuntimeTestError: string | null;
};

function freshStore(): Promise<Store> {
  return import("./_reactivity/gen/ai-runtime.js").then((m) =>
    (m as { createAiRuntimeStore: (deps: unknown) => Store }).createAiRuntimeStore({
      getProviders: () => [{ id: "chatgpt", kind: "chatgpt", label: "", baseUrl: "" }],
      getMcpServers: () => [],
      isCloudProviderKind: (kind: string) => kind !== "ollama" && kind !== "llamafile",
      labelForProvider: (id: string) => (id === "chatgpt" ? "ChatGPT" : id),
      loadAskAiAvailability: () => {},
    }),
  );
}

test("a spent ChatGPT login names the fix instead of printing the reason code", async () => {
  testConnectionError = "needs_reconnect:chatgpt";
  const store = await freshStore();

  await store.runAiRuntimeTestConnection();

  const banner = store.aiRuntimeTestError ?? "";
  expect(banner.toLowerCase()).not.toContain("needs_reconnect");
  expect(banner).toContain("ChatGPT");
  expect(banner.toLowerCase()).toContain("sign in");
});

test("an unreachable auth endpoint does not tell the user to sign in again", async () => {
  // ADR 0048's rule: telling an offline user to reconnect invites them to
  // disconnect a credential that is fine.
  testConnectionError = "provider_unreachable:chatgpt";
  const store = await freshStore();

  await store.runAiRuntimeTestConnection();

  const banner = store.aiRuntimeTestError ?? "";
  expect(banner.toLowerCase()).not.toContain("provider_unreachable");
  expect(banner.toLowerCase()).not.toContain("sign in");
  expect(banner).toContain("ChatGPT");
});

test("a plain transport failure is still tidied, not passed through raw", async () => {
  testConnectionError = "  error: connection refused  ";
  const store = await freshStore();

  await store.runAiRuntimeTestConnection();

  expect(store.aiRuntimeTestError).toBe("Connection refused");
});
