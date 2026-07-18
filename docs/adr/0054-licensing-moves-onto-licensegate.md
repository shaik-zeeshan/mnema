# Licensing moves onto licensegate

## Status

Accepted. Implemented on PR #162 (2026-07-18). Supersedes the *implementation* of
[ADR 0053](0053-licenses-activate-once-per-machine-via-a-signed-activation-receipt.md) (and the
mechanism halves of [ADR 0045](0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md)
and [ADR 0056](0056-refunded-licenses-die-via-a-signed-revocation-list.md)): the in-repo
Fulfillment worker (`services/fulfillment`) and the app-side verify modules are replaced by
**licensegate**, a standalone licensing platform. Every piece of philosophy those ADRs decided
**stands unchanged**: offline verification forever, one mandatory activation per machine then
never phone home *(amended 2026-07-17 by
[ADR 0055](0055-receipt-refresh-is-event-driven-from-unhealthy-states.md): a machine whose
Update Window has lapsed quietly re-activates on a cadence until healthy again — a healthy
in-window machine still never phones home)*, staleness never locks, Read-Only Mode never holds
history hostage, the 3-device cap with self-service reset, and the privacy commitment (the
activation request carries exactly a license id, a salted irreversible machine hash, and — per
ADR 0055 — a generic hardware model label such as "Mac15,7" (`sysctl hw.model`); no name, email,
personal computer name, OS version, or telemetry; the CRL fetch stays an anonymous GET).
[ADR 0044](0044-monetize-as-one-time-purchase-with-paid-update-window.md) (the business model) is
untouched.

## Context

The bespoke stack worked but concentrated risk in the wrong place: hand-rolled Ed25519
verify/receipt/CRL code in `crates/app-infra`, an ~1200-line state machine in `licensing.rs`, a
Cloudflare Worker with KV as the server, and cross-language wire-pin tests in two repos' worth of
fixtures. Every licensing change meant touching crypto code and a second deploy target.

licensegate is that design extracted into a product and hardened: minting, activation, server
trials, checkout claim, CRL, email delivery, renewals, refund revocation, and an admin dashboard,
with its own conformance suite. Mnema should consume it and delete the bespoke stack.

The cut is clean because of one confirmed premise: **zero keys exist in the wild** (no orders, no
comps). There is nothing to migrate, so there is no dual-format period, no old-key parsing, and no
trial carryover.

## Decision

**Mnema consumes licensegate — one deployment at `license.mnema.day` (baked before the first
release build; never a platform subdomain), two products (`mnema` for release builds,
`mnema-dev` for debug, chosen by `cfg!(debug_assertions)`; the slug is part of the signature
domain, so a sandbox key can never verify on a release build). The desktop app keeps only a thin
adapter; `services/fulfillment` is deleted.**

- **The adapter, not a state machine.** `licensing.rs` verifies wires with the crate's `Verifier`
  and calls `licensegate::evaluate()`; the app-infra crypto modules (`license_verify.rs`,
  `receipt_verify.rs`, `crl_verify.rs`) are deleted. `machine_id.rs`, the keychain store, and the
  SQLite projection stay. The frontend wire types (`capture-types/src/licensing.rs` ↔
  `lib/licensing.ts`) are unchanged — the UI never sees licensegate shapes.
- **Mnema owns the clock; licensegate owns the state machine.** The existing
  max-timestamp-ever-seen rollback guard feeds `evaluate()` a *guarded* now
  (`max(wall clock, high-water mark)`), so winding the clock back still cannot stretch a trial or
  reopen a lapsed Provisional Window. The crate's `clock_tampered` flag is log-only — a broken
  clock never punishes a paying customer.
- **State mapping**: Revoked → Revoked; Expired → Read-Only Mode; Activated → Licensed
  (an activated trial-shaped key surfaces as the Trial countdown); Provisional → Licensed +
  Pending (or RefusedOverCap); ActivationRequired → Licensed + Lapsed ("connect once to finish
  activation"). **"Capture is forever" is encoded in server product config, not adapter code**:
  paid keys mint `app` perpetual + `updates` for 365 days, so Expired is unreachable for a paid
  key by construction — one pinned test asserts a perpetual-`app` key can never evaluate Expired
  (guards server-side config fat-fingering). `update_through_ms` is synthesized from the `updates`
  entitlement's day-granular expiry at exactly 00:00 UTC, the crate's own lapse instant; the
  updater gate and fresh-install-after-lapse check are untouched.
- **Trials are server-issued.** At first Capture the app requests a trial key
  (`POST /v1/trials`, trial machine hash salted by product slug — unlinkable to the
  license-id-salted activation hash by construction) and immediately chains activation. Capture
  never blocks on issuance: offline, capture starts anyway and the app retries quietly; after 7
  days of never reaching the server (the same grace length as the Provisional Window), capture
  pauses ("connect once to start your trial") until one issuance succeeds. One trial per machine
  is enforced server-side; `trial_already_used` lands in the trial-expired Read-Only UX with the
  buy door. The old serverless keychain trial is deleted, not migrated.
- **Claim flow, email stays.** Polar checkout redirects to
  `mnema://license/claim?checkout_id=…`; the app polls the claim endpoint for ~30 s while the
  webhook lands and installs + activates the key with zero paste. Every purchase still emails the
  key — the durable record and the paste-path fallback.
- **CRL repoints, floor is CI-baked.** The daily tick fetches `GET /v1/crl/mnema` anonymously via
  the shared client with the same monotonic accept; release CI fetches the live prod CRL and bakes
  it as the fresh-install floor (`MNEMA_CRL_FLOOR`), failing the release loudly if the fetch fails.
- **Over-cap is one button.** "Free up my devices" → key-authed `POST /v1/reset` → auto-retry
  activation; a rate-limited reset shows the server's retry date. Settings shows the device
  **count** (`{used, cap}` from validate) — never a list; licensegate never lists machines back,
  keeping the privacy commitment true word-for-word. `reset_url` points at a support note on
  mnema.day.
- **Old artifacts are abandoned, never parsed.** New keychain account names
  (`licensegate_key`/`_receipt`/`_first_seen`/`_trial_issuance`) in the same service; old entries
  are never read or deleted. A new migration (0048) recreates `licensing_state` preserving only
  `max_timestamp_ever_seen_ms` — an upgraded machine is indistinguishable from a fresh install.
- **The git-dep pin is the wire contract.** The client crate is a git dependency on the private
  licensegate repo, pinned to a rev (`[net] git-fetch-with-cli` in `.cargo/config.toml`; release
  CI authenticates via the `LICENSEGATE_TOKEN` secret). The crate's conformance vectors are the
  cross-repo wire truth — Mnema's old-format wire-pin tests and fixtures die with the worker.
  Bumps are deliberate, never floating.
- **Teardown.** `services/fulfillment` (worker, KV, mint/bake scripts, wire-pin tests) is deleted
  in the same cycle. Env surface shrinks to one base-URL override (`MNEMA_LICENSE_BASE_URL`,
  replacing `MNEMA_DEV_ACTIVATION_URL`/`MNEMA_ACTIVATION_URL`) plus `MNEMA_LICENSE_KID`/
  `MNEMA_LICENSE_PK_TOKEN`/`MNEMA_LICENSE_PUBLIC_KEY` (now hex); `MNEMA_CRL_URL`/
  `MNEMA_DEV_CRL_URL` are gone; `MNEMA_TRIAL_LEN_MS` is repurposed as the offline-issuance-ceiling
  override. See `docs/licensing/ENV.md`.

## Considered options

- **Keep the bespoke stack.** It shipped and its tests were green — but every future licensing
  feature (server trials, claim, device counts) would be built twice: once in the worker, once in
  the very platform extracted from it. Two implementations of one design is the definition of
  drift.
- **Keygen (ADR 0053's named fallback).** Still open, still rejected: licensegate *is* the
  "managed platform" answer with the exact offline-forever semantics 0053 chose, under the
  seller's own keys and privacy statement.
- **Dual-format transition period.** Pointless with zero keys in the wild; every hour of
  dual-format support defends nobody.

## Consequences

- **licensegate is load-bearing at the purchase moment** (as the worker was, per ADR 0053);
  server-down is still absorbed by the Provisional Window, offline grace, and email delivery —
  never a brick. Killing the server someday strands no activated machine (offline-forever
  receipts; CRL staleness never locks).
- A second private repo enters the build: cargo needs git-CLI fetch locally and a token in CI.
  The pinned rev makes wire drift a deliberate, reviewable event.
- Accepted gift at cutover: every pre-cutover machine (mid-trial or expired) gets one fresh
  server trial at next capture — cheaper than carrying two trial systems for ~no field users.
- Accepted shift: update windows now lapse at 00:00 UTC of the expiry day, not the purchase
  minute.
- Ops posture is deliberately manual at this scale: Polar's per-sale/refund emails prompt a
  dashboard glance; nightly pg_dump → R2 is the disaster plan (Polar stays the money truth;
  worst case re-mints a day's orders by hand).
- Resolved and amended domain language (**Trial**, **Fulfillment**, **Renewal**) lives in
  [`docs/licensing/CONTEXT.md`](../licensing/CONTEXT.md), "licensegate migration decisions
  (2026-07-16)".
