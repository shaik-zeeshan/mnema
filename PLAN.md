# Plan: Migrate Mnema licensing onto licensegate

## Problem

Mnema's licensing runs on a bespoke in-repo stack: hand-rolled Ed25519 verify/CRL/receipt code in `crates/app-infra`, a 1179-line state machine in `licensing.rs`, and a Cloudflare Worker (`services/fulfillment`) with KV as the server. Every licensing change means touching crypto code, wire-pin tests in two languages, and a second deploy target. licensegate (the user's standalone licensing platform, extracted from this very design and since hardened — PR #67 closed every migration blocker) now does all of it as a product: minting, activation, trials, claim, CRL, email delivery, renewals, refund revocation, admin dashboard. Mnema should consume it and delete the bespoke stack.

All decisions below were resolved in the 2026-07-16 grill and are recorded in `docs/licensing/CONTEXT.md` ("licensegate migration decisions"). Premise confirmed: **zero keys exist in the wild** (no orders, no comps) — hard cut, no dual-format support.

## Solution

Deploy licensegate (Railway + Postgres + Clerk, custom domain `license.mnema.day`), create `mnema` and `mnema-sandbox` products, repoint Polar. In the desktop app: add the `licensegate` Rust client as a git dependency, delete the four app-infra crypto modules, and rewrite `licensing.rs` as a thin adapter mapping the crate's `Evaluation` onto the existing `LicenseStatus`/`Activation` wire types — the Svelte frontend stays untouched except one new button. Trials move server-side (issued at first capture, capture never blocks offline). New purchase UX: claim flow over the existing deep-link plugin, email delivery continues as today. Delete `services/fulfillment` in the same cycle.

## User Stories

1. As a new user, I want my first capture to start instantly even offline, so that my first experience is never a network error.
2. As a trial user, I want my 30 days to be honest (no clock-rollback stretching, one trial per machine), so that the trial is fair without being hostile.
3. As a buyer, I want Mnema to be licensed automatically the moment I finish Polar checkout, so that I never copy-paste a key.
4. As a buyer, I want the key emailed to me anyway, so that I have a durable record for reinstalls years later.
5. As an owner on my 4th machine, I want a one-click "free up my devices" in the app, so that hitting the device cap is never a dead end.
6. As a paying owner, I want capture to work forever regardless of update-window lapse, clock trouble, or server death, so that my purchase is truly one-time.
7. As the seller, I want licensing state in one dashboard (trials, licenses, devices, deliveries), so that support is a lookup, not spelunking KV.

## Implementation Decisions

### Server / ops (licensegate side — mostly console/ops work, not Mnema code)

- **One deployment, two products**: `mnema` (prod, Polar live) and `mnema-sandbox` (Polar sandbox). Per-product keypair/kid/publishable token/CRL provide environment isolation; no second deployment.
- **Custom domain `license.mnema.day` before the first release build** — builds bake the base URL forever; never bake a Railway subdomain.
- Product config: paid plan mints entitlements `app` (perpetual, `duration_days` NULL) + `updates` (fixed 365 days); device cap 3. Trial plan: `app` 30 days, device cap 1, trial anchor = issuance (licensegate's only mode; correct here because the app requests at first capture). Renewal mapping `action: renew` (extends same license, re-mints key). Per-product `reset_url` → support note on mnema.day; `buy_url` → Polar checkout page.
- Polar webhook → licensegate provider endpoint; Resend for outbox email (delivery format/content stays as today). Nightly pg_dump → R2 is the stated disaster plan (worst case: restore yesterday + hand-re-mint a day's orders from Polar — Polar remains the money truth).
- Monitoring is deliberately manual (licensegate v1 has no alerting): Polar's per-sale/refund emails prompt a dashboard glance; orphan renewals stay manual. Revisit at real volume.

### Desktop — adapter core

- **Add `licensegate` client crate** (git dep, `clients/rust`). **Delete** `license_verify.rs`, `receipt_verify.rs`, `crl_verify.rs` from `crates/app-infra` (crate replaces them). **Keep** `machine_id.rs` (`hardware_uuid()`), the keychain store, and the SQLite projection.
- `licensing.rs` becomes an adapter: verify wires with crate `Verifier`, call `evaluate(key, receipt, crl, first_seen_at, 7, guarded_now)`, map `State` → existing `LicenseStatus`/`Activation`. Frontend wire types in `capture-types/src/licensing.rs` + `lib/licensing.ts` unchanged.
- **Mnema owns the clock**: `guarded_now = max(wall clock, max_timestamp_ever_seen_ms)`. The crate's `clock_tampered` flag is log-only, never a lock.
- **Expired → Read-Only Mode** — reachable only by trial keys (paid keys have perpetual `app` by construction). State mapping: Revoked→Revoked, Expired→ReadOnly, Activated→Licensed, Provisional→Licensed+Activation::Pending, ActivationRequired→ReadOnly+Activation::Lapsed ("connect once to finish activation" copy stays).
- `first_seen_at` stamped by the app when a key is first stored: keychain, beside the key, write-once per license id, rollback-guarded.
- `update_through_ms` synthesized from the `updates` entitlement's day-granular expiry at **00:00 UTC** (the exact instant the crate uses). `in_window`, updater gate, and `MNEMA_BUILD_DATE_MS` fresh-install check unchanged.
- Machine hashes per licensegate spec (activation hash salted by license id, trial hash salted by product slug — unlinkable by construction; replaces the old salting scheme).
- Non-macOS behavior carries over: activation returns Activated unconditionally, never locks out.

### Desktop — trials

- Trial = server-issued key: at first capture, app calls `POST /v1/trials` (trial machine hash) then **immediately chains** `POST /v1/activate` — online users are fully Activated in one breath; a mid-chain failure falls into the ordinary 7-day provisional window.
- **Capture never blocks on issuance**: if the trial request fails, capture starts anyway and the app retries quietly. After **7 days** of never reaching the server since first attempted issuance, capture pauses (Read-Only-style gate with "connect once" copy) until one issuance succeeds. Persist the first-attempt timestamp (keychain, rollback-guarded) to drive the ceiling.
- No carryover at cutover: old keychain `trial_record` never read; every pre-cutover machine (mid-trial or expired) gets a fresh server trial at next capture. Accepted gift.
- `trial_already_used` refusal (keychain wiped, reinstall on same machine after trial) maps to ReadOnly/trial-expired UX with the buy door.

### Desktop — CRL

- Repoint daily tick to `GET /v1/crl/mnema` (still piggybacking activation retry, `crl_refresh.rs`). Crate's monotonic `accept` replaces `crl_verify.rs`; cache storage stays.
- Release CI fetches the live prod CRL during build and bakes it as the fresh-install floor.

### Desktop — claim flow + purchase UX

- Buy button → Polar checkout (hardcoded URL + `VITE_LICENSE_CHECKOUT_URL` override, as today) with success redirect to a `mnema://` deep link carrying the checkout id (existing `tauri-plugin-deep-link`; new route beside the MCP OAuth handler).
- On deep link: poll `POST /v1/claim` (checkout id, 30-day window) for a few seconds while the Polar webhook lands, then install + activate the key with zero paste. Email delivery continues on every purchase — the durable record and cross-machine fallback. Paste-a-key UI stays for the email path.

### Desktop — over-cap / reset

- Over-cap screen gains one button: "Free up my devices" → key-authed `POST /v1/reset` → auto-retry activation. `reset_rate_limited` shows the server's `retry_at` date. This is the single deliberate frontend change.
- Settings licensing panel shows the device **count** from `POST /v1/validate` (`devices: {used, cap}`) — never a device list (privacy commitment stays word-for-word: no device names sent or stored).
- `RefusedOverCap{reset_url, buy_url}` stays server-fed from `device_cap_reached` details.

### Config, storage migration, teardown

- Baked per flavor: release = prod pubkey+kid pinned, `pk_live`, `https://license.mnema.day`, CI-fetched CRL floor; dev = sandbox equivalents. `MNEMA_DEV_ACTIVATION_URL` generalizes to one base-URL override; `MNEMA_LICENSE_PUBLIC_KEY`-style compile-time overrides stay. All dev bypass knobs unchanged (debug builds skip enforcement unless `MNEMA_LICENSE_ENFORCE`; `MNEMA_TRIAL_LEN_MS`, `MNEMA_TRIAL_RESET`). Any new/renamed `MNEMA_*` var goes into `turbo.json` `passThroughEnv`.
- Keychain: same service, **new account names** for key/receipt/`first_seen_at`/trial-issuance. Old entries never read, never deleted.
- `licensing_state`: **new migration** (do NOT edit 0047 — field trial users have it applied; checksum panic). Drop/recreate the single-row cache in the new shape, **preserving only `max_timestamp_ever_seen_ms`**.
- Delete `services/fulfillment/` entirely, including its wire-pin tests and the app-side old-format fixtures; the licensegate crate's conformance vectors are the wire truth now.
- "Licensed to ⟨name/email⟩" in Settings carries over — licensegate keys carry `customer{name,email}` in the signed payload.
- Docs: write **ADR 0054** ("Licensing moves onto licensegate") superseding 0053's implementation while preserving its philosophy (offline-forever, activation-once, staleness-never-locks, privacy commitment); add supersession notes to ADR 0053 (and pointers on 0044/0045/0052). `docs/licensing/CONTEXT.md` decisions section already updated in the grill; refresh the **Fulfillment**/**Renewal** term definitions to name licensegate.

### Assumptions / open items

- licensegate Rust crate stays a git dependency (unpublished v0.2.0) — pin a rev.
- Comp keys in the new world = admin-minted licenses via `POST /admin/v1/licenses` (dashboard); no Mnema-side work.
- Trial-ceiling copy ("connect once to start your trial") needs a UX string pass during implementation — not a design blocker.

## Testing Decisions

- **Adapter mapping pins**: one table-driven test mapping every crate `State` × key-kind (paid/trial) to the expected `LicenseStatus`/`Activation` — including the sacred pin: **a key with perpetual `app` can never evaluate Expired** (guards server-side config fat-fingering).
- **Clock ownership**: rollback tests — winding `now` below the high-water mark neither extends a trial nor reopens a lapsed provisional window; `clock_tampered` never changes gating.
- **Trial grace**: capture allowed while issuance fails; pauses at day 7; resumes and anchors on first successful issuance; `first_seen_at`/first-attempt stamps are write-once and rollback-guarded.
- **Update window**: day-granular → ms synthesis pinned at 00:00 UTC against the crate's own lapse instant; existing `app_updates` gate tests keep passing untouched.
- **Migration**: new migration preserves `max_timestamp_ever_seen_ms`, drops the rest; upgraded DB ≡ fresh DB to the new code.
- **Serde round-trip** tests for `capture-types` ↔ `lib/licensing.ts` stay as-is (wire types unchanged).
- **Deleted**: all old-format wire-pin/fixture tests die with `services/fulfillment` (replaced by the crate's conformance vectors, which run in licensegate CI, not Mnema's).
- **Manual, on sandbox before cutover**: full loop — Polar sandbox checkout → webhook → mint → email arrives → deep-link claim → activated; trial on a fresh VM (online + offline-first-capture); over-cap on a 2nd/3rd/4th machine hash → in-app reset → re-activate; refund → CRL → Revoked on next daily tick.
- Not tested: licensegate server internals (its own 177-test suite owns that); keychain prompt behavior under dev signing (known env quirk).

## Slices

1. **Server/ops bring-up** (user + console; no Mnema code)
   - Goal: licensegate live at `license.mnema.day` with `mnema` + `mnema-sandbox` products, plans/entitlements per above, Polar (live+sandbox) webhooks, Resend, R2 backup cron.
   - Areas: Railway, Clerk, Polar dashboards; licensegate admin.
   - Acceptance: sandbox checkout mints a key and the email lands; `GET /v1/crl/mnema` serves.
   - Depends on: none. Parallel: with 2.
2. **Adapter core swap**
   - Goal: crate dep in; app-infra crypto modules deleted; `licensing.rs` rewritten as the `evaluate()` adapter (guarded clock, `first_seen_at`, state mapping, update-window synthesis); keychain new accounts; new `licensing_state` migration.
   - Areas: `crates/app-infra/src/`, `apps/desktop/src-tauri/src/licensing.rs`, `crates/app-infra/migrations/`.
   - Acceptance: adapter mapping pins + clock/migration tests green; `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
   - Depends on: none (test against fixture wires from the crate's vectors). Parallel: with 1.
3. **Server trial flow**
   - Goal: first capture triggers trial mint+activate chain; offline grace with 7-day ceiling; `trial_already_used` → ReadOnly UX; old trial path deleted.
   - Areas: `licensing.rs`, capture-gate seam (`license_block`), keychain stamps.
   - Acceptance: trial-grace tests green; fresh sandbox VM gets a dashboard-visible trial at first capture; capture works with network cut.
   - Depends on: 2 (and 1 for live verification).
4. **CRL repoint + baked floor**
   - Goal: daily tick fetches `/v1/crl/mnema` via crate monotonic accept; release CI fetches and bakes the floor.
   - Areas: `crl_refresh.rs`, `crl_cache.rs`, `.github/workflows/macos-release.yml`.
   - Acceptance: tick applies a newer CRL, rejects an older one; CI artifact embeds a floor.
   - Depends on: 2. Parallel: with 3, 5.
5. **Claim flow**
   - Goal: `mnema://` claim deep link → poll claim → install + activate, no paste; Polar success-redirect configured.
   - Areas: deep-link handler (`lib.rs` routing), `licensing.rs` client calls, Polar checkout config, buy-button URL params.
   - Acceptance: sandbox checkout ends with the app licensed, no manual key entry; paste path still works.
   - Depends on: 2 + 1. Parallel: with 3, 4.
6. **Over-cap UX + web support note**
   - Goal: "Free up my devices" button (reset → auto-retry), device count in Settings via validate; support note page for `reset_url`.
   - Areas: Svelte licensing UI, one Tauri command, `apps/web`.
   - Acceptance: 4th-machine sandbox activation refuses → one click resets → activates; cooldown shows `retry_at`.
   - Depends on: 2. Parallel: with 3–5.
7. **Teardown + docs**
   - Goal: delete `services/fulfillment/` + old wire tests; env-var rename in `turbo.json`; write ADR 0054 + supersession notes on 0053 (pointers on 0044/0045/0052); refresh CONTEXT.md term definitions.
   - Areas: `services/`, `turbo.json`, `docs/adr/`, `docs/licensing/CONTEXT.md`.
   - Acceptance: repo greps clean of the old worker; `bun run check` + full desktop cargo check green.
   - Depends on: 2–6 verified on sandbox.

Parallel groups: [1, 2] → [3, 4, 5, 6] → [7].

## Out of Scope

- Per-device list or per-machine deactivate UI (would amend the privacy promise; revisit on real support demand).
- Any dual-format/parallel-server period, old-key migration, or trial carryover (zero keys in the wild; hard cut).
- licensegate server feature work (all migration blockers closed in PR #67; deferred items there — in-memory rate limiter, compile-time TTLs — accepted as-is).
- Alerting/metrics stack; automated orphan-renewal refunds (manual by design at this scale).
- Windows/Linux enforcement changes (stays unconditionally Activated).

## Further Notes

- **Ordering constraint**: the custom domain must exist before the first release build bakes a base URL; slice 1 blocks the first *release*, not development (dev builds point at sandbox via override).
- **Risk — git-dep drift**: pin the licensegate crate rev; bump deliberately. Its conformance vectors are the cross-repo wire contract.
- The deleted worker's KV holds nothing worth exporting (no customers). Old builds in the field survive worker deletion by design (offline verification; CRL staleness never locks).
- Rollback story pre-cutover: until slice 7 lands, `services/fulfillment` still exists in git history; after cutover, rolling back means reverting the desktop PR — server-side nothing to unwind (Polar webhook repoint is a console toggle).
