# Licenses are verified offline (self-signed Ed25519); Polar is Merchant-of-Record only

## Status

Proposed. Amended by [ADR 0052](0052-refunded-licenses-die-via-a-signed-revocation-list.md):
"keys are non-revocable" no longer holds — fully-refunded orders are revoked via a signed
revocation list (staleness never locks).

## Context

[ADR 0044](0044-monetize-as-one-time-purchase-with-paid-update-window.md) commits Mnema to a
one-time License + paid Update Window + 30-day Trial. This ADR records *how* that is verified and
enforced, under a hard constraint: **the desktop app and the customer must never phone home.**
Zero runtime network for licensing is a privacy-brand requirement, not a nice-to-have.

The payment platform is **Polar** (chosen for Merchant-of-Record tax handling, one-time products,
low onboarding friction, and clean webhooks). Research into Polar's built-in **License Keys**
benefit found it architecturally incompatible with our model: it issues opaque UUID keys, offers
**no** bring-your-own-key, and validates **only** via a live `POST` to `api.polar.sh` on every
check. Using it would break the zero-phone-home promise outright.

## Decision

**Verify everything on-device; use Polar strictly as Merchant-of-Record + checkout; do our own
key minting and delivery.**

- **License = self-signed Ed25519 payload** `{email, license_id, tier, issued_at, update_through}`.
  One keypair, generated once. The **public key is hardcoded in the Rust binary**; verification is
  local (`ed25519-dalek`), no network ever. Forgery/field-edit is cryptographically impossible; a
  determined user can still patch the binary — the goal is "keep honest people honest," not perfect
  DRM. `email` stays in the payload for a soft "licensed to you" deterrent and support lookup.
- **Trial = signed record in the OS keychain**, written at **first successful Capture**, with a
  **"max timestamp ever seen"** check to blunt casual clock-rollback. Keychain survives uninstall
  (macOS/Windows), defeating casual reinstall-reset. Deliberately not over-engineered.
- **Update Window enforcement lives in the auto-updater, not a runtime lock.** The signed
  `update_through` date lets the updater decline builds released after the window; a non-renewing
  owner is simply never offered a newer build and keeps their covered build running forever
  (**Perpetual Fallback**). The one runtime check is the **fresh-install-after-lapse edge**: on a
  clean machine the download site serves the latest (too-new) build, so the app compares its own
  build date against `update_through` at launch and, for an out-of-window owner, directs them to
  the newest build their License covers (kept downloadable indefinitely) or to renew — it **never**
  hard-locks and **never** degrades already-recorded history.
- **The licensing gate runs in the deferred-startup thread, after the window opens, and emits an
  event** for the frontend to show a modal. It does **not** gate synchronously at launch (respects
  the fast-window-open invariant).
- **Fulfillment is an automated serverless step** (e.g. Cloudflare Worker / Vercel function): it
  catches Polar's `order.paid` webhook (Standard-Webhooks HMAC-SHA256 — a Rust/any verifier must
  base64-decode the signing secret first), mints the Ed25519 key with the **private key held as a
  cloud secret**, and emails it via a mail provider (Resend). Polar cannot carry a per-customer
  minted key, so we send the delivery email. The handler is idempotent on the Polar order id
  (webhooks are at-least-once, retried up to 10×).
- **Renewal = a separate one-time Polar SKU.** Its `order.paid` mints a fresh key with
  `update_through = renewal_date + 1 year`; the owner pastes it into the app, which keeps
  whichever key has the latest `update_through`. Fulfillment keeps no prior-license record of
  its own, but a renewal mints the same full `tier="license"` grant as the license SKU (just
  cheaper), so honoring one for a non-owner is a full-price bypass. Before minting a renewal,
  Fulfillment queries the **Polar API** for the buyer's license orders (ownership truth lives
  in Polar, the MoR — not a Fulfillment DB); a renewal from a non-owner is **auto-refunded**
  with an explanatory email rather than minted. A lookup/refund API failure 500s so Polar
  retries — it never refunds a legitimate renewal on a transient blip.
- **Keys are non-revocable.** With no runtime phone-home there is no revocation channel; a refunded
  or charged-back buyer keeps a working key. Accepted — refunds are rare, Polar handles the money
  side, and this matches the "keep honest people honest" posture.

## Considered options

- **Polar's built-in License Keys benefit.** Rejected: online-validation-only (a live call to
  `api.polar.sh` per check) breaks zero-phone-home; opaque UUID keys with no bring-your-own-key
  means our signed payload cannot be expressed in it at all.
- **Keygen.sh / `rust-license-key`.** Deferred: Keygen does offer offline Ed25519 license files and
  a Tauri plugin, but its value is managed seat-limits/revocation/self-host — none of which a
  one-time + trial model needs. Revisit only if managed revocation or seat limits ever become
  requirements.
- **Pre-generated key pool uploaded to the platform (no fulfillment server at all).** Rejected:
  pool keys can't embed the buyer's email and make renewals (which extend a specific owner's
  window) awkward. We chose per-purchase signing, which requires the serverless Fulfillment step.
- **Manual fulfillment at launch (private key never leaves the build machine).** Rejected for
  launch: making a paying customer wait for a human to email a key is a poor first impression. The
  app verifies whatever signed key arrives, so this is reversible — but automated serverless ships
  from day one.
- **Synchronous launch-time gate.** Rejected: violates the fast-window-open invariant. The gate is
  deferred and event-driven.

## Consequences

- **"No server anywhere" is scoped to the app and the user, not the seller.** A minimal seller-side
  Fulfillment endpoint exists and holds the Ed25519 **private key** as a cloud secret plus a mail
  provider account. The runtime privacy promise (app/user never phone home) is intact.
- **Private-key compromise** would let an attacker mint valid keys. Acceptable under the
  "keep honest people honest" threat model (binary patching is already possible); rotate the
  keypair + ship a new hardcoded public key if it ever leaks.
- **Old covered builds must remain downloadable indefinitely** so out-of-window owners can install
  the newest build they are entitled to.
- Resolved domain language (Capture, License, Update Window, Perpetual Fallback, Trial,
  Read-Only Mode, Fulfillment, Renewal) lives in
  [`docs/licensing/CONTEXT.md`](../licensing/CONTEXT.md). Read-Only Mode and the transient-liveness
  **Capture Suspension** (ADR 0021, 0040) are distinct states and must not share a code path.
