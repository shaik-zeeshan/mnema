// Regression: the LIFECYCLE of the `chatgpt_login_update` subscription, not its
// payload handling (that is onboarding-ai-chatgpt-login-event.test.ts).
//
// `listen()` returns a PROMISE of the unlisten function — the handler is
// registered by the backend a tick (or an IPC round trip) after the call. The
// store parks the unlisten in `.then()`, so every teardown that runs inside that
// window sees `chatgptLoginUnlisten === null`, does nothing, and the listener is
// registered a moment later with nobody left holding its unlisten: it is
// unreachable and permanent. Every other listener in this app guards that window
// (`onboarding-listeners.ts`, `+page.svelte`, `ChatgptConnect.svelte`,
// `settings/panels/intelligence/Providers.svelte` all use a `destroyed` latch);
// this one did not.
//
// The second test is the one that shows up as behaviour: a leaked handler is not
// idle, it re-runs the whole landed-sign-in path — a vault probe plus a real
// `verify_ai_provider` network round trip — once per leaked subscription.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL store with Svelte's compiler under node (same harness as
// onboarding-ai-chatgpt-login-event.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const here = resolve(import.meta.dir, "_reactivity");

type LoginUpdate = { providerId: string; connected: boolean; error?: string };
type Sub = { handler: (e: { payload: LoginUpdate }) => void; live: boolean };

let subs: Sub[] = [];
let pendingListens: Array<() => void> = [];
let unlistenCalls = 0;
let verifyCalls = 0;

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string) => {
    if (cmd === "ai_runtime_has_provider_key") return true;
    if (cmd === "verify_ai_provider") {
      verifyCalls += 1;
      return { models: ["gpt-5.6-sol"], latencyMs: 7 };
    }
    return undefined;
  },
  convertFileSrc: (p: string) => p,
}));

// A `listen()` that resolves ONLY when the test says so — the window the real
// one has, made deterministic.
mock.module("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (e: { payload: LoginUpdate }) => void) => {
    const sub: Sub = { handler, live: true };
    if (event === "chatgpt_login_update") subs.push(sub);
    return new Promise<() => void>((res) => {
      pendingListens.push(() =>
        res(() => {
          sub.live = false;
          unlistenCalls += 1;
        }),
      );
    });
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
};

async function freshStore(): Promise<Store> {
  subs = [];
  pendingListens = [];
  unlistenCalls = 0;
  verifyCalls = 0;
  const { createOnboardingAiStore } = (await import("./_reactivity/gen/onboarding-ai.js")) as {
    createOnboardingAiStore: () => Store;
  };
  return createOnboardingAiStore();
}

/** Let every outstanding `listen()` resolve its unlisten function. */
const resolveListens = () => {
  const waiting = pendingListens;
  pendingListens = [];
  for (const release of waiting) release();
};
const settled = () => new Promise((r) => setTimeout(r, 0));
const deliver = (payload: LoginUpdate) => {
  for (const sub of subs) if (sub.live) sub.handler({ payload });
};

test("a teardown inside the listen() window still drops the subscription", async () => {
  const ai = await freshStore();
  ai.init();
  // The controller tears down before the IPC registration has answered — a
  // screen swap, a `goto`, a fast Skip. The subscription must not survive it.
  ai.disposeChatgptLoginUpdates();
  resolveListens();
  await settled();

  expect(unlistenCalls).toBe(1);
});

test("re-subscribing inside the listen() window leaves exactly one live handler", async () => {
  const ai = await freshStore();
  ai.init();
  ai.disposeChatgptLoginUpdates();
  ai.init();
  resolveListens();
  await settled();

  const id = ai.addProvider("chatgpt");
  expect(id).toBeTruthy();
  await settled();
  verifyCalls = 0;

  // One terminal event from the backend. A leaked duplicate handler makes the
  // store verify the same sign-in twice — two network model-list round trips.
  deliver({ providerId: id!, connected: true });
  await settled();
  await settled();
  await settled();

  expect(verifyCalls).toBe(1);
});

// The store's disposer is only a disposer if something calls it. The onboarding
// page owns the controller's lifetime (`+page.svelte` creates `OnboardingFlow`
// and unmounts on `goto("/")` at Finish/Skip), so its teardown is the one place
// that can drop the subscription; an exported disposer with no caller is a leak
// that outlives the whole screen and keeps re-verifying against a dead store.
// A full component mount isn't wired into this bun:test setup, so assert the
// structural guarantee against the source (same approach as
// onboarding-voice-clip-lifetime.test.ts / settings-mount-untrack.test.ts).
test("the onboarding page's teardown drops the ChatGPT login subscription", () => {
  const source = readFileSync(
    fileURLToPath(new URL("../src/routes/onboarding/+page.svelte", import.meta.url)),
    "utf8",
  );
  expect(source).toContain("disposeChatgptLoginUpdates()");
});
