# Refunded licenses die via a signed revocation list; staleness never locks

## Status

Proposed. Amends [ADR 0045](0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md)'s
"keys are non-revocable" decision.

## Context

ADR 0045 accepted that a refunded buyer keeps a working key, because zero runtime phone-home
leaves no revocation channel. That leaves a clean abuse path: buy → keep the key → refund inside
the window → enjoy a year of updates on a resellable key. Production peers solve this with an
online license server (Sublime) or TTL'd license files that force periodic re-checkout (Keygen) —
both are per-user runtime validation, which the privacy brand forbids.

## Decision

**Fulfillment publishes a signed revocation list (CRL); the app fetches it anonymously and treats
a listed key like an expired trial. A stale or missing list never locks anything.**

- **Revoke only on full refund.** Polar's `order.refunded` webhook fires for partial refunds too;
  the worker revokes only when the order's `status` is `refunded`. Partial refunds are goodwill,
  not an unwound sale. Chargebacks ride along: Polar (as MoR) resolves disputes through the same
  refund pipeline, so a charged-back order reaches `refunded` status like any other.
- **License ids derive from the Polar order id** (deterministic, e.g. UUIDv5) instead of
  `crypto.randomUUID()`. Fulfillment stays stateless: no mint-time `order → license` record to
  store, lose, or migrate, and a lost-key re-mint of the same order reproduces the same license id
  — a revocation automatically covers every re-minted copy. (Re-mints also date `update_through`
  from the *original* order, never `now`.) The one exception: **comp keys** (gifted licenses with
  no order behind them) carry seller-chosen ids (`comp:<slug>`), minted by the same local script
  as re-mints, hand-revocable by id.
- **The CRL is one signed document** (`{schema, issued_at, revoked_license_ids[]}`) served by the
  fulfillment worker at a stable GET route on a seller-owned domain (the URL is baked into
  shipped binaries — a `workers.dev` URL would be unmigratable). It is signed with the same Ed25519
  key but **domain-separated** (`"mnema-crl-v1:" + payload`) so a CRL can never be replayed as a
  license or vice versa. `issued_at` is monotonic (`max(now, prev + 1)` on regenerate); the app
  accepts only fresher documents than its cache, blocking rollback to a shorter list. The worker
  **rebuilds the signed document from the revoked-set in KV** whenever the set changes — so a
  manual revocation (a leaked comp key) is one `wrangler kv` write, no admin endpoint.
- **The app fetches anonymously** — a GET of a public static file, no license id or identifier
  sent, no per-user record server-side. Cadence: piggybacked on the auto-updater check plus a
  daily timer (Mnema runs 24/7; launch-only checks could lag weeks). The verified document is
  cached verbatim and re-verified on read.
- **Enforcement reuses the trial-expiry seam.** A fresher CRL naming the active key flips the app
  to Read-Only Mode live (capture stops mid-session through the same capture-seam gate as trial
  expiry; recorded history stays fully readable). User-facing copy says **"revoked"**, never
  "refunded" — honest and distinct (a second-hand buyer of a resold key learns the key is dead,
  not that the app is broken) without airing the payment story.
- **Staleness never locks.** No freshness deadline, no TTL: unreachable worker, firewalled app, or
  dead fetch → the license stands. Never-lock-existing-data outranks refund enforcement. Killing
  the worker someday means apps stop learning new revocations but never lock.
- **Each release bakes in the then-current CRL** as the floor for fresh installs. Release CI
  fetches it from the worker and embeds it (signature-verified at runtime like a fetched copy);
  the fetch failing fails the release loudly rather than silently shipping a stale floor.

## Considered options

- **Do nothing (ADR 0045 status quo).** Rejected: refund abuse is a standing free-license coupon,
  and the key stays resellable forever.
- **Online license server / TTL'd licenses.** Rejected: per-user runtime validation breaks the
  zero-phone-home privacy brand outright. The CRL keeps the request anonymous and the failure
  mode open (server dead → licenses stand; a license server dead → nobody can validate).
- **Storing an `order → license` mapping in KV at mint time.** Rejected for deterministic
  derivation: the mapping adds durability questions, a missing-at-refund-time failure mode, and a
  re-mint hole (a support re-mint would get a fresh id the CRL doesn't name).

## Consequences

- **Accepted residual:** a refunder who keeps the app permanently offline keeps a frozen build.
  Every scheme short of hard-lock-on-offline shares this; hard-lock is product-destroying for a
  memory-holding recorder.
- The online guarantee: a refunded key dies within ~a day on connected installs, and immediately
  on any fresh install newer than the refund.
- Refund policy becomes cheap to make generous (14-day no-questions-asked): the refunder loses the
  key, so the cost is a fortnight of usage, not a lifetime license.
- Pre-CRL orders (minted with random UUIDs) are unrevocable — accepted, the population is tiny.
- Resolved language (**Revocation List (CRL)**, **Revoked**) lives in
  [`docs/licensing/CONTEXT.md`](../licensing/CONTEXT.md).
