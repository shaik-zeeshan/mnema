import { test, expect } from "bun:test";
import defaultExport, { type Env } from "../src/index";
import { bytesToBase64 } from "../src/util";

const SEED = new Uint8Array(32).fill(5);

// Awaitable KV mock: get/put/list/delete each yield to the microtask queue on
// every await, exactly like a real KV round-trip, so two concurrent handler
// invocations INTERLEAVE at their await points inside ONE isolate. Backed by one
// strongly-consistent Map with NO compare-and-swap (last write wins per key).
function awaitableEnv(): { env: Env; store: Map<string, string> } {
  const store = new Map<string, string>();
  const env = {
    ED25519_PRIVATE_KEY: bytesToBase64(SEED),
    POLAR_LICENSE_PRODUCT_ID: "prod_license",
    POLAR_RENEWAL_PRODUCT_ID: "prod_renewal",
    RESEND_API_KEY: "re_test",
    UPDATE_WINDOW_DAYS: "365",
    IDEMPOTENCY: {
      get: async (k: string) => { await Promise.resolve(); return store.get(k) ?? null; },
      put: async (k: string, v: string) => { await Promise.resolve(); store.set(k, v); },
      delete: async (k: string) => { await Promise.resolve(); store.delete(k); },
      list: async ({ prefix }: { prefix: string }) => {
        await Promise.resolve();
        const keys = [...store.keys()].filter((k) => k.startsWith(prefix)).map((name) => ({ name }));
        return { keys, list_complete: true, cursor: undefined };
      },
    },
  } as unknown as Env;
  return { env, store };
}

const LIC = "order:cap-race";
function activate(env: Env, hash: string): Promise<Response> {
  return defaultExport.fetch(
    new Request("https://f/activate", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ schema: 1, license_id: LIC, machine_hash: hash }),
    }),
    env,
  );
}

// INVARIANT: the 3-device cap can never be exceeded, even when two brand-new
// machines activate concurrently in the same isolate. Both read count < CAP in
// their pre-check and both blind-write their slot; without a post-claim re-verify
// the cap silently becomes 4.
test("cap: two concurrent activations past a 2-slot license stay within the cap of 3", async () => {
  const { env, store } = awaitableEnv();
  store.set(`activation:${LIC}:${"a".repeat(64)}`, "1");
  store.set(`activation:${LIC}:${"b".repeat(64)}`, "1");

  await Promise.all([
    activate(env, "c".repeat(64)),
    activate(env, "d".repeat(64)),
  ]);

  const slots = [...store.keys()].filter((k) => k.startsWith(`activation:${LIC}:`)).length;
  expect(slots).toBeLessThanOrEqual(3);
  // The two pre-existing machines are never evicted — only a losing newcomer withdraws.
  expect(store.has(`activation:${LIC}:${"a".repeat(64)}`)).toBe(true);
  expect(store.has(`activation:${LIC}:${"b".repeat(64)}`)).toBe(true);
});
