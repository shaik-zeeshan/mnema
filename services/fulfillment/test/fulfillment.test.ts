import { test, expect, afterEach } from "bun:test";
import * as ed from "@noble/ed25519";
import { verifyWebhook } from "../src/verify";
import { mintKey } from "../src/mint";
import defaultExport, { handleOrderPaid, handleOrderRefunded, type Env } from "../src/index";
import { buildAndSignCrl, serializeCrlPayload, crlIssuedAt, crlRevokedIds } from "../src/crl";
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
    POLAR_API_BASE: "https://sandbox-api.polar.sh",
    POLAR_ACCESS_TOKEN: "polar_test",
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
let refundCount = 0;
let refundStatus = 200; // let a test simulate Polar's 422 "exceeds refundable"
let lastRefundBody: { order_id?: string; amount?: number; reason?: string } | null = null;
// License orders Polar's /v1/orders/ lookup returns for the renewal ownership gate.
let polarLicenseOrders: Array<{ status?: string }> = [];
const realFetch = globalThis.fetch;
function mockResend() {
  emailCount = 0;
  refundCount = 0;
  refundStatus = 200;
  lastRefundBody = null;
  polarLicenseOrders = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    if (url.includes("/v1/orders/")) {
      return new Response(JSON.stringify({ items: polarLicenseOrders }), { status: 200 });
    }
    if (url.includes("/v1/refunds/")) {
      refundCount++;
      lastRefundBody = JSON.parse((init?.body as string) ?? "{}");
      const body =
        refundStatus === 422
          ? JSON.stringify({ detail: [{ msg: "Refund amount exceeds refundable amount" }] })
          : "{}";
      return new Response(body, { status: refundStatus });
    }
    // Resend (or anything else) — counts as an email send.
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

test("webhook verify: accepts Polar's raw-string key scheme (whsec_ prefix as-is)", async () => {
  // Polar signs with the RAW secret string (prefix included) as the HMAC key,
  // NOT the base64-decoded Standard-Webhooks key. Verify we accept that.
  const secret = "whsec_dw8CerghgXDpNBk1FALxQycFinZvLvnaPcI7j0HwXlt";
  const id = "msg_polar";
  const ts = String(Math.floor(Date.now() / 1000));
  const body = JSON.stringify({ type: "order.paid", data: { id: "ord_x" } });

  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret), // raw string, prefix included
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = new Uint8Array(
    await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(`${id}.${ts}.${body}`)),
  );
  const headers = new Headers({
    "webhook-id": id,
    "webhook-timestamp": ts,
    "webhook-signature": `v1,${bytesToBase64(sig)}`,
  });
  expect(await verifyWebhook(body, headers, secret)).toBe(true);
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

test("purchase and renewal (from an owner) both mint update_through = issued_at + 365d", async () => {
  mockResend();
  polarLicenseOrders = [{ status: "paid" }]; // renewal buyer owns a license
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const now = 1_700_000_000_000;

  for (const productId of ["prod_license", "prod_renewal"]) {
    const { env } = fakeEnv(seedB64);
    const order = { id: `ord_${productId}`, billing_reason: "purchase", product_id: productId, customer_id: "cus_1", customer: { email: "c@d.io" } };
    const res = await handleOrderPaid(order, env, now);
    expect(res.status).toBe("minted");

    const payload = JSON.parse(new TextDecoder().decode(base64ToBytes(res.key!.split(".")[0])));
    expect(payload.tier).toBe("license");
    expect(payload.issued_at).toBe(now);
    expect(payload.update_through - payload.issued_at).toBe(365 * DAY_MS);
  }
});

// --- renewal ownership gate: no license => auto-refund, no key --------------

test("renewal without a license: refunds the NET (pre-tax) amount + notes, no key minted", async () => {
  mockResend();
  polarLicenseOrders = []; // Polar shows no license order for this customer
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const { env, store } = fakeEnv(seedB64);
  // net 2900 + tax 522 = total 3422 — we must refund 2900 (Polar adds tax on top).
  const order = { id: "ord_bypass", billing_reason: "purchase", product_id: "prod_renewal", customer_id: "cus_x", refundable_amount: 2900, customer: { email: "sneaky@x.io" } };

  const res = await handleOrderPaid(order, env);

  expect(res.status).toBe("refunded-no-license");
  expect(res.key).toBeUndefined();
  expect(refundCount).toBe(1); // refunded once
  expect(lastRefundBody?.amount).toBe(2900); // NET, not the tax-inclusive total
  expect(emailCount).toBe(1); // the "why you were refunded" note
  expect(store.get("ord_bypass")).toBeTruthy(); // terminal — idempotency recorded

  // A retry does not double-refund.
  const retry = await handleOrderPaid(order, env);
  expect(retry.status).toBe("duplicate");
  expect(refundCount).toBe(1);
});

test("renewal when the only license order was fully refunded: treated as non-owner", async () => {
  mockResend();
  polarLicenseOrders = [{ status: "refunded" }]; // revoked license doesn't count
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const { env } = fakeEnv(seedB64);
  const order = { id: "ord_revoked_owner", billing_reason: "purchase", product_id: "prod_renewal", customer_id: "cus_r", refundable_amount: 2900, customer: { email: "r@x.io" } };

  const res = await handleOrderPaid(order, env);
  expect(res.status).toBe("refunded-no-license");
  expect(refundCount).toBe(1);
});

test("renewal refund that already happened (422 exceeds refundable): treated as done, not an error", async () => {
  mockResend();
  polarLicenseOrders = [];
  refundStatus = 422; // Polar: prior refund exhausted the refundable amount
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const { env, store } = fakeEnv(seedB64);
  const order = { id: "ord_already", billing_reason: "purchase", product_id: "prod_renewal", customer_id: "cus_a", refundable_amount: 2900, customer: { email: "a@x.io" } };

  const res = await handleOrderPaid(order, env); // must NOT throw
  expect(res.status).toBe("refunded-no-license");
  expect(emailCount).toBe(1); // still sends the note
  expect(store.get("ord_already")).toBeTruthy(); // recorded → no endless retry
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

// --- Slice 1: derived license id -------------------------------------------

test("Slice 1: paid order mints license_id === order:<orderId>", async () => {
  mockResend();
  const seedB64 = bytesToBase64(crypto.getRandomValues(new Uint8Array(32)));
  const { env } = fakeEnv(seedB64);
  const orderId = "22222222-2222-2222-2222-222222222222";
  const res = await handleOrderPaid(
    { id: orderId, billing_reason: "purchase", product_id: "prod_license", customer: { email: "g@h.io" } },
    env,
  );
  const payload = JSON.parse(new TextDecoder().decode(base64ToBytes(res.key!.split(".")[0])));
  expect(payload.license_id).toBe("order:" + orderId);
});

// --- Slice 2: CRL wire format + refund revocation --------------------------

// Verify a CRL wire string's signature under a given domain-separation context.
async function verifyCrlUnder(wire: string, context: string, pub: Uint8Array): Promise<boolean> {
  const [payloadB64, sigB64] = wire.split(".");
  const payloadJson = new TextDecoder().decode(base64ToBytes(payloadB64));
  const signed = new TextEncoder().encode(context + payloadJson);
  return ed.verifyAsync(base64ToBytes(sigB64), signed, pub);
}

const TEST_SEED = new Uint8Array(32).fill(7);
const PINNED_WIRE =
  "eyJzY2hlbWEiOjEsImlzc3VlZF9hdCI6MTcwMDAwMDAwMDAwMCwicmV2b2tlZF9saWNlbnNlX2lkcyI6WyJvcmRlcjoxMTExMTExMS0xMTExLTExMTEtMTExMS0xMTExMTExMTExMTEiXX0=.XjfSyUtXSRRjn6NPWmpGwGMKBwDaXXm1qEj682a4Cdgv4755Df2ZsvRLqJdZVmLVRdAuTBaYUdyEF2xzvXwMBQ==";

test("Slice 2: serializeCrlPayload is exact compact JSON, fixed field order", () => {
  expect(serializeCrlPayload(["order:11111111-1111-1111-1111-111111111111"], 1_700_000_000_000)).toBe(
    `{"schema":1,"issued_at":1700000000000,"revoked_license_ids":["order:11111111-1111-1111-1111-111111111111"]}`,
  );
});

test("Slice 2: pinned fixture reproduces + verifies under domain context, FAILS under license context", async () => {
  const pub = await ed.getPublicKeyAsync(TEST_SEED);

  // buildAndSignCrl reproduces the pinned wire byte-for-byte with the test seed.
  const wire = await buildAndSignCrl(
    ["order:11111111-1111-1111-1111-111111111111"],
    0,
    1_700_000_000_000,
    TEST_SEED,
  );
  expect(wire).toBe(PINNED_WIRE);

  // Cross-replay both directions: valid under "mnema-crl-v1:", invalid under the
  // plain (license) context of "".
  expect(await verifyCrlUnder(PINNED_WIRE, "mnema-crl-v1:", pub)).toBe(true);
  expect(await verifyCrlUnder(PINNED_WIRE, "", pub)).toBe(false);
});

test("Slice 2: full refund adds order:<id> to revoked + (re)builds a verifying crl", async () => {
  const seedB64 = bytesToBase64(TEST_SEED);
  const { env, store } = fakeEnv(seedB64);
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  const orderId = "33333333-3333-3333-3333-333333333333";

  const res = await handleOrderRefunded(
    { id: orderId, status: "refunded", product_id: "prod_license" },
    env,
  );
  expect(res.status).toBe("revoked");

  expect(JSON.parse(store.get("revoked")!)).toEqual(["order:" + orderId]);
  const wire = store.get("crl")!;
  expect(await verifyCrlUnder(wire, "mnema-crl-v1:", pub)).toBe(true);
  expect(crlRevokedIds(wire)).toContain("order:" + orderId);
});

test("Slice 2: partial refund is a no-op (revoked set unchanged)", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  const res = await handleOrderRefunded(
    { id: "ord_partial", status: "partially_refunded", product_id: "prod_license" },
    env,
  );
  expect(res.status).toBe("not-full-refund");
  expect(store.has("revoked")).toBe(false);
  expect(store.has("crl")).toBe(false);
});

test("Slice 2: unknown-product refund is a no-op", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  const res = await handleOrderRefunded(
    { id: "ord_unk", status: "refunded", product_id: "prod_other" },
    env,
  );
  expect(res.status).toBe("unknown-product");
  expect(store.has("revoked")).toBe(false);
});

test("Slice 2: CRL rebuild is monotonic even with a clock stuck in the past", async () => {
  const past = 1_000; // absurdly stale clock
  const a = await buildAndSignCrl(["order:a"], 0, past, TEST_SEED);
  const b = await buildAndSignCrl(["order:a"], crlIssuedAt(a), past, TEST_SEED);
  const c = await buildAndSignCrl(["order:a"], crlIssuedAt(b), past, TEST_SEED);
  expect(crlIssuedAt(b)).toBeGreaterThan(crlIssuedAt(a));
  expect(crlIssuedAt(c)).toBeGreaterThan(crlIssuedAt(b));
});

test("Slice 2: GET /revocations.json serves the signed doc, lazy-rebuilds when crl missing", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  // Seed `revoked` directly (simulating a manual comp-key revocation) with no crl.
  store.set("revoked", JSON.stringify(["order:44444444-4444-4444-4444-444444444444"]));

  const req = new Request("https://f.example/revocations.json", { method: "GET" });
  const res = await defaultExport.fetch(req, env);
  expect(res.status).toBe(200);
  expect(res.headers.get("content-type")).toBe("text/plain; charset=utf-8");

  const wire = await res.text();
  expect(await verifyCrlUnder(wire, "mnema-crl-v1:", pub)).toBe(true);
  expect(crlRevokedIds(wire)).toContain("order:44444444-4444-4444-4444-444444444444");
  // Lazily stored.
  expect(store.get("crl")).toBe(wire);
});
