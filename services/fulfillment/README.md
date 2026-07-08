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
2. Act on `type == "order.paid"` with `billing_reason == "purchase"` (mint) and
   on `type == "order.refunded"` with `status == "refunded"` (revoke — see
   Revocation List below). Everything else is ACKed with 200 and ignored.
3. Idempotency on the Polar order `id` via the `IDEMPOTENCY` KV namespace
   (webhooks are at-least-once, retried up to 10×). The order id is recorded
   **after** mint+email succeed, so a failed email leaves it un-recorded and the
   retry re-attempts.
4. Map `product_id`: `POLAR_LICENSE_PRODUCT_ID` and `POLAR_RENEWAL_PRODUCT_ID`
   both mint a fresh `tier="license"` key with
   `update_through = now + UPDATE_WINDOW_DAYS` (default 365). Unknown product →
   200 ACK, no mint.
5. **Renewal ownership gate.** Since a renewal mints the same full `tier="license"`
   grant as the license SKU (just cheaper), a renewal from a non-owner would be a
   full-price bypass. Before minting a renewal, look up the buyer's license orders
   via the Polar API (`POLAR_API_BASE` + `POLAR_ACCESS_TOKEN`); if they own no
   non-refunded license, the renewal is **auto-refunded** and a note email is sent
   instead of a key. Ownership state lives in Polar, not here — Fulfillment keeps
   no ownership record. A lookup/refund API failure 500s so Polar retries (never
   refunds a legit renewal on a blip).
6. Email the key via Resend from a `licenses@` sender.

## License key wire format (must match `license_verify.rs`)

    key = base64(payload_json) + "." + base64(signature)

- `payload_json` = **compact** JSON, exact field order:
  `{email, license_id, tier, issued_at, update_through}`
- `issued_at` / `update_through` are **unix epoch milliseconds** (integers).
- `signature` = Ed25519 over the raw `payload_json` UTF-8 bytes.
- base64 is **standard, with padding** (NOT url-safe) for both halves.
- `license_id` derives from the Polar order id: `order:<order_id>` (deterministic,
  so re-mints reproduce it and a revocation covers every copy). Comp keys use
  `comp:<slug>`.

> The Rust verifier (Slice 3, `crates/app-infra/src/license_verify.rs`) did not
> exist when this was written. `src/mint.ts` carries a `TODO: reconcile` — once
> Slice 3 lands, diff its parser against `mintKey` (base64 variant, field
> order/names, ms-vs-seconds, signed bytes).

## Revocation List (CRL) wire format (must match the app verifier)

A full refund (`order.refunded` with `status == "refunded"`) revokes that
order's license. The worker keeps two KV keys in `IDEMPOTENCY`: `revoked` (a
JSON array of revoked license ids, the source of truth — a leaked comp key is
killed with one `wrangler kv key put revoked ...`) and `crl` (the current signed
document, rebuilt from `revoked` whenever the set changes, and lazily on GET if
it has drifted). Partial refunds (`partially_refunded`) and unknown products are
no-ops.

    wire = base64(payload_json) + "." + base64(signature)

- `payload_json` = **compact** JSON, exact field order:
  `{"schema":1,"issued_at":<unix ms int>,"revoked_license_ids":[<string>...]}`
- `revoked_license_ids` is sorted for stable output; ids are `order:<order_id>`
  (or `comp:<slug>` for comp keys).
- `issued_at` is **monotonic**: `max(now, prev.issued_at + 1)`, so it strictly
  increases across redeploys even with a clock stuck in the past (the app
  accepts only strictly-fresher documents, blocking rollback).
- `signature` = Ed25519 over the UTF-8 bytes of `"mnema-crl-v1:" + payload_json`
  — the **domain-separation** prefix means a CRL can never replay as a license
  key (raw payload, no prefix) or vice versa. Same seed as license keys.
- base64 is **standard, with padding** (NOT url-safe) for both halves.

Served at `GET /revocations.json`, body = the wire string,
`content-type: text/plain; charset=utf-8` (it is not JSON), status 200. The GET
is anonymous — no identifier sent — and a stale/missing document always means
the license stands (ADR 0052).

**Re-mint rule:** a lost-key re-mint of an existing order reproduces the same
`order:<id>` license id (so a revocation still covers it) and dates
`update_through` from the **original order date**, never `now` — re-mints must
not extend the buyer's Update Window.

## Signing keys

Each env has its **own** Ed25519 keypair, so a dev-minted key verifies only
against a dev desktop build and a prod-minted key only against a shipped build.
Generate one with `bun scripts/gen-keypair.ts` (prints seed + public key + a Rust
literal); store each seed in the password manager, one per env, never in the repo.

- **prod**: the desktop default. `ED25519_PRIVATE_KEY --env production` is the
  prod seed, matching the hardcoded `PRODUCTION_LICENSE_PUBLIC_KEY` in
  `crates/app-infra/src/license_verify.rs`. Rotate by pasting a new literal +
  shipping a build.
- **dev**: `ED25519_PRIVATE_KEY --env dev` is the dev seed; build the dev desktop
  app with its public key via `MNEMA_LICENSE_PUBLIC_KEY` (base64) — `dev-app.sh`
  auto-loads it from `~/.mnema-licensing-keys/dev_public_key.b64`.

`bake-crl.ts` always verifies against the prod key (it bakes the prod CRL floor).
Full detail: `docs/licensing/ENV.md` → "Signing keys — one keypair per env".
Never hand a dev-minted key to a real buyer.

## Env / secrets

Set with `wrangler secret put <NAME> --env <dev|production>` (never commit values):

| Name | What |
| --- | --- |
| `ED25519_PRIVATE_KEY` | **base64 of the raw 32-byte Ed25519 seed** — the one keypair both envs share (`~/.mnema-licensing-keys/ed25519_private_key.raw.b64`); see "Signing keys" above |
| `POLAR_WEBHOOK_SECRET` | Polar webhook signing secret (`whsec_<base64>`) |
| `RESEND_API_KEY` | Resend API key |

Vars (per-environment `vars` in `wrangler.jsonc`, non-secret):

| Name | What |
| --- | --- |
| `POLAR_LICENSE_PRODUCT_ID` | Polar License SKU product id |
| `POLAR_RENEWAL_PRODUCT_ID` | Polar Renewal SKU product id |
| `UPDATE_WINDOW_DAYS` | default `365` |
| `RESEND_FROM` | optional sender, default `Mnema Licenses <licenses@mnema.app>` |
| `IDEMPOTENCY` | KV namespace binding |

## Environments

`wrangler.jsonc` declares two named environments — `dev` and `production` — with
**no** top-level bindings, so every command must pass `--env dev` or `--env
production` (a bare `wrangler deploy` has no KV/vars). Each env has its **own** KV
namespace, product ids, worker name, and secrets — so dev traffic never touches
prod's idempotency/revoked store. Wrangler does not inherit `vars`/`kv_namespaces`
into a named env, so both are declared in full.

| | Dev (`--env dev`) | Prod (`--env production`) |
| --- | --- | --- |
| worker | `mnema-fulfillment` | `mnema-fulfillment-prod` |
| Polar | **sandbox** product ids | live product ids |
| KV | dev `IDEMPOTENCY` | separate prod `IDEMPOTENCY` |
| signing key | see "Signing keys" below | see "Signing keys" below |

## Develop / test

    bun install
    bun run test        # unit tests (bun's built-in runner, no framework dep)
    bun run typecheck    # tsc --noEmit over src/

## Deploy

Every command carries `--env dev` or `--env production` — same steps per env.

1. `wrangler kv namespace create IDEMPOTENCY --env <dev|production>` → paste the
   returned id into the matching block in `wrangler.jsonc`.
2. Set the three secrets and fill the product-id vars for that env:
   `wrangler secret put ED25519_PRIVATE_KEY --env <dev|production>` (same for
   `POLAR_WEBHOOK_SECRET`, `RESEND_API_KEY`).
3. `bun run deploy:dev` or `bun run deploy:prod`.
4. In Polar, add a webhook endpoint pointed at the deployed Worker URL,
   subscribed to `order.paid`, and copy its signing secret into
   `POLAR_WEBHOOK_SECRET`. Use the Polar **sandbox** for dev, live for prod.
