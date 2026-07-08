# Licensing & CRL — environment reference

Every environment variable, secret, and config value the licensing system needs,
and which surface sets it. Covers the Polar fulfillment worker, the local
mint/bake scripts, the desktop build + dev knobs, and the release workflow.

The one value that ties everything together is the **Ed25519 seed**. The worker
signs both license keys and the revocation list (CRL) with it; the desktop
verifies both against the hardcoded public key in
`crates/app-infra/src/license_verify.rs` (`LICENSE_PUBLIC_KEY`). The seed must
correspond to that public key. It lives only in the seller's password manager
and as a worker secret — never in the repo.

---

## 1. Fulfillment worker (`services/fulfillment`, Cloudflare)

Deployed with `wrangler deploy` from `services/fulfillment/`. Handles Polar
`order.paid` (mint + email a key), `order.refunded` (revoke on full refund), and
`GET /revocations.json` (serve the signed CRL).

### Secrets — set with `wrangler secret put <NAME>` (never committed)

| Secret | What it is | Notes |
|---|---|---|
| `ED25519_PRIVATE_KEY` | base64 of the raw 32-byte Ed25519 seed | Signs license keys **and** the CRL. Must match `LICENSE_PUBLIC_KEY` in `license_verify.rs`. |
| `POLAR_WEBHOOK_SECRET` | Polar webhook signing secret (`whsec_<base64>`) | Verifies incoming webhooks. Accepts both the raw `whsec_…` string and the base64-decoded key. |
| `RESEND_API_KEY` | Resend API key | Sends the license-delivery email. |

```sh
cd services/fulfillment
wrangler secret put ED25519_PRIVATE_KEY   # paste base64 seed
wrangler secret put POLAR_WEBHOOK_SECRET
wrangler secret put RESEND_API_KEY
```

### Non-secret vars — in `wrangler.jsonc` under `vars`

| Var | Current value | What it is |
|---|---|---|
| `UPDATE_WINDOW_DAYS` | `"365"` | Days of update window a purchase/renewal grants. |
| `POLAR_LICENSE_PRODUCT_ID` | `51482d45-…` | Polar product id for the one-time license. Refunds/paid are filtered to known product ids. |
| `POLAR_RENEWAL_PRODUCT_ID` | `adb6fc3d-…` | Polar product id for the renewal. |
| `RESEND_FROM` | `Mnema Licenses <mail@mnema.day>` | From-address for the delivery email. |

### KV — `IDEMPOTENCY` namespace (binding in `wrangler.jsonc`)

Create once, then paste the id into `wrangler.jsonc`:

```sh
wrangler kv namespace create IDEMPOTENCY
```

Holds three kinds of keys:
- per-order idempotency markers (keyed by Polar order id),
- `revoked` — JSON array of revoked license ids (source of truth),
- `crl` — the current signed CRL wire string (rebuilt when `revoked` changes).

**Manually revoke a comp key (or any leaked key)** — edit the `revoked` set; the
worker re-signs `crl` lazily on the next `GET`:

```sh
# read current set, add your id, write it back
wrangler kv key get   --binding IDEMPOTENCY revoked
wrangler kv key put   --binding IDEMPOTENCY revoked '["comp:press-jane","order:<uuid>"]'
```

---

## 2. Local scripts (`services/fulfillment/scripts`, run with `bun`)

### `mint-local.ts` — issue re-mints and comp keys

Needs the seed in the environment. Prints the key to stdout; delivery is manual.

| Env | Required | What it is |
|---|---|---|
| `ED25519_PRIVATE_KEY` | yes | base64 seed (same as the worker secret). |

```sh
# Re-mint a lost key for an existing order (window dates from the ORIGINAL order date):
ED25519_PRIVATE_KEY=<b64-seed> bun scripts/mint-local.ts \
  --order-id <polar_order_id> --email <buyer@x> --order-date <ISO-8601 | unix-ms>

# Comp key (gift; no order behind it), default 90-day update window:
ED25519_PRIVATE_KEY=<b64-seed> bun scripts/mint-local.ts \
  --comp <slug> --email <person@x> [--update-days 90]
```
Mode is chosen by `--order-id` (re-mint) xor `--comp` (comp). License ids become
`order:<id>` / `comp:<slug>` — the same ids the CRL revokes.

### `bake-crl.ts` — verify + bake the live CRL into the binary floor

Fetches `/revocations.json`, verifies its signature against the **production
public key** (no secret needed — verification only), and overwrites
`crates/app-infra/revocations.json`. Fails loudly on non-2xx / bad signature.

| Env | Required | Default | What it is |
|---|---|---|---|
| `CRL_ENDPOINT` | no | dev `workers.dev` deploy | URL to fetch and verify. |

```sh
# Dry-run against the live worker (does not need the seed):
CRL_ENDPOINT=https://<crl-host>/revocations.json bun services/fulfillment/scripts/bake-crl.ts
```
Release CI runs this automatically (below). Locally you normally don't run it —
the committed placeholder floor keeps offline builds working.

---

## 3. Desktop app (`apps/desktop`)

### Build-time (baked into the binary)

| Env | Who sets it | Effect |
|---|---|---|
| `MNEMA_CRL_URL` | release CI (or you, for a custom build) | The CRL URL compiled into the binary via `option_env!`. When unset, falls back to `DEFAULT_CRL_URL` (the dev `workers.dev` deploy) in `crl_refresh.rs`. `build.rs` re-bakes on change. |
| `MNEMA_BUILD_DATE_MS` | automatic (`build.rs`) | Build timestamp for the update-window gate. No action needed. |

> A `workers.dev` host baked into a shipped binary can't be repointed later.
> Before shipping to real users, put a custom domain in front of the worker
> (e.g. `crl.mnema.app`) and set `MNEMA_CRL_URL` to it.

### Runtime dev knobs (debug builds only; already in `turbo.json` `passThroughEnv`)

| Env | Example | Effect |
|---|---|---|
| `MNEMA_LICENSE_ENFORCE` | `1` | Debug builds skip the gate (always Licensed) unless this is set. Set it to exercise the real trial / read-only / revoked flow locally. |
| `MNEMA_DEV_CRL_URL` | `http://localhost:8787/revocations.json` | Override the CRL fetch URL at runtime to test a revoked key without a rebuild. Takes precedence over the baked `MNEMA_CRL_URL`. |
| `MNEMA_TRIAL_RESET` | `1` | Wipe the stored trial start once at launch, to re-run the fresh-trial flow. |
| `MNEMA_TRIAL_LEN_MS` | `300000` | Shrink the whole trial window (here, 5 min) so the trial→read-only flip is testable in one sitting. |

> Any new `MNEMA_*` var must be added to `turbo.json` `passThroughEnv` — turbo
> silently strips undeclared vars, so a knob set on the command line never
> reaches the app otherwise.

**Simulate a revocation end-to-end locally:**
```sh
# 1. serve a signed CRL naming your active key at some URL (or point at the worker)
# 2. run a debug build with the gate on and the dev CRL URL:
MNEMA_LICENSE_ENFORCE=1 MNEMA_DEV_CRL_URL=http://localhost:8787/revocations.json bun run tauri -- dev
# → app flips to Read-Only with the "revoked" message; the flip persists (cache is monotonic)
```

---

## 4. Release workflow (`.github/workflows/macos-release.yml`)

### Repository variable — Settings → Secrets and variables → Actions → **Variables**

| Variable | Required | Effect |
|---|---|---|
| `CRL_URL` | optional | Production CRL endpoint. Feeds **both** the baked const (`MNEMA_CRL_URL`) and the baked floor (`CRL_ENDPOINT` for `bake-crl.ts`) from one source. Unset → falls back to the dev `workers.dev` deploy. Set this to your custom domain before a real release. |

### Repository secrets — Settings → Secrets and variables → Actions → **Secrets**

| Secret | Required | What it is |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | yes | Tauri updater signing key (signs the release artifacts). |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | yes | Password for that key. |
| `GITHUB_TOKEN` | automatic | Provided by Actions; no action needed. |

### Job env already set in the workflow (no action)

| Env | Value | Why |
|---|---|---|
| `MNEMA_CRL_URL` | `${{ vars.CRL_URL || <dev deploy> }}` | Baked into the build; also drives the bake step's `CRL_ENDPOINT`. |
| `OPENBLAS_DYNAMIC_ARCH` | `1` | All-generation arm64 kernels so an M-series build runs on older Macs. |
| `APPLE_SIGNING_IDENTITY` | `-` | Ad-hoc signing. |
| `CI` | `true` | Standard CI marker. |

> The worker is **not** deployed by this workflow — deploy it (`wrangler deploy`)
> before shipping a desktop build, so the baked CRL URL is already live.

---

## Quick reference — what to set, where

| Surface | You must set |
|---|---|
| Worker (Cloudflare) | secrets `ED25519_PRIVATE_KEY`, `POLAR_WEBHOOK_SECRET`, `RESEND_API_KEY`; the `IDEMPOTENCY` KV id; product ids in `wrangler.jsonc` |
| `mint-local.ts` | `ED25519_PRIVATE_KEY` in the shell |
| `bake-crl.ts` | nothing (optionally `CRL_ENDPOINT`) |
| Desktop dev | nothing to build; `MNEMA_LICENSE_ENFORCE` / `MNEMA_DEV_CRL_URL` / `MNEMA_TRIAL_*` as needed |
| Release CI | repo variable `CRL_URL`; secrets `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` |

_Other `MNEMA_*` vars exist for unrelated subsystems (capture dirs, keychain test
dirs, etc.) and are out of scope here — see `turbo.json` and each crate._
