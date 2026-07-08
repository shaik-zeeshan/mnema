import { test, expect, afterEach } from "bun:test";
import { handleOrderRefunded, handleOrderPaid, type Env } from "../src/index";
import { bytesToBase64 } from "../src/util";
import { crlRevokedIds } from "../src/crl";

const TEST_SEED = new Uint8Array(32).fill(9);

// A Cloudflare-KV-shaped mock whose get/put/list are async (they yield to the
// microtask queue on every await, exactly like real KV round-trips), backed by a
// single Map. NO compare-and-swap — last write wins on the same key, just like KV.
function fakeEnv(): { env: Env; store: Map<string, string> } {
  const store = new Map<string, string>();
  const env = {
    ED25519_PRIVATE_KEY: bytesToBase64(TEST_SEED),
    POLAR_LICENSE_PRODUCT_ID: "prod_license",
    POLAR_RENEWAL_PRODUCT_ID: "prod_renewal",
    UPDATE_WINDOW_DAYS: "365",
    RESEND_API_KEY: "re_test",
    IDEMPOTENCY: {
      get: async (k: string) => {
        await Promise.resolve();
        return store.get(k) ?? null;
      },
      put: async (k: string, v: string) => {
        await Promise.resolve();
        store.set(k, v);
      },
      list: async ({ prefix }: { prefix: string }) => {
        await Promise.resolve();
        const keys = [...store.keys()]
          .filter((k) => k.startsWith(prefix))
          .map((name) => ({ name }));
        return { keys, list_complete: true, cursor: undefined };
      },
    },
  } as unknown as Env;
  return { env, store };
}

afterEach(() => {});

// INVARIANT 1 — REVOKED-SET LOST UPDATE.
// Two order.refunded webhooks for DIFFERENT orders are delivered concurrently.
// Both readRevokedSet() see the same empty array before either put()s, so the
// second put clobbers the first. A revocation is silently lost — and because both
// handlers returned success, Polar never retries either, so it is lost forever.
test("concurrent refunds of two different orders both end up revoked", async () => {
  const { env } = fakeEnv();

  const a = handleOrderRefunded(
    { id: "AAAA", status: "refunded", product_id: "prod_license" },
    env,
    1_700_000_000_000,
  );
  const b = handleOrderRefunded(
    { id: "BBBB", status: "refunded", product_id: "prod_license" },
    env,
    1_700_000_000_001,
  );
  await Promise.all([a, b]);

  // Both revocations must survive in the source of truth AND the served CRL.
  const crl = (env.IDEMPOTENCY as unknown as { get: (k: string) => Promise<string | null> });
  const wire = await crl.get("crl");
  const served = wire ? crlRevokedIds(wire) : [];
  expect(new Set(served)).toEqual(new Set(["order:AAAA", "order:BBBB"]));
});
