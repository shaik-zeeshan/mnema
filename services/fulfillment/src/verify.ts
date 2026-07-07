import { base64ToBytes, bytesToBase64 } from "./util";

// Standard Webhooks (https://www.standardwebhooks.com) signature verification,
// as used by Polar. Signed content is `${webhook-id}.${webhook-timestamp}.${body}`.
//
// GOTCHA: Polar deviates from the spec. The canonical scheme base64-decodes the
// secret (stripping the `whsec_` prefix) for use as the HMAC key, but Polar signs
// with the RAW secret STRING — prefix included — as the key (verified against a
// live delivery, 2026-07-07). We accept EITHER so we're correct regardless of how
// a given Polar endpoint signs.

const TOLERANCE_SECONDS = 5 * 60;

function decodeSecret(secret: string): Uint8Array {
  const raw = secret.startsWith("whsec_") ? secret.slice("whsec_".length) : secret;
  return base64ToBytes(raw);
}

// Constant-time string compare (both are base64 of the same-length HMAC digest).
function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

async function hmacBase64(key: Uint8Array, content: string): Promise<string> {
  const cryptoKey = await crypto.subtle.importKey(
    "raw",
    key,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", cryptoKey, new TextEncoder().encode(content));
  return bytesToBase64(new Uint8Array(sig));
}

export async function verifyWebhook(
  rawBody: string,
  headers: Headers,
  secret: string,
): Promise<boolean> {
  const id = headers.get("webhook-id");
  const timestamp = headers.get("webhook-timestamp");
  const signatureHeader = headers.get("webhook-signature");
  if (!id || !timestamp || !signatureHeader) return false;

  const ts = Number(timestamp);
  const now = Math.floor(Date.now() / 1000);
  if (!Number.isFinite(ts) || Math.abs(now - ts) > TOLERANCE_SECONDS) return false;

  const content = `${id}.${timestamp}.${rawBody}`;
  // Polar's raw-string key (what we observed live) and the Standard-Webhooks
  // base64-decoded key — accept a match against either.
  const expected = await Promise.all([
    hmacBase64(new TextEncoder().encode(secret), content),
    hmacBase64(decodeSecret(secret), content),
  ]);

  // Header is space-delimited `v1,<base64sig>` entries (versioned, may be several).
  for (const part of signatureHeader.split(" ")) {
    const comma = part.indexOf(",");
    if (comma < 0) continue;
    const version = part.slice(0, comma);
    const sig = part.slice(comma + 1);
    if (version === "v1" && sig && expected.some((e) => timingSafeEqual(sig, e))) return true;
  }
  return false;
}
