import { verifyWebhook } from "./verify";
import { mintKey } from "./mint";
import { base64ToBytes } from "./util";
import { licenseEmail } from "./email";

export interface Env {
  ED25519_PRIVATE_KEY: string; // base64 of the raw 32-byte Ed25519 seed
  POLAR_WEBHOOK_SECRET: string; // Standard-Webhooks signing secret (whsec_<base64>)
  RESEND_API_KEY: string;
  RESEND_FROM?: string; // e.g. "Mnema Licenses <licenses@mnema.app>"
  POLAR_LICENSE_PRODUCT_ID: string;
  POLAR_RENEWAL_PRODUCT_ID: string;
  UPDATE_WINDOW_DAYS?: string; // default 365
  IDEMPOTENCY: KVNamespace;
}

const DAY_MS = 24 * 60 * 60 * 1000;
const IDEMPOTENCY_TTL_SECONDS = 30 * DAY_MS / 1000; // 30 days — well past Polar's retry window

// Minimal shape of the Polar order.paid payload we depend on.
interface PolarOrder {
  id?: string;
  billing_reason?: string;
  product_id?: string;
  customer?: { email?: string };
}

async function sendLicenseEmail(env: Env, to: string, key: string): Promise<void> {
  const { subject, text, html } = licenseEmail(key);
  const res = await fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${env.RESEND_API_KEY}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      from: env.RESEND_FROM ?? "Mnema Licenses <licenses@mnema.app>",
      to: [to],
      subject,
      text,
      html,
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

export interface FulfillResult {
  status: "minted" | "duplicate" | "unknown-product";
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

  const email = order.customer?.email;
  if (!email) throw new Error("missing customer email");

  // Both license and renewal mint a fresh tier="license" key. Renewal is stateless:
  // just a new key with a future window (no prior-license lookup).
  const days = Number(env.UPDATE_WINDOW_DAYS ?? "365") || 365;
  const issuedAt = now;
  const updateThrough = now + days * DAY_MS;

  const seed = base64ToBytes(env.ED25519_PRIVATE_KEY);
  const key = await mintKey(
    {
      email,
      license_id: crypto.randomUUID(),
      tier: "license",
      issued_at: issuedAt,
      update_through: updateThrough,
    },
    seed,
  );

  await sendLicenseEmail(env, email, key);
  await env.IDEMPOTENCY.put(orderId, new Date(now).toISOString(), {
    expirationTtl: IDEMPOTENCY_TTL_SECONDS,
  });

  return { status: "minted", key };
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
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

    // Only mint on a genuine purchase. Ignore refunds and everything else (200 ACK).
    if (event.type !== "order.paid") return new Response("ignored", { status: 200 });
    const order = event.data ?? {};
    if (order.billing_reason !== "purchase") return new Response("ignored", { status: 200 });

    try {
      const result = await handleOrderPaid(order, env);
      return new Response(result.status, { status: 200 });
    } catch (e) {
      // Mint/email failed — 500 so Polar retries. Idempotency was NOT recorded.
      return new Response(`error: ${(e as Error).message}`, { status: 500 });
    }
  },
};
