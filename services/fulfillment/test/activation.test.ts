import { test, expect } from "bun:test";
import * as ed from "@noble/ed25519";
import defaultExport, { type Env } from "../src/index";
import { mintKey } from "../src/mint";
import { signReceipt } from "../src/receipt";
import { bytesToBase64, base64ToBytes } from "../src/util";

// Same test key the Rust crl/license tests pin against.
const TEST_SEED = new Uint8Array(32).fill(7);

// Cross-language vector — the Rust slice pins the identical string.
const PINNED_RECEIPT_WIRE =
  "eyJzY2hlbWEiOjEsImxpY2Vuc2VfaWQiOiJvcmRlcjoxMTExMTExMS0xMTExLTExMTEtMTExMS0xMTExMTExMTExMTEiLCJtYWNoaW5lX2hhc2giOiJlM2IwYzQ0Mjk4ZmMxYzE0OWFmYmY0Yzg5OTZmYjkyNDI3YWU0MWU0NjQ5YjkzNGNhNDk1OTkxYjc4NTJiODU1IiwiYWN0aXZhdGVkX2F0IjoxNzAwMDAwMDAwMDAwfQ==.09bH7xJoQusxo5FP8VF4VF1VGEZWReZJd5LXDw1aXraBgyo0WrXXzXzJZnCw5hUR2C7W1FeaZPLkOdWx5fQ2CQ==";

// KV mock with delete + prefix listing (the activation code lists & deletes slots).
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
      delete: async (k: string) => {
        store.delete(k);
      },
      list: async ({ prefix }: { prefix: string }) => {
        const keys = [...store.keys()].filter((k) => k.startsWith(prefix)).map((name) => ({ name }));
        return { keys, list_complete: true, cursor: undefined };
      },
    },
  } as unknown as Env;
  return { env, store };
}

async function activate(env: Env, licenseId: string, machineHash: string): Promise<Response> {
  return defaultExport.fetch(
    new Request("https://f.example/activate", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ schema: 1, license_id: licenseId, machine_hash: machineHash }),
    }),
    env,
  );
}

async function reset(env: Env, key: string): Promise<Response> {
  return defaultExport.fetch(
    new Request("https://f.example/reset", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key }),
    }),
    env,
  );
}

// Verify a receipt wire under a given domain-separation context.
async function verifyReceiptUnder(wire: string, context: string, pub: Uint8Array): Promise<boolean> {
  const [payloadB64, sigB64] = wire.split(".");
  const signed = new TextEncoder().encode(context + new TextDecoder().decode(base64ToBytes(payloadB64)));
  return ed.verifyAsync(base64ToBytes(sigB64), signed, pub);
}

const LICENSE = "order:aaaa-bbbb";
const HASH_A = "a".repeat(64);
const HASH_B = "b".repeat(64);
const HASH_C = "c".repeat(64);
const HASH_D = "d".repeat(64);

// Mint a valid license key for the reset flow (raw-payload signature, no domain).
async function mintLicense(licenseId: string): Promise<string> {
  return mintKey(
    { email: "b@x.io", license_id: licenseId, tier: "license", issued_at: 1, update_through: 2 },
    TEST_SEED,
  );
}

// --- pinned cross-language vector -------------------------------------------

test("signReceipt reproduces PINNED_RECEIPT_WIRE byte-for-byte", async () => {
  const wire = await signReceipt(
    "order:11111111-1111-1111-1111-111111111111",
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    1700000000000,
    TEST_SEED,
  );
  expect(wire).toBe(PINNED_RECEIPT_WIRE);
});

// --- happy path -------------------------------------------------------------

test("activate: new machine under cap → 200, receipt verifies, KV written, lifetime_count=1", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  const pub = await ed.getPublicKeyAsync(TEST_SEED);

  const res = await activate(env, LICENSE, HASH_A);
  expect(res.status).toBe(200);
  const { receipt } = (await res.json()) as { receipt: string };

  expect(await verifyReceiptUnder(receipt, "mnema-receipt-v1:", pub)).toBe(true);
  expect(store.get(`activation:${LICENSE}:${HASH_A}`)).toBe("1");
  expect(JSON.parse(store.get(`activation-meta:${LICENSE}`)!).lifetime_count).toBe(1);

  const payload = JSON.parse(new TextDecoder().decode(base64ToBytes(receipt.split(".")[0])));
  expect(payload).toMatchObject({ schema: 1, license_id: LICENSE, machine_hash: HASH_A });
});

// --- cross-domain replay guard ----------------------------------------------

test("receipt verifies ONLY under the receipt domain, not license/CRL domains", async () => {
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  expect(await verifyReceiptUnder(PINNED_RECEIPT_WIRE, "mnema-receipt-v1:", pub)).toBe(true);
  expect(await verifyReceiptUnder(PINNED_RECEIPT_WIRE, "", pub)).toBe(false); // license domain
  expect(await verifyReceiptUnder(PINNED_RECEIPT_WIRE, "mnema-crl-v1:", pub)).toBe(false); // CRL domain
});

// --- idempotent re-activation -----------------------------------------------

test("activate: same machine again → 200 fresh receipt, no new slot, lifetime_count unchanged", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  await activate(env, LICENSE, HASH_A);
  const before = store.get(`activation-meta:${LICENSE}`);

  const res = await activate(env, LICENSE, HASH_A);
  expect(res.status).toBe(200);
  expect((await res.json()).receipt).toBeTruthy();

  // Only the one slot; lifetime_count still 1.
  expect([...store.keys()].filter((k) => k.startsWith(`activation:${LICENSE}:`)).length).toBe(1);
  expect(store.get(`activation-meta:${LICENSE}`)).toBe(before);
  expect(JSON.parse(before!).lifetime_count).toBe(1);
});

// --- cap refusal ------------------------------------------------------------

test("activate: 3 distinct machines OK, 4th → 409 over_cap with reset_url + buy_url", async () => {
  const { env } = fakeEnv(bytesToBase64(TEST_SEED));
  for (const h of [HASH_A, HASH_B, HASH_C]) expect((await activate(env, LICENSE, h)).status).toBe(200);

  const res = await activate(env, LICENSE, HASH_D);
  expect(res.status).toBe(409);
  const body = (await res.json()) as { code: string; reset_url: string; buy_url: string };
  expect(body.code).toBe("over_cap");
  expect(body.reset_url).toBe("https://f.example/reset");
  expect(body.buy_url).toBe("https://mnema.day/#pricing");
});

// --- revoked → 403 ----------------------------------------------------------

test("activate: revoked license → 403 revoked (checked before the cap)", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  store.set(`revoked:${LICENSE}`, "1");
  const res = await activate(env, LICENSE, HASH_A);
  expect(res.status).toBe(403);
  expect((await res.json()).code).toBe("revoked");
  expect(store.has(`activation:${LICENSE}:${HASH_A}`)).toBe(false); // no slot granted
});

// --- malformed → 400 --------------------------------------------------------

test("activate: malformed bodies → 400", async () => {
  const { env } = fakeEnv(bytesToBase64(TEST_SEED));
  const post = (body: string) =>
    defaultExport.fetch(
      new Request("https://f.example/activate", { method: "POST", headers: { "content-type": "application/json" }, body }),
      env,
    );
  expect((await post("{not json")).status).toBe(400); // bad JSON
  expect((await post(JSON.stringify({ schema: 2, license_id: LICENSE, machine_hash: HASH_A }))).status).toBe(400); // wrong schema
  expect((await post(JSON.stringify({ schema: 1, license_id: "", machine_hash: HASH_A }))).status).toBe(400); // blank id
  expect((await post(JSON.stringify({ schema: 1, license_id: LICENSE }))).status).toBe(400); // missing hash
});

// --- reset: empties slots, keeps lifetime_count -----------------------------

test("reset: valid key empties slots, keeps lifetime_count, re-activation allowed", async () => {
  const { env, store } = fakeEnv(bytesToBase64(TEST_SEED));
  for (const h of [HASH_A, HASH_B, HASH_C]) await activate(env, LICENSE, h);
  expect(JSON.parse(store.get(`activation-meta:${LICENSE}`)!).lifetime_count).toBe(3);

  const res = await reset(env, await mintLicense(LICENSE));
  expect(res.status).toBe(200);
  expect((await res.json()).ok).toBe(true);

  // Every slot gone, lifetime_count survives, last_reset_at stamped.
  expect([...store.keys()].filter((k) => k.startsWith(`activation:${LICENSE}:`)).length).toBe(0);
  const meta = JSON.parse(store.get(`activation-meta:${LICENSE}`)!);
  expect(meta.lifetime_count).toBe(3);
  expect(meta.last_reset_at).toBeGreaterThan(0);

  // A slot can be re-granted after reset; lifetime_count keeps climbing.
  expect((await activate(env, LICENSE, HASH_D)).status).toBe(200);
  expect(JSON.parse(store.get(`activation-meta:${LICENSE}`)!).lifetime_count).toBe(4);
});

// --- reset: 30-day rate limit -----------------------------------------------

test("reset: a second reset within 30 days → 429 with retry_after_ms", async () => {
  const { env } = fakeEnv(bytesToBase64(TEST_SEED));
  const key = await mintLicense(LICENSE);
  expect((await reset(env, key)).status).toBe(200);

  const res = await reset(env, key);
  expect(res.status).toBe(429);
  const body = (await res.json()) as { retry_after_ms: number };
  expect(body.retry_after_ms).toBeGreaterThan(0);
  expect(body.retry_after_ms).toBeLessThanOrEqual(30 * 24 * 60 * 60 * 1000);
});

// --- reset: bad signature → 403 ---------------------------------------------

test("reset: a key with a broken signature → 403", async () => {
  const { env } = fakeEnv(bytesToBase64(TEST_SEED));
  const key = await mintLicense(LICENSE);
  const [payloadB64, sigB64] = key.split(".");
  // Flip a byte in the signature.
  const badSig = base64ToBytes(sigB64);
  badSig[0] ^= 0xff;
  const tampered = `${payloadB64}.${bytesToBase64(badSig)}`;

  expect((await reset(env, tampered)).status).toBe(403);
  expect((await reset(env, "garbage")).status).toBe(403);
});

// --- GET /reset serves the static form --------------------------------------

test("GET /reset returns a self-contained HTML form", async () => {
  const { env } = fakeEnv(bytesToBase64(TEST_SEED));
  const res = await defaultExport.fetch(new Request("https://f.example/reset", { method: "GET" }), env);
  expect(res.status).toBe(200);
  expect(res.headers.get("content-type")).toBe("text/html; charset=utf-8");
  const html = await res.text();
  expect(html).toContain("<form");
  expect(html).toContain("/reset");
});
