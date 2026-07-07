import { test, expect, afterEach } from "bun:test";
import * as ed from "@noble/ed25519";
import { verifyWebhook } from "../src/verify";
import { mintKey } from "../src/mint";
import { handleOrderPaid, type Env } from "../src/index";
import { bytesToBase64, base64ToBytes } from "../src/util";

const DAY_MS = 24 * 60 * 60 * 1000;

// --- helpers ---------------------------------------------------------------

async function signWebhook(secretB64: string, id: string, ts: string, body: string): Promise<Headers> {
  const key = await crypto.subtle.importKey(
    "raw",
    base64ToBytes(secretB64),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = new Uint8Array(
    await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(`${id}.${ts}.${body}`)),
  );
  return new Headers({
    "webhook-id": id,
    "webhook-timestamp": ts,
    "webhook-signature": `v1,${bytesToBase64(sig)}`,
  });
}

function fakeEnv(seedB64: string): { env: Env; store: Map<string, string> } {
  const store = new Map<string, string>();
  const env = {
    ED25519_PRIVATE_KEY: seedB64,
    POLAR_WEBHOOK_SECRET: "unused-here",
    RESEND_API_KEY: "re_test",
    POLAR_LICENSE_PRODUCT_ID: "prod_license",
    POLAR_RENEWAL_PRODUCT_ID: "prod_renewal",
    UPDATE_WINDOW_DAYS: "365",
    IDEMPOTENCY: {
      get: async (k: string) => store.get(k) ?? null,
      put: async (k: string, v: string) => {
        store.set(k, v);
      },
    },
  } as unknown as Env;
  return { env, store };
}

let emailCount = 0;
const realFetch = globalThis.fetch;
function mockResend() {
  emailCount = 0;
  globalThis.fetch = (async () => {
    emailCount++;
    return new Response("{}", { status: 200 });
  }) as typeof fetch;
}
afterEach(() => {
  globalThis.fetch = realFetch;
});

// --- (a) webhook signature -------------------------------------------------

test("webhook verify: accepts correctly-signed, rejects tampered (base64-decodes the secret)", async () => {
  const secretB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(24)));
  const ts = String(Math.floor(Date.now() / 1000));
  const body = JSON.stringify({ type: "order.paid", data: { id: "ord_1" } });

  const headers = await signWebhook(secretB64, "msg_1", ts, body);
  expect(await verifyWebhook(body, headers, secretB64)).toBe(true);
  // whsec_ prefix is stripped before base64-decoding:
  expect(await verifyWebhook(body, headers, `whsec_${secretB64}`)).toBe(true);

  // Tampered body -> signature no longer matches.
  expect(await verifyWebhook(body + "x", headers, secretB64)).toBe(false);
  // Wrong secret -> reject.
  const otherB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(24)));
  expect(await verifyWebhook(body, headers, otherB64)).toBe(false);
});

// --- (b) idempotency -------------------------------------------------------

test("idempotency: duplicate order id does not re-mint", async () => {
  mockResend();
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const { env } = fakeEnv(seedB64);
  const order = { id: "ord_dup", billing_reason: "purchase", product_id: "prod_license", customer: { email: "a@b.co" } };

  const first = await handleOrderPaid(order, env);
  const second = await handleOrderPaid(order, env);

  expect(first.status).toBe("minted");
  expect(second.status).toBe("duplicate");
  expect(emailCount).toBe(1); // minted + emailed exactly once
});

// --- (c) minted key format + signature verifies against public key ---------

test("minted key: split on '.', base64-decode -> compact JSON with ms timestamps, sig verifies", async () => {
  const seed = crypto.getRandomValues(new Uint8Array(32));
  const pub = await ed.getPublicKeyAsync(seed);

  const issued = 1_700_000_000_000; // ms
  const through = issued + 365 * DAY_MS;
  const key = await mintKey(
    { email: "buyer@x.io", license_id: "11111111-1111-1111-1111-111111111111", tier: "license", issued_at: issued, update_through: through },
    seed,
  );

  const [payloadB64, sigB64] = key.split(".");
  expect(payloadB64).toBeTruthy();
  expect(sigB64).toBeTruthy();

  const payloadBytes = base64ToBytes(payloadB64);
  const json = new TextDecoder().decode(payloadBytes);

  // Compact JSON, exact field order, ms-integer timestamps.
  expect(json).toBe(
    `{"email":"buyer@x.io","license_id":"11111111-1111-1111-1111-111111111111","tier":"license","issued_at":${issued},"update_through":${through}}`,
  );
  const parsed = JSON.parse(json);
  expect(parsed.issued_at).toBe(issued);
  expect(parsed.update_through).toBe(through);

  // Signature is Ed25519 over the raw payload bytes, verifies against the public key.
  const ok = await ed.verifyAsync(base64ToBytes(sigB64), payloadBytes, pub);
  expect(ok).toBe(true);

  // Tampered payload fails verification.
  const bad = new Uint8Array(payloadBytes);
  bad[0] ^= 0xff;
  expect(await ed.verifyAsync(base64ToBytes(sigB64), bad, pub)).toBe(false);
});

// --- (d) purchase and renewal both give update_through = issued_at + 365d ---

test("purchase and renewal both mint update_through = issued_at + 365d", async () => {
  mockResend();
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const now = 1_700_000_000_000;

  for (const productId of ["prod_license", "prod_renewal"]) {
    const { env } = fakeEnv(seedB64);
    const order = { id: `ord_${productId}`, billing_reason: "purchase", product_id: productId, customer: { email: "c@d.io" } };
    const res = await handleOrderPaid(order, env, now);
    expect(res.status).toBe("minted");

    const payload = JSON.parse(new TextDecoder().decode(base64ToBytes(res.key!.split(".")[0])));
    expect(payload.tier).toBe("license");
    expect(payload.issued_at).toBe(now);
    expect(payload.update_through - payload.issued_at).toBe(365 * DAY_MS);
  }
});

// --- unknown product is ACKed, not minted ----------------------------------

test("unknown product: ACK without minting", async () => {
  mockResend();
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const { env } = fakeEnv(seedB64);
  const res = await handleOrderPaid(
    { id: "ord_unknown", billing_reason: "purchase", product_id: "prod_other", customer: { email: "e@f.io" } },
    env,
  );
  expect(res.status).toBe("unknown-product");
  expect(emailCount).toBe(0);
});
