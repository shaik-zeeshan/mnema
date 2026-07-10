# Licenses activate once per machine via a signed Activation Receipt

## Status

Proposed. Supersedes [ADR 0045](0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md)'s
rejection of an activation server ("the desktop app and the customer must never phone home" →
now "activates once per machine, then never phones home"). Amends
[ADR 0052](0052-refunded-licenses-die-via-a-signed-revocation-list.md): the CRL's charter widens
from full refunds only to also cover demonstrably-leaked keys, and the activation endpoint
refuses revoked ids outright.

## Context

ADR 0045 chose pure offline verification: any machine holding a valid signed key gets Capture,
forever, with zero server contact. That leaves exactly one abuse channel wide open: **casual key
sharing** — one purchased key pasted on unlimited machines. The CRL (ADR 0052) can kill a key
after the fact but has no way to *see* sharing happen.

A deep-research pass over shipped comparables found that **one-time online activation is the one
mechanism with evidence of measurably suppressing casual piracy** (Tyler Hall's Nottingham 2.0
data: piracy "virtually non-existent" post-activation; directional — one dated self-reported
source). Honor-system peers (Sublime Text, Little Snitch) survive fine without it, but Sublime
still blacklists widely-leaked keys — enforcement-heavy peers (TablePlus, 1-device cap) show the
overcorrection to avoid. Crackers are out of scope either way: any client check is patchable
(Sublime 3 fell to a single `cmp`); Ed25519 kills keygens, never cracks.

A market survey (2026 pricing) confirmed no off-the-shelf option beats building on what exists:
every payment-platform key mechanism (Polar, Lemon Squeezy, Gumroad) is online-validation-per-
check; Paddle exited licensing; Cryptlex/Zentitle TTL their offline artifacts; only **Keygen**
matches the "activate once, verify offline forever" model (Cloud likely $0 at our scale, CE
self-hosted). But the Fulfillment worker already ships the signing key, KV, domain separation,
and webhook plumbing — the self-built delta (one endpoint, one reset route, one KV record shape)
is smaller than a Keygen integration, and the migration path onto Keygen was verified open in
both directions (bring-your-own key strings, script-importable licenses/machines with
developer-supplied fingerprints, full API export back out).

## Decision

**Pasting a key performs one mandatory online Activation per machine; the server binds
license↔machine in a signed Activation Receipt the app verifies offline forever. No heartbeat,
no expiry, no runtime calls after.**

- **The request** carries exactly `license_id` + a salted, irreversible hash of a
  hardware-stable machine identifier (macOS Hardware UUID — survives factory reset). The salt
  derives from the license id, so the same machine always hashes the same (idempotency depends
  on it). No name, email, device name, OS, or telemetry. This privacy commitment is public.
- **The receipt** is Ed25519-signed with the existing key, **domain-separated** (as the CRL is)
  so receipts, licenses, and CRLs can never be replayed as one another. Verified against the
  same hardcoded public key. A receipt never outranks the CRL.
- **Mandatory, with a Provisional Window**: a signature-valid key grants Capture immediately
  while activation retries in the background; after **7 days from the first activation attempt
  that could not reach the server** (pasting while online consumes nothing; a paste that
  activates immediately never opens the window) the app drops to Read-Only Mode until an
  activation succeeds. *(Amended 2026-07-10: originally "7 days of actual server
  unreachability" — the implementation deliberately does not distinguish server-down from
  user-offline once the window opens; metering true unreachability would need attempt-outcome
  tracking for a residual that self-heals on the next reconnect.)* The window
  is consumed **per license id**, recorded in the OS keychain with the trial's
  max-timestamp-ever-seen rollback guard — re-pasting the same key grants no new window; a
  different purchased key gets its own. "Staleness never locks" is hereby scoped: it protects
  machines *holding a receipt*; a never-activated machine is still establishing the license.
  Comp Keys activate through the same path — a leaked comp key at machine 4 is exactly what the
  cap catches.
- **Cap = 3 machines**, held as a lifetime *set* of machine hashes (not a counter) in one KV
  record per license id: `activation:<license_id> → { machines[], last_reset_at,
  lifetime_machine_count }`. Idempotent by construction: factory reset, reinstall, re-paste,
  and lost-key re-mints (same derived license id) all land on an existing hash and consume
  nothing. **Renewals** keep their own license id (per ADR 0052's derivation — a refunded
  renewal must revoke only itself) and therefore carry their own slots; pasting a renewal key
  triggers one silent re-activation. Accepted: slots multiply only with paid, owner-verified
  renewals.
- **At-cap refusal is never a dead end**: the refusal response carries a self-service **reset**
  link and a buy-another-license link. Reset is authorized by **possession of the key** (pasted
  on a seller web page, verified against the public key — no accounts), rate-limited to once
  per 30 days, and empties the slot set. It **cannot** invalidate issued receipts — those are
  offline-forever by design. The structural residual (activate 3, reset monthly, activate 3
  more; old machines keep working) is policed not at the reset lever but by the **lifetime
  distinct-machine count**: a license that has touched an implausible number of machines is
  leaked, and that is a CRL entry — which *does* reach already-activated machines. This is the
  telemetry the CRL always lacked, and it widens the CRL's charter beyond refunds (amends 0052).
- **The activation endpoint refuses Revoked license ids**, so a dead key can never plant itself
  on a new machine; previously-activated machines die via the CRL as before.
- **The Trial stays 100% serverless.** The activation request is the first byte Mnema's
  licensing ever sends anywhere; nothing happens before a key is pasted.
- **Identity in the key**: the mint adds the buyer's `name` (empty when Polar lacks it) beside
  `email` in the signed payload; the app shows "Licensed to ⟨name, falling back to email⟩" in
  Settings. Display only — never verified. Social deterrent at the cost of one field.
- **No air-gap challenge-response, no per-device dashboard, no device names** at launch. Mnema
  is local-first, not offline-only (models, updater, and CRL already need the network), so the
  permanently-air-gapped buyer is not a real persona; the Provisional Window absorbs transient
  offline and support email is the escape hatch. A dashboard stores more than the privacy
  commitment allows, for a moment most users hit once in years — reset-all covers it.

## Considered options

- **Stay honor-system (ADR 0045 status quo).** Viable — Sublime/Little Snitch prove it — but
  leaves casual sharing invisible and unenforceable, and the CRL blind. The activation count is
  as much about *seeing* leaks as stopping them.
- **Keygen Cloud / CE.** The only off-the-shelf match for offline-forever receipts (Ed25519
  machine files, `ttl=null`). Rejected for now: rewrites working mint/renewal/CRL code around a
  third-party API, adds a second party to the privacy statement, and its free tier is someone
  else's pricing decision. **Named fallback**: if activation ever demands a device dashboard,
  admin UI, or real operational burden, migrate to Keygen rather than growing an admin platform
  in the worker — the import path (BYOK keys, pre-registered machines with our fingerprints,
  full export back) is verified open.
- **Payment-platform license keys (Polar / Lemon Squeezy / Gumroad).** All
  online-validation-per-check in 2026 — breaks offline-forever outright. Lemon Squeezy is
  additionally winding into Stripe. Paddle exited licensing.
- **Cryptlex / LicenseSpring / LimeLM / Zentitle.** TTL'd or unclear offline artifacts
  (Cryptlex, LicenseSpring, Zentitle) or a closed C library with no Rust binding and
  lifetime-metered pricing (LimeLM). None beat a few hundred lines on owned infrastructure.
- **Soft activation (unlock anyway on failure).** Worthless: a firewall rule defeats it while
  the architectural cost is fully paid.
- **Provisional Window ends in a nag, not Read-Only.** Rejected by the seller: "just because a
  person will not pay doesn't mean we shouldn't have safeguards." Read-Only Mode is explicitly
  not a lock-out (history stays fully usable), so the never-lock-existing-data rule holds.
- **Renewals inherit the original license id (shared slots).** Rejected: revocation kills by
  license id, so a refunded renewal would either kill the original paid license or be
  unrevocable.

## Consequences

- **Positioning changes**: "never phones home" becomes **"activates once per machine, then
  never phones home"** everywhere (site, docs, CONTEXT.md). Honest, and still a differentiator
  — competitors with license servers check continuously.
- **Fulfillment gains per-license state** (the activation KV record) and its first
  app-initiated runtime endpoint. The worker is now load-bearing at the purchase moment;
  server-down is absorbed by the Provisional Window, never a brick.
- The purchase moment gains a network dependency; the trial and all post-activation runtime
  remain zero-contact.
- New domain terms (**Activation**, **Activation Receipt**, **Provisional Window**,
  **Device Cap / Reset**) live in [`docs/licensing/CONTEXT.md`](../licensing/CONTEXT.md).
- Research provenance: activation-efficacy and comparables from the 2026-07 deep-research pass
  (tyler.io Nottingham 2.0 post; Sublime portable-license docs; Little Snitch order page;
  TablePlus licensing docs; keygen.sh offline-license docs); pricing survey and Keygen
  migration verification from the 2026-07-09 grill session.
