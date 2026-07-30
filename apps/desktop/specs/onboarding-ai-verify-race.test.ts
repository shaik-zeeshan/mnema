// Regression: the onboarding AI verification probe (`verify_ai_provider`) is
// re-runnable — "Re-check" on a provider card, and every key/base-URL edit
// (`saveEdit` → `invalidateVerification` + `verifyProvider`) fires it again.
// Two round trips for the SAME instance id can therefore be in flight at once,
// and the command's latency is a network round trip: a bad key that times out
// can settle long after a good key that answered in 300 ms.
//
// Whichever call resolves LAST wins today, so a stale earlier probe overwrites
// the fresher one and `aiConfigReady` reports the wrong engine state.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles
// the REAL store with Svelte's compiler under node (same harness as
// licensing-store-race.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

// One deferred promise per `verify_ai_provider` call, so the test can settle
// them in the adversarial order (second call first, first call last).
type Deferred = { resolve: (v: unknown) => void; reject: (e: unknown) => void };
const verifyCalls: Deferred[] = [];

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string) => {
    if (cmd !== "verify_ai_provider") return undefined;
    return await new Promise((res, rej) => {
      verifyCalls.push({ resolve: res, reject: rej });
    });
  },
  convertFileSrc: (p: string) => p,
}));
// NOTE: `@tauri-apps/plugin-dialog` (imported by ai-runtime.svelte.ts) is left
// UNMOCKED on purpose — bun's module mocks fix the export-name set
// process-wide, and mocking it here breaks openCapturedUrl.test.ts's `message`.

beforeAll(() => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], { stdio: "inherit" });
  if (built.status !== 0) throw new Error("precompile failed");
});

const flush = () => new Promise((r) => setTimeout(r, 0));

test("a slow earlier verification never overwrites a fresher one", async () => {
  const { createOnboardingAiStore } = (await import("./_reactivity/gen/onboarding-ai.js")) as {
    createOnboardingAiStore: () => {
      addProvider: (kind: string, baseUrl?: string) => string | null;
      verifyProvider: (id: string) => Promise<void>;
      aiVerifications: Record<string, { status: string }>;
    };
  };

  const ai = createOnboardingAiStore();
  const id = ai.addProvider("ollama", "http://localhost:11434");
  expect(id).toBeTruthy();

  // First probe: the old, wrong endpoint. Slow — still hanging.
  const first = ai.verifyProvider(id!);
  await flush();
  // The user fixes the endpoint and re-checks. Second probe, fast.
  const second = ai.verifyProvider(id!);
  await flush();
  expect(verifyCalls.length).toBe(2);

  // Fresh answer lands first: the engine is live.
  verifyCalls[1].resolve({ models: ["llama3"], latencyMs: 12 });
  await second;
  await flush();
  expect(ai.aiVerifications[id!].status).toBe("live");

  // The stale first probe finally fails, long after the user moved on.
  verifyCalls[0].reject(new Error("connection refused"));
  await first;
  await flush();

  // It must NOT clobber the fresher result.
  expect(ai.aiVerifications[id!].status).toBe("live");
});
