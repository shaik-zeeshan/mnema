import { verifyWebhook } from "./verify";
import { mintKey } from "./mint";
import { base64ToBytes } from "./util";
import { licenseEmail, renewalWithoutLicenseEmail } from "./email";
import { buildAndSignCrl, crlIssuedAt, crlRevokedIds } from "./crl";

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
  IDEMPOTENCY: KVNamespace;
}

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
  customer?: { id?: string; email?: string };
}

// KV keys (in the IDEMPOTENCY namespace, alongside order-id idempotency keys):
//   revoked = JSON array of revoked license ids (source of truth)
//   crl     = the current signed CRL wire string, rebuilt from `revoked`
const REVOKED_KEY = "revoked";
const CRL_KEY = "crl";

async function readRevokedSet(env: Env): Promise<string[]> {
  const raw = await env.IDEMPOTENCY.get(REVOKED_KEY);
  if (!raw) return [];
  try {
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? arr : [];
  } catch {
    return [];
  }
}

// Rebuild + re-sign the CRL from `revokedIds`, monotonic against any prev crl.
async function rebuildCrl(env: Env, revokedIds: string[], now: number): Promise<string> {
  const prevWire = await env.IDEMPOTENCY.get(CRL_KEY);
  const prevIssuedAt = prevWire ? crlIssuedAt(prevWire) : 0;
  const seed = base64ToBytes(env.ED25519_PRIVATE_KEY);
  const wire = await buildAndSignCrl(revokedIds, prevIssuedAt, now, seed);
  await env.IDEMPOTENCY.put(CRL_KEY, wire);
  return wire;
}

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
  const revoked = await readRevokedSet(env);
  if (revoked.includes(licenseId)) return { status: "already-revoked", license_id: licenseId };

  revoked.push(licenseId);
  await env.IDEMPOTENCY.put(REVOKED_KEY, JSON.stringify(revoked));
  await rebuildCrl(env, revoked, now);

  return { status: "revoked", license_id: licenseId };
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    // Public, anonymous CRL endpoint. Serve the signed doc; lazily rebuild if
    // `crl` is missing or its id set has drifted from the `revoked` source.
    if (req.method === "GET" && new URL(req.url).pathname === "/revocations.json") {
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
