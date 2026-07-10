import * as ed from "@noble/ed25519";
import { verifyWebhook } from "./verify";
import { mintKey } from "./mint";
import { base64ToBytes } from "./util";
import { licenseEmail, renewalWithoutLicenseEmail } from "./email";
import { buildAndSignCrl, crlIssuedAt, crlRevokedIds } from "./crl";
import { signReceipt } from "./receipt";

export interface Env {
  ED25519_PRIVATE_KEY: string; // base64 of the raw 32-byte Ed25519 seed
  POLAR_WEBHOOK_SECRET: string; // Standard-Webhooks signing secret (whsec_<base64>)
  RESEND_API_KEY: string;
  RESEND_FROM?: string; // e.g. "Mnema Licenses <licenses@mnema.app>"
  POLAR_LICENSE_PRODUCT_ID: string;
  POLAR_RENEWAL_PRODUCT_ID: string;
  POLAR_API_BASE: string; // https://api.polar.sh (prod) | https://sandbox-api.polar.sh (dev)
  POLAR_ACCESS_TOKEN: string; // secret — orders:read + refunds:write, used for the renewal ownership gate
  UPDATE_WINDOW_DAYS?: string; // default 365
  BUY_URL?: string; // public checkout/pricing link surfaced on an over-cap 409
  IDEMPOTENCY: KVNamespace;
}

const BUY_URL_DEFAULT = "https://mnema.day/#pricing";

const DAY_MS = 24 * 60 * 60 * 1000;
const IDEMPOTENCY_TTL_SECONDS = 30 * DAY_MS / 1000; // 30 days — well past Polar's retry window

// Minimal shape of the Polar order.paid / order.refunded payload we depend on.
interface PolarOrder {
  id?: string;
  billing_reason?: string;
  product_id?: string;
  status?: string;
  customer_id?: string;
  // Net cents still refundable (before tax). Polar refunds the tax proportionally
  // on top of the `amount` we pass, so this — NOT total_amount — is the refund amount.
  refundable_amount?: number;
  customer?: { id?: string; email?: string; name?: string };
}

// KV keys (in the IDEMPOTENCY namespace, alongside order-id idempotency keys):
//   revoked:<license_id> = "1"  — ONE key per revoked license id (source of truth)
//   crl                  = the current signed CRL wire string, rebuilt from the set
//
// The revoked set is stored as one key per id, NOT a single JSON array, on purpose:
// KV has no compare-and-swap, so a read-modify-write of a shared array key silently
// loses updates when two different orders are refunded concurrently (both read the
// same array, each pushes its own id, the last put clobbers the other — and since
// both handlers 200, Polar never retries, so the lost revocation is gone forever).
// A per-id key is a blind single-key write: two different ids touch two different
// keys and cannot clobber each other. Re-revoking the same id just rewrites "1".
const REVOKED_PREFIX = "revoked:";
const CRL_KEY = "crl";

async function readRevokedSet(env: Env): Promise<string[]> {
  const ids: string[] = [];
  let cursor: string | undefined;
  do {
    const page = await env.IDEMPOTENCY.list({ prefix: REVOKED_PREFIX, cursor });
    for (const k of page.keys) ids.push(k.name.slice(REVOKED_PREFIX.length));
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return ids;
}

// Rebuild + re-sign the CRL from `revokedIds`, monotonic against any prev crl.
// Revocations are append-only (nothing ever un-revokes), so the id set is UNIONED
// with the previous CRL's ids: a stale, eventually-consistent `list` that returns
// a SUBSET of what the durable CRL already covers must never drop a revocation and
// re-sign it away as "newer" — that would silently un-revoke a refunded license.
async function rebuildCrl(env: Env, revokedIds: string[], now: number): Promise<string> {
  const prevWire = await env.IDEMPOTENCY.get(CRL_KEY);
  const prevIssuedAt = prevWire ? crlIssuedAt(prevWire) : 0;
  const merged = [...new Set([...(prevWire ? crlRevokedIds(prevWire) : []), ...revokedIds])];
  const seed = base64ToBytes(env.ED25519_PRIVATE_KEY);
  const wire = await buildAndSignCrl(merged, prevIssuedAt, now, seed);
  await env.IDEMPOTENCY.put(CRL_KEY, wire);
  return wire;
}

// Activation KV keys (in the IDEMPOTENCY namespace; ADR 0053):
//   activation:<license_id>:<machine_hash> = "1"  — one blind key per machine
//   activation-meta:<license_id>           = JSON { last_reset_at, lifetime_count }
//
// Same lost-update reasoning as revoked:* — one blind key per activated machine so
// two machines activating concurrently touch two different keys and cannot clobber
// each other (KV has no compare-and-swap). The meta key IS a read-modify-write, but
// it only carries telemetry/rate-limit fields, not entitlement, so a racy count is
// acceptable. The trailing ":" on the slot prefix keeps "order:11" from matching
// "order:111" when listing a license's slots.
const ACTIVATION_PREFIX = "activation:";
const ACTIVATION_META_PREFIX = "activation-meta:";
const ACTIVATION_CAP = 3;
const RESET_COOLDOWN_MS = 30 * DAY_MS;
const RECEIPT_SCHEMA = 1;

interface ActivationMeta {
  last_reset_at: number; // ms; 0 = never reset
  lifetime_count: number; // total slots ever granted — NEVER decremented (leak telemetry)
}

async function readActivationMeta(env: Env, licenseId: string): Promise<ActivationMeta> {
  const raw = await env.IDEMPOTENCY.get(ACTIVATION_META_PREFIX + licenseId);
  if (!raw) return { last_reset_at: 0, lifetime_count: 0 };
  return JSON.parse(raw) as ActivationMeta;
}

// Every slot key for a license: `activation:<license_id>:*`.
async function listActivationKeys(env: Env, licenseId: string): Promise<string[]> {
  const prefix = ACTIVATION_PREFIX + licenseId + ":";
  const out: string[] = [];
  let cursor: string | undefined;
  do {
    const page = await env.IDEMPOTENCY.list({ prefix, cursor });
    for (const k of page.keys) out.push(k.name);
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return out;
}

function jsonResponse(obj: unknown, status: number): Response {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

// POST /activate — bind a machine to a license, mint a signed Activation Receipt.
async function handleActivate(req: Request, env: Env, now: number = Date.now()): Promise<Response> {
  let body: { schema?: unknown; license_id?: unknown; machine_hash?: unknown };
  try {
    body = await req.json();
  } catch {
    return new Response("bad json", { status: 400 });
  }
  const licenseId = body.license_id;
  const machineHash = body.machine_hash;
  if (
    body.schema !== RECEIPT_SCHEMA ||
    typeof licenseId !== "string" ||
    !licenseId.trim() ||
    typeof machineHash !== "string" ||
    !machineHash.trim()
  ) {
    return new Response("bad request", { status: 400 });
  }

  // Revoked id → 403, checked FIRST (a revoked license activates nowhere).
  if (await env.IDEMPOTENCY.get(REVOKED_PREFIX + licenseId)) {
    return jsonResponse({ code: "revoked" }, 403);
  }

  const seed = base64ToBytes(env.ED25519_PRIVATE_KEY);
  const slotKey = ACTIVATION_PREFIX + licenseId + ":" + machineHash;

  // Known machine → idempotent: re-sign a fresh receipt, touch nothing.
  if (await env.IDEMPOTENCY.get(slotKey)) {
    return jsonResponse({ receipt: await signReceipt(licenseId, machineHash, now, seed) }, 200);
  }

  // New machine — enforce the 3-device cap.
  if ((await listActivationKeys(env, licenseId)).length >= ACTIVATION_CAP) {
    return jsonResponse(
      {
        code: "over_cap",
        reset_url: new URL("/reset", req.url).toString(),
        buy_url: env.BUY_URL ?? BUY_URL_DEFAULT,
      },
      409,
    );
  }

  // Grant the slot (blind write), then RE-VERIFY the cap. The pre-check above is a
  // check-then-act: two new machines can both read count < CAP and both write,
  // overshooting the cap (KV has no compare-and-swap). Claiming first, then
  // re-listing, lets each racer observe the other's just-written slot and withdraw
  // its own — only the newcomer ever deletes its own slot, so an established
  // activation is never evicted. This bounds the cap within one datacenter/isolate
  // (the common retry case). ponytail: residual cross-POP overshoot within KV's
  // ~60s list-staleness window remains — true serialization needs a Durable Object.
  await env.IDEMPOTENCY.put(slotKey, "1");
  if ((await listActivationKeys(env, licenseId)).length > ACTIVATION_CAP) {
    await env.IDEMPOTENCY.delete(slotKey);
    return jsonResponse(
      {
        code: "over_cap",
        reset_url: new URL("/reset", req.url).toString(),
        buy_url: env.BUY_URL ?? BUY_URL_DEFAULT,
      },
      409,
    );
  }
  const meta = await readActivationMeta(env, licenseId);
  meta.lifetime_count += 1;
  await env.IDEMPOTENCY.put(ACTIVATION_META_PREFIX + licenseId, JSON.stringify(meta));

  return jsonResponse({ receipt: await signReceipt(licenseId, machineHash, now, seed) }, 200);
}

// POST /reset — free a license's activation slots, gated once per 30 days.
async function handleReset(req: Request, env: Env, now: number = Date.now()): Promise<Response> {
  let body: { key?: unknown };
  try {
    body = await req.json();
  } catch {
    return new Response("bad json", { status: 400 });
  }
  const key = body.key;
  if (typeof key !== "string" || !key.trim()) return new Response("bad request", { status: 400 });

  // Verify the pasted license key: Ed25519 over the RAW payload bytes (no domain
  // prefix — see mint.ts). A bad signature → 403. Public key derived from the seed.
  const seed = base64ToBytes(env.ED25519_PRIVATE_KEY);
  const pub = await ed.getPublicKeyAsync(seed);
  let licenseId: string;
  try {
    const [payloadB64, sigB64] = key.trim().split(".");
    const payloadBytes = base64ToBytes(payloadB64);
    if (!(await ed.verifyAsync(base64ToBytes(sigB64), payloadBytes, pub))) {
      return new Response("bad signature", { status: 403 });
    }
    licenseId = (JSON.parse(new TextDecoder().decode(payloadBytes)) as { license_id?: string })
      .license_id as string;
    if (typeof licenseId !== "string" || !licenseId) {
      return new Response("bad signature", { status: 403 });
    }
  } catch {
    return new Response("bad signature", { status: 403 });
  }

  const meta = await readActivationMeta(env, licenseId);
  const sinceReset = now - meta.last_reset_at;
  if (meta.last_reset_at > 0 && sinceReset < RESET_COOLDOWN_MS) {
    return jsonResponse({ retry_after_ms: RESET_COOLDOWN_MS - sinceReset }, 429);
  }

  // Free every slot; KEEP lifetime_count (leak telemetry survives resets).
  for (const k of await listActivationKeys(env, licenseId)) await env.IDEMPOTENCY.delete(k);
  meta.last_reset_at = now;
  await env.IDEMPOTENCY.put(ACTIVATION_META_PREFIX + licenseId, JSON.stringify(meta));
  return jsonResponse({ ok: true }, 200);
}

// Minimal same-origin form for a buyer to paste their key and reset activations.
const RESET_FORM_HTML = `<!doctype html>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Reset Mnema activations</title>
<style>
  body { font: 15px/1.5 system-ui, sans-serif; max-width: 40rem; margin: 4rem auto; padding: 0 1rem; }
  textarea { width: 100%; min-height: 6rem; font-family: ui-monospace, monospace; }
  button { margin-top: 0.75rem; padding: 0.5rem 1rem; }
  pre { background: #f4f4f4; padding: 0.75rem; overflow-x: auto; white-space: pre-wrap; }
</style>
<h1>Reset Mnema activations</h1>
<p>Paste your full license key to free every activated machine (allowed once every 30 days).</p>
<form id="f">
  <textarea id="key" placeholder="base64payload.base64signature" required></textarea>
  <button type="submit">Reset activations</button>
</form>
<pre id="out" hidden></pre>
<script>
  const out = document.getElementById("out");
  document.getElementById("f").addEventListener("submit", async (e) => {
    e.preventDefault();
    out.hidden = false;
    out.textContent = "Working…";
    try {
      const res = await fetch("/reset", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ key: document.getElementById("key").value.trim() }),
      });
      const text = await res.text();
      out.textContent = res.status + " " + text;
    } catch (err) {
      out.textContent = "request failed: " + err;
    }
  });
</script>`;

async function sendEmail(
  env: Env,
  to: string,
  msg: { subject: string; text: string; html: string },
): Promise<void> {
  const res = await fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${env.RESEND_API_KEY}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      from: env.RESEND_FROM ?? "Mnema Licenses <licenses@mnema.app>",
      to: [to],
      subject: msg.subject,
      text: msg.text,
      html: msg.html,
    }),
  });
  if (!res.ok) {
    // Throw so the caller returns 500 and Polar retries. Idempotency is recorded
    // only after this succeeds, so a retry re-attempts the mint+email. Include
    // Resend's body — it names the cause (unverified domain, test-mode, etc.).
    const detail = await res.text().catch(() => "");
    throw new Error(`resend failed: ${res.status} ${detail}`);
  }
}

// Does this customer already own a (non-refunded) license? Source of truth is
// Polar — Fulfillment keeps no ownership record of its own. A renewal only
// extends an existing owner's window; minting one for a non-owner would hand out
// a full tier="license" grant for the cheaper renewal price.
async function customerOwnsLicense(order: PolarOrder, env: Env): Promise<boolean> {
  const customerId = order.customer_id ?? order.customer?.id;
  if (!customerId) throw new Error("missing customer id on renewal order");
  const url =
    `${env.POLAR_API_BASE}/v1/orders/?customer_id=${encodeURIComponent(customerId)}` +
    `&product_id=${encodeURIComponent(env.POLAR_LICENSE_PRODUCT_ID)}&limit=10`;
  const res = await fetch(url, {
    headers: { Authorization: `Bearer ${env.POLAR_ACCESS_TOKEN}` },
  });
  if (!res.ok) {
    // Transient/auth failure — throw so the webhook 500s and Polar retries,
    // rather than wrongly refunding a legitimate renewal on a lookup blip.
    const detail = await res.text().catch(() => "");
    throw new Error(`polar orders lookup failed: ${res.status} ${detail}`);
  }
  const body = (await res.json()) as { items?: Array<{ status?: string }> };
  // Ownership = a genuinely-paid license order. `partially_refunded` still owns;
  // `refunded` (revoked), `draft`, `void`, and `pending` do not.
  return (body.items ?? []).some(
    (o) => o.status === "paid" || o.status === "partially_refunded",
  );
}

// Full-refund a renewal bought by a non-owner, and email them why. The Polar
// refund also fires an `order.refunded` webhook — that lands on the revocation
// path with a `license_id` we never minted, so it's an inert phantom CRL entry.
async function refundRenewalWithoutLicense(order: PolarOrder, env: Env): Promise<void> {
  const amount = order.refundable_amount;
  if (amount === undefined) throw new Error("missing refundable_amount on renewal order");
  // amount === 0 means it's already fully refunded (a retry after a prior success);
  // skip the API call and fall through to the note. Polar refunds tax on top of `amount`.
  if (amount > 0) {
    const res = await fetch(`${env.POLAR_API_BASE}/v1/refunds/`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${env.POLAR_ACCESS_TOKEN}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        order_id: order.id,
        reason: "other",
        amount, // net, pre-tax — Polar refunds the sales tax proportionally
        comment:
          "Renewal purchased without an existing Mnema license — auto-refunded; buyer directed to buy the license first.",
      }),
    });
    if (!res.ok) {
      const detail = await res.text().catch(() => "");
      // A retry after a prior successful refund exhausts the refundable amount → 422.
      // Treat that as already-done rather than throwing into an endless retry loop.
      const alreadyRefunded = res.status === 422 && detail.includes("refundable");
      if (!alreadyRefunded) throw new Error(`polar refund failed: ${res.status} ${detail}`);
    }
  }
  // Courtesy note — best-effort. A Resend blip must not un-record the refund and
  // trigger a re-refund on the webhook retry.
  const email = order.customer?.email;
  if (email) {
    try {
      await sendEmail(env, email, renewalWithoutLicenseEmail());
    } catch (e) {
      console.error(`renewal-refund note email failed: ${(e as Error).message}`);
    }
  }
}

export interface FulfillResult {
  status: "minted" | "duplicate" | "unknown-product" | "refunded-no-license";
  key?: string;
}

// Assumes billing_reason == "purchase" already filtered by the caller.
export async function handleOrderPaid(
  order: PolarOrder,
  env: Env,
  now: number = Date.now(),
): Promise<FulfillResult> {
  const orderId = order.id;
  if (!orderId) throw new Error("missing order id");

  // Idempotency: at-least-once delivery (retried up to 10x). Recorded LAST.
  if (await env.IDEMPOTENCY.get(orderId)) return { status: "duplicate" };

  const productId = order.product_id;
  const isLicense = productId === env.POLAR_LICENSE_PRODUCT_ID;
  const isRenewal = productId === env.POLAR_RENEWAL_PRODUCT_ID;
  if (!isLicense && !isRenewal) return { status: "unknown-product" }; // ACK, no mint

  // A renewal only extends an existing owner's window. Bought by a non-owner it's
  // a full-price bypass (renewal < license, both mint tier="license") — refund it
  // and explain. Terminal, so record idempotency to prevent a retry double-refund.
  if (isRenewal && !(await customerOwnsLicense(order, env))) {
    await refundRenewalWithoutLicense(order, env);
    await env.IDEMPOTENCY.put(orderId, new Date(now).toISOString(), {
      expirationTtl: IDEMPOTENCY_TTL_SECONDS,
    });
    return { status: "refunded-no-license" };
  }

  const email = order.customer?.email;
  if (!email) throw new Error("missing customer email");

  // License, or a renewal from a verified owner: mint a fresh tier="license" key
  // with a window starting now. Renewal is otherwise stateless — no prior-key lookup.
  const days = Number(env.UPDATE_WINDOW_DAYS ?? "365") || 365;
  const issuedAt = now;
  const updateThrough = now + days * DAY_MS;

  const seed = base64ToBytes(env.ED25519_PRIVATE_KEY);
  const key = await mintKey(
    {
      email,
      license_id: "order:" + orderId,
      tier: "license",
      issued_at: issuedAt,
      update_through: updateThrough,
      name: order.customer?.name ?? "",
    },
    seed,
  );

  await sendEmail(env, email, licenseEmail(key));
  await env.IDEMPOTENCY.put(orderId, new Date(now).toISOString(), {
    expirationTtl: IDEMPOTENCY_TTL_SECONDS,
  });

  return { status: "minted", key };
}

export interface RefundResult {
  status: "revoked" | "already-revoked" | "not-full-refund" | "unknown-product";
  license_id?: string;
}

// Revoke a license on a FULL refund. partially_refunded (or any other status) is
// a no-op — goodwill, not an unwound sale. Idempotent by nature (KV set add).
export async function handleOrderRefunded(
  order: PolarOrder,
  env: Env,
  now: number = Date.now(),
): Promise<RefundResult> {
  if (order.status !== "refunded") return { status: "not-full-refund" };

  const orderId = order.id;
  if (!orderId) throw new Error("missing order id");

  const productId = order.product_id;
  const isLicense = productId === env.POLAR_LICENSE_PRODUCT_ID;
  const isRenewal = productId === env.POLAR_RENEWAL_PRODUCT_ID;
  if (!isLicense && !isRenewal) return { status: "unknown-product" };

  const licenseId = "order:" + orderId;
  const revokedKey = REVOKED_PREFIX + licenseId;
  if (await env.IDEMPOTENCY.get(revokedKey)) return { status: "already-revoked", license_id: licenseId };

  // Blind single-key write — cannot lose a concurrent revocation of a different id.
  await env.IDEMPOTENCY.put(revokedKey, "1");
  // Re-read the full set (now including our id) so the CRL covers every revocation.
  // A concurrent rebuild that raced ahead of our put is self-healed by the GET
  // endpoint's drift-detection, which rebuilds from this durable source of truth.
  await rebuildCrl(env, await readRevokedSet(env), now);

  return { status: "revoked", license_id: licenseId };
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const pathname = new URL(req.url).pathname;

    // Activation endpoints — routed BEFORE the Polar webhook-signature gate below
    // (they are not webhooks, so they must not be rejected as invalid ones).
    if (pathname === "/activate" && req.method === "POST") return handleActivate(req, env);
    if (pathname === "/reset") {
      if (req.method === "GET") {
        return new Response(RESET_FORM_HTML, {
          status: 200,
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      }
      if (req.method === "POST") return handleReset(req, env);
    }

    // Public, anonymous CRL endpoint. Serve the signed doc; lazily rebuild if
    // `crl` is missing or its id set has drifted from the `revoked` source.
    if (req.method === "GET" && pathname === "/revocations.json") {
      const revoked = await readRevokedSet(env);
      let wire = await env.IDEMPOTENCY.get(CRL_KEY);
      const stale =
        !wire ||
        JSON.stringify([...crlRevokedIds(wire)].sort()) !== JSON.stringify([...revoked].sort());
      if (stale) wire = await rebuildCrl(env, revoked, Date.now());
      return new Response(wire!, {
        status: 200,
        headers: { "content-type": "text/plain; charset=utf-8" },
      });
    }

    if (req.method !== "POST") return new Response("method not allowed", { status: 405 });

    const rawBody = await req.text();
    if (!(await verifyWebhook(rawBody, req.headers, env.POLAR_WEBHOOK_SECRET))) {
      return new Response("invalid signature", { status: 401 });
    }

    let event: { type?: string; data?: PolarOrder };
    try {
      event = JSON.parse(rawBody);
    } catch {
      return new Response("bad json", { status: 400 });
    }

    const order = event.data ?? {};
    try {
      if (event.type === "order.paid") {
        // Only mint on a genuine purchase.
        if (order.billing_reason !== "purchase") return new Response("ignored", { status: 200 });
        const result = await handleOrderPaid(order, env);
        return new Response(result.status, { status: 200 });
      }
      if (event.type === "order.refunded") {
        // Revoke only on a full refund (handler no-ops otherwise).
        const result = await handleOrderRefunded(order, env);
        return new Response(result.status, { status: 200 });
      }
      // Everything else — 200 ACK, ignored.
      return new Response("ignored", { status: 200 });
    } catch (e) {
      // Mint/email/revoke failed — 500 so Polar retries. Idempotency was NOT recorded.
      return new Response(`error: ${(e as Error).message}`, { status: 500 });
    }
  },
};
