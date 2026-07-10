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

### Environments (dev vs prod)

`wrangler.jsonc` has two **named** environments — `dev` and `production` — with
no top-level bindings, so dev and prod never share state and every command must
name an env:

- **dev** = `--env dev`. `wrangler dev --env dev` / `wrangler deploy --env dev` /
  `wrangler secret put <NAME> --env dev` / `bun run deploy:dev`. Deploys
  `mnema-fulfillment` to `workers.dev`; Polar **sandbox** product ids.
- **prod** = `--env production`. `wrangler deploy --env production` /
  `wrangler secret put <NAME> --env production` / `bun run deploy:prod`. Its own
  worker name (`mnema-fulfillment-prod`), KV namespace, product ids, and secrets.

A bare `wrangler deploy` (no `--env`) has no KV/vars and must not be used.
Wrangler does **not** inherit `vars` / `kv_namespaces` into a named env, so both
blocks declare them in full. Everything below applies per-env — set each secret
under both envs and give each its own KV id + product ids.

> **Signing key (`ED25519_PRIVATE_KEY`) is the one exception.** See "Signing
> keys: one keypair, both envs" at the end of this section before deciding
> whether dev and prod share it.

### Secrets — set with `wrangler secret put <NAME> --env <dev|production>` (never committed)

| Secret | What it is | Notes |
|---|---|---|
| `ED25519_PRIVATE_KEY` | base64 of the raw 32-byte Ed25519 seed | Signs license keys **and** the CRL. Must match `LICENSE_PUBLIC_KEY` in `license_verify.rs`. |
| `POLAR_WEBHOOK_SECRET` | Polar webhook signing secret (`whsec_<base64>`) | Verifies incoming webhooks. Accepts both the raw `whsec_…` string and the base64-decoded key. |
| `RESEND_API_KEY` | Resend API key | Sends the license-delivery email. |
| `POLAR_ACCESS_TOKEN` | Polar API token, scopes `orders:read` + `refunds:write` | Powers the renewal ownership gate: reads the buyer's license orders, auto-refunds a renewal from a non-owner. Dev = sandbox token, prod = live token. |

```sh
cd services/fulfillment
wrangler secret put ED25519_PRIVATE_KEY   # paste base64 seed
wrangler secret put POLAR_WEBHOOK_SECRET
wrangler secret put RESEND_API_KEY
wrangler secret put POLAR_ACCESS_TOKEN
```

### Non-secret vars — in `wrangler.jsonc` under `vars` (dev) and `env.production.vars` (prod)

| Var | Dev value | What it is |
|---|---|---|
| `UPDATE_WINDOW_DAYS` | `"365"` | Days of update window a purchase/renewal grants. |
| `POLAR_LICENSE_PRODUCT_ID` | `51482d45-…` (sandbox) | Polar product id for the one-time license. Refunds/paid are filtered to known product ids. Prod uses the live SKU id. |
| `POLAR_RENEWAL_PRODUCT_ID` | `adb6fc3d-…` (sandbox) | Polar product id for the renewal. Prod uses the live SKU id. |
| `POLAR_API_BASE` | `https://sandbox-api.polar.sh` | Polar REST base for the renewal ownership gate. Prod: `https://api.polar.sh`. |
| `RESEND_FROM` | `Mnema Licenses <mail@mnema.day>` | From-address for the delivery email. |

Prod values live in the `env.production.vars` block (placeholders `REPLACE_WITH_PROD_*`
until you fill them with the live Polar ids).

### KV — `IDEMPOTENCY` namespace (binding in `wrangler.jsonc`)

One namespace per env — dev and prod must not share the `revoked`/`crl` state.
Create each, then paste its id into the matching block:

```sh
wrangler kv namespace create IDEMPOTENCY --env dev           # → env.dev.kv_namespaces
wrangler kv namespace create IDEMPOTENCY --env production    # → env.production.kv_namespaces
```

### Signing keys — one keypair per env (build-time selectable)

Each env has its **own** Ed25519 keypair, so a key minted by the dev worker (dev
seed) verifies only against a dev desktop build, and a prod-minted key only
against a shipped build. The public key the desktop verifies against is chosen
at **build time**:

- **Production build** (default): verifies against the hardcoded
  `PRODUCTION_LICENSE_PUBLIC_KEY` in `crates/app-infra/src/license_verify.rs`.
  Release CI sets **nothing** — the default is production. Rotate it by pasting a
  new literal (from `gen-keypair.ts`) and shipping a build.
- **Dev/staging build**: export `MNEMA_LICENSE_PUBLIC_KEY` (standard base64 of
  the 32 raw public key bytes) before building. `license_public_key()` reads it
  via `option_env!`; `app-infra/build.rs` rebuilds when it changes. This drives
  **both** license and CRL verification (they share the key). `scripts/dev-app.sh`
  auto-exports it from `~/.mnema-licensing-keys/dev_public_key.b64` if present.

Generate a keypair with `bun scripts/gen-keypair.ts` (from `services/fulfillment/`):
it prints the base64 seed (→ `ED25519_PRIVATE_KEY` worker secret / mint seed),
the base64 public key (→ `MNEMA_LICENSE_PUBLIC_KEY` / the dev pubkey file), and
the Rust literal for a prod rotation. Store each seed in the seller's password
manager, one per env; never commit a seed.

To match keys per env: worker `ED25519_PRIVATE_KEY --env dev` uses the **dev**
seed; `--env production` uses the **prod** seed. The dev seed's public key is
what dev builds bake. `bake-crl.ts` still verifies against the production key
(it bakes the prod floor into release builds) — dev doesn't bake a floor; a dev
build's placeholder floor simply verifies to `None` and it live-fetches the dev
CRL. **Even with the split: never hand a dev/sandbox-minted key to a real buyer.**

Holds three kinds of keys:
- per-order idempotency markers (keyed by Polar order id),
- `revoked:<license_id>` — one blind key per revoked license id, value `"1"`
  (the source of truth; the worker lists the `revoked:` prefix — there is NO
  single `revoked` array key),
- `crl` — the current signed CRL wire string (rebuilt when the `revoked:` set
  changes).

**Manually revoke a comp key (or any leaked key)** — write ONE per-id key (the
worker lists the `revoked:` prefix; a bare `revoked` array key is never read).
The `crl` is re-signed lazily on the next `GET /revocations.json`:

```sh
# one key per id, value "1" (append --env production for prod)
wrangler kv key put   --binding IDEMPOTENCY 'revoked:comp:press-jane' 1
wrangler kv key put   --binding IDEMPOTENCY 'revoked:order:<uuid>'    1
```

---

## 2. Local scripts (`services/fulfillment/scripts`, run with `bun`)

### `gen-keypair.ts` — generate a per-env Ed25519 licensing keypair

No env needed. Prints a fresh seed (base64 → `ED25519_PRIVATE_KEY`), its public
key (base64 → `MNEMA_LICENSE_PUBLIC_KEY` / the dev pubkey file), and the Rust
literal for `PRODUCTION_LICENSE_PUBLIC_KEY` (prod rotation). Run once per env;
store each seed in the password manager, never in the repo.

```sh
bun scripts/gen-keypair.ts
# dev: save the printed public key to ~/.mnema-licensing-keys/dev_public_key.b64
#      (scripts/dev-app.sh bakes it), and set the seed as the dev worker secret.
```

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

There's no dev/prod switch in the script — you pass the seed, and the seed *is*
the env. Pass the **prod** seed to mint real keys (verify against shipped
builds); pass the **dev** seed for keys that only verify against a dev build.
Keep the two seeds in separate password-manager entries. See "Signing keys"
above. Never hand a dev-seed mint to a real buyer.

### `bake-crl.ts` — verify + bake the live CRL into the binary floor

Fetches `/revocations.json`, verifies its signature against the **production
public key** (no secret needed — verification only), and overwrites
`crates/app-infra/revocations.json`. Fails loudly on non-2xx / bad signature.

| Env | Required | Default | What it is |
|---|---|---|---|
| `CRL_ENDPOINT` | no | dev `workers.dev` deploy | URL to fetch and verify. |

Dev/prod for this script is just the endpoint: point `CRL_ENDPOINT` at the dev
worker or the prod worker/custom domain. The default fallback is the dev deploy.

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
| `MNEMA_LICENSE_PUBLIC_KEY` | you, for a dev/staging build (unset = production) | Base64 of the 32-byte Ed25519 public key licenses **and** the CRL verify against. Read via `option_env!` in `license_public_key()` (`app-infra`); unset → the hardcoded `PRODUCTION_LICENSE_PUBLIC_KEY`. `app-infra/build.rs` re-bakes on change. `scripts/dev-app.sh` auto-sets it from `~/.mnema-licensing-keys/dev_public_key.b64`. Must be a real exported env var (not `cargo:rustc-env`) so `app-infra`'s rustc sees it — and it's in `turbo.json` `passThroughEnv` so turbo doesn't strip it. |
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
| Worker (Cloudflare) — **per env** (`--env dev` / `--env production`) | secrets `ED25519_PRIVATE_KEY`, `POLAR_WEBHOOK_SECRET`, `RESEND_API_KEY`; the `IDEMPOTENCY` KV id; product ids in `wrangler.jsonc` (`env.dev` / `env.production`) |
| `gen-keypair.ts` | nothing — run once per env to create the keypair |
| `mint-local.ts` | `ED25519_PRIVATE_KEY` in the shell (the env's seed) |
| `bake-crl.ts` | nothing (optionally `CRL_ENDPOINT`) — always verifies against the prod key |
| Desktop **dev** build | drop the dev public key at `~/.mnema-licensing-keys/dev_public_key.b64` (or export `MNEMA_LICENSE_PUBLIC_KEY`) so dev-minted keys verify; `MNEMA_LICENSE_ENFORCE` / `MNEMA_DEV_CRL_URL` / `MNEMA_TRIAL_*` as needed |
| Release CI | repo variable `CRL_URL`; secrets `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)`. Leave `MNEMA_LICENSE_PUBLIC_KEY` **unset** → production key |

_Other `MNEMA_*` vars exist for unrelated subsystems (capture dirs, keychain test
dirs, etc.) and are out of scope here — see `turbo.json` and each crate._
