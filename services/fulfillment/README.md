# Mnema Fulfillment

Standalone Cloudflare Worker. Catches Polar's `order.paid` webhook, mints an
Ed25519-signed license key, and emails it via Resend. **Not** part of the Mnema
Bun/Turbo workspace — it has its own deps and never touches `bun run check`.

The desktop app verifies the key entirely offline against a hardcoded public
key (ADR 0045). This service holds the **private** key as a cloud secret.

## Flow

1. Verify the Standard-Webhooks HMAC-SHA256 signature over
   `${webhook-id}.${webhook-timestamp}.${rawBody}`.
   **Gotcha:** Polar deviates from the spec — it signs with the **raw secret
   string** (`whsec_` prefix included) as the HMAC key, *not* the base64-decoded
   key the spec prescribes (verified against a live delivery 2026-07-07).
   `verifyWebhook` accepts **either** scheme, so it's correct regardless.
2. Act only on `type == "order.paid"` with `billing_reason == "purchase"`.
   Refunds and everything else are ACKed with 200 and ignored — keys are
   non-revocable by design.
3. Idempotency on the Polar order `id` via the `IDEMPOTENCY` KV namespace
   (webhooks are at-least-once, retried up to 10×). The order id is recorded
   **after** mint+email succeed, so a failed email leaves it un-recorded and the
   retry re-attempts.
4. Map `product_id`: `POLAR_LICENSE_PRODUCT_ID` and `POLAR_RENEWAL_PRODUCT_ID`
   both mint a fresh `tier="license"` key with
   `update_through = now + UPDATE_WINDOW_DAYS` (default 365, stateless — renewal
   does no prior-license lookup). Unknown product → 200 ACK, no mint.
5. Email the key via Resend from a `licenses@` sender.

## License key wire format (must match `license_verify.rs`)

    key = base64(payload_json) + "." + base64(signature)

- `payload_json` = **compact** JSON, exact field order:
  `{email, license_id, tier, issued_at, update_through}`
- `issued_at` / `update_through` are **unix epoch milliseconds** (integers).
- `signature` = Ed25519 over the raw `payload_json` UTF-8 bytes.
- base64 is **standard, with padding** (NOT url-safe) for both halves.
- `license_id` is a UUID (`crypto.randomUUID()`).

> The Rust verifier (Slice 3, `crates/app-infra/src/license_verify.rs`) did not
> exist when this was written. `src/mint.ts` carries a `TODO: reconcile` — once
> Slice 3 lands, diff its parser against `mintKey` (base64 variant, field
> order/names, ms-vs-seconds, signed bytes).

## Env / secrets

Set with `wrangler secret put <NAME>` (never commit values):

| Name | What |
| --- | --- |
| `ED25519_PRIVATE_KEY` | **base64 of the raw 32-byte Ed25519 seed** (dev keypair: `~/.mnema-licensing-keys/ed25519_private_key.raw.b64`) |
| `POLAR_WEBHOOK_SECRET` | Polar webhook signing secret (`whsec_<base64>`) |
| `RESEND_API_KEY` | Resend API key |

Vars (`[vars]` in `wrangler.toml`, non-secret):

| Name | What |
| --- | --- |
| `POLAR_LICENSE_PRODUCT_ID` | Polar License SKU product id |
| `POLAR_RENEWAL_PRODUCT_ID` | Polar Renewal SKU product id |
| `UPDATE_WINDOW_DAYS` | default `365` |
| `RESEND_FROM` | optional sender, default `Mnema Licenses <licenses@mnema.app>` |
| `IDEMPOTENCY` | KV namespace binding |

## Develop / test

    bun install
    bun run test        # unit tests (bun's built-in runner, no framework dep)
    bun run typecheck    # tsc --noEmit over src/

## Deploy

1. `wrangler kv namespace create IDEMPOTENCY` → paste the returned id into
   `wrangler.toml` under `[[kv_namespaces]]`.
2. Set the three secrets (`wrangler secret put ED25519_PRIVATE_KEY`, etc.) and
   fill the product-id vars in `wrangler.toml`.
3. `bun run deploy` (`wrangler deploy`).
4. In Polar, add a webhook endpoint pointed at the deployed Worker URL,
   subscribed to `order.paid`, and copy its signing secret into
   `POLAR_WEBHOOK_SECRET`. Test against the Polar **sandbox** first.
