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
// INVARIANT 2 — DOUBLE-MINT ON CONCURRENT DUPLICATE order.paid.
// Idempotency is get-then-put (no compare-and-swap): a concurrent duplicate
// delivery (Polar retries on timeout) passes the get in both invocations and
// both mint. Order-anchored dating makes the mint deterministic, so both
// produce the SAME key bytes — never two distinct valid licenses for one order.
test("concurrent duplicate order.paid mints at most one distinct key", async () => {
  const { env } = fakeEnv();
  const realFetch = globalThis.fetch;
  let emailCount = 0;
  globalThis.fetch = (async () => {
    emailCount++;
    await Promise.resolve(); // yield, like a real network round-trip
    return new Response("{}", { status: 200 });
  }) as typeof fetch;

  try {
    const order = {
      id: "ord_dup_race",
      billing_reason: "purchase",
      product_id: "prod_license",
      created_at: "2026-07-01T12:00:00.000Z",
      customer: { email: "buyer@example.com" },
    };
    // Distinct per-invocation `now` — exactly what previously produced two keys.
    const results = await Promise.all([
      handleOrderPaid(order, env, 1_700_000_000_000),
      handleOrderPaid(order, env, 1_700_000_000_777),
    ]);

    const mintedKeys = new Set(
      results.filter((r) => r.status === "minted").map((r) => r.key),
    );
    expect(mintedKeys.size).toBeLessThanOrEqual(1);
    expect(emailCount).toBeLessThanOrEqual(2); // same key at most twice — never two keys
  } finally {
    globalThis.fetch = realFetch;
  }
});

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
