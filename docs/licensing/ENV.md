# Licensing — environment reference

Every environment variable, secret, and config value Mnema's licensing needs,
and which surface sets it. The server side (minting, activation, trials, claim,
reset, CRL, email) is **licensegate** — a separate deployment with its own repo,
console, and docs; nothing server-side lives in this repo (ADR 0054). This file
covers only what Mnema itself reads.

Signing keys live in licensegate: each product (`mnema` prod, `mnema-dev`
dev) has its own Ed25519 keypair, key id (kid), and publishable token. The
desktop build bakes the *verifying* side; the private keys never exist outside
licensegate.

---

## 1. Desktop app (`apps/desktop`)

### Build-time (baked into the binary via `option_env!`; `build.rs` re-bakes on change)

| Env | Who sets it | Effect |
|---|---|---|
| `MNEMA_LICENSE_PUBLIC_KEY` | release CI (prod), you (sandbox, for a dev build) | Ed25519 verifying key for the product, from the licensegate console — 64-char hex or base64 of the 32 raw bytes (both accepted; normalized in `adapter.rs`). Unset → an all-zero placeholder that constructs but verifies **nothing** (a configless build can never accept a key). |
| `MNEMA_LICENSE_KID` | same | The key id paired with the verifying key. Unset → placeholder `00000000`. |
| `MNEMA_LICENSE_PK_TOKEN` | same | Publishable API token (`pk_…`) for the product; sent on client API calls. Unset → `pk_placeholder`. |
| `MNEMA_LICENSE_BASE_URL` | optional | licensegate base URL baked as the default. Unset → `https://license.mnema.day`. |
| `MNEMA_CRL_FLOOR` | release CI | The signed CRL wire baked as the fresh-install floor. Release CI fetches it live from `https://license.mnema.day/v1/crl/mnema` and fails the release loudly if the fetch fails. Local/dev builds leave it unset — no floor. |
| `MNEMA_BUILD_DATE_MS` | automatic (`build.rs`) | Build timestamp for the update-window gate. No action needed. |

The product slug is not an env var: debug builds talk to `mnema-dev`,
release builds to `mnema` (`cfg!(debug_assertions)` in
`apps/desktop/src-tauri/src/licensing/adapter.rs`). The slug is part of the
signature domain, so a sandbox key can never verify on a release build.

`scripts/dev-app.sh` auto-exports `MNEMA_LICENSE_PUBLIC_KEY` from
`~/.mnema-licensing-keys/dev_public_key.hex` if present (drop the sandbox
product's verifying key there); export the matching `MNEMA_LICENSE_KID` and
`MNEMA_LICENSE_PK_TOKEN` yourself.

### Runtime dev knobs (debug builds only; all in `turbo.json` `passThroughEnv`)

| Env | Example | Effect |
|---|---|---|
| `MNEMA_LICENSE_ENFORCE` | `1` | Debug builds skip the gate (always Licensed) unless this is set. Set it to exercise the real trial / read-only / revoked flow locally. |
| `MNEMA_LICENSE_BASE_URL` | `http://localhost:8080` | Runtime override of the licensegate base URL (debug builds read the env var before the baked value) — point a dev build at a local licensegate without a rebuild. |
| `MNEMA_TRIAL_RESET` | `1` | Clear the trial-issuance stamp once at launch (stored keys untouched), to re-run the first-capture trial flow. |
| `MNEMA_TRIAL_LEN_MS` | `300000` | Shrink the 7-day offline-issuance ceiling (here, 5 min) so the "connect once to start your trial" gate is testable in one sitting. The trial days themselves are server-issued and not overridable client-side. |
| `MNEMA_TRIAL_LEN_DAYS` | `45` | **Compile-time** (build env, not runtime): the trial-days number shown in the pre-trial promise copy (`TrialNotStarted`). Display-only — enforcement is the server plan; set this at build time if the server plan changes. Baked fallback: 30. |
| `MNEMA_LICENSE_TOKEN_DIR` | a temp dir | Store licensing keychain items (key/receipt/stamps) as files in a directory instead of the OS keychain — test/dev isolation, same idea as `MNEMA_DEV_MASTER_KEY_FILE`. |

> Any new `MNEMA_*` var must be added to `turbo.json` `passThroughEnv` — turbo
> silently strips undeclared vars, so a knob set on the command line never
> reaches the app otherwise.

### Frontend (Vite, baked into the JS bundle)

| Env | Who sets it | Effect |
|---|---|---|
| `VITE_LICENSE_CHECKOUT_URL` | release CI (`vars.LICENSE_CHECKOUT_URL`) | Live Polar checkout link for the Buy button. Unset → the sandbox link baked as the code default (`apps/desktop/src/lib/licensing.ts`) — fine for prereleases, wrong for real buyers. |
| `VITE_RENEWAL_CHECKOUT_URL` | release CI (`vars.RENEWAL_CHECKOUT_URL`) | Live Polar renewal checkout link. Same sandbox fallback caveat. |

---

## 2. Cargo / git (the licensegate client crate)

The `licensegate` crate is a **git dependency on a private repo**, pinned to a
rev in `apps/desktop/src-tauri/Cargo.toml`. `.cargo/config.toml` sets
`[net] git-fetch-with-cli = true` so cargo uses the git CLI (and your ambient
git auth) to fetch it.

| Surface | What it needs |
|---|---|
| Local dev | git access to `github.com/shaik-zeeshan/licensegate` (SSH key or gh auth). |
| CI (`macos-release.yml`, type-check) | the `LICENSEGATE_TOKEN` repo secret — a PAT with read access to the licensegate repo; the workflow rewrites the URL to embed it. |

---

## 3. Release workflow (`.github/workflows/macos-release.yml`)

### Repository variables — Settings → Secrets and variables → Actions → **Variables**

| Variable | Required | Effect |
|---|---|---|
| `LICENSE_CHECKOUT_URL` | before selling | Live Polar checkout link → `VITE_LICENSE_CHECKOUT_URL`. |
| `RENEWAL_CHECKOUT_URL` | before selling | Live Polar renewal link → `VITE_RENEWAL_CHECKOUT_URL`. |
| `LICENSE_PUBLIC_KEY` | before selling | Prod verifying key (hex/base64) → `MNEMA_LICENSE_PUBLIC_KEY`. Unset → the all-zero placeholder is baked and the released gate verifies **nothing**. |
| `LICENSE_KID` | before selling | Prod key id → `MNEMA_LICENSE_KID`. Same placeholder caveat. |
| `LICENSE_PK_TOKEN` | before selling | Prod publishable token → `MNEMA_LICENSE_PK_TOKEN`. Same placeholder caveat. |

### Repository secrets

| Secret | Required | What it is |
|---|---|---|
| `LICENSEGATE_TOKEN` | yes | PAT with read access to the private licensegate repo (git dep fetch). |
| `TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD` | yes | Tauri updater signing key. |

The workflow itself fetches the live prod CRL and exports it as
`MNEMA_CRL_FLOOR` before the build — no variable to set; a failed fetch fails
the release.

> Release builds must bake the **prod** product's `MNEMA_LICENSE_PUBLIC_KEY` /
> `MNEMA_LICENSE_KID` / `MNEMA_LICENSE_PK_TOKEN` (from the licensegate console)
> and the `https://license.mnema.day` base URL. The domain is baked forever —
> never a hosting-platform subdomain.

---

## 4. licensegate side (console/ops, not this repo)

Deployment, products (`mnema` / `mnema-dev`), plan entitlements (`app`
perpetual + `updates` 365d paid; `app` 30d trial), device caps, Polar webhook
endpoints, Resend delivery, comp-key minting, manual revocation, and backups
are configured in the licensegate console and documented in the licensegate
repo. Mnema's side of the contract is the pinned client crate rev and the baked
config above.

_Other `MNEMA_*` vars exist for unrelated subsystems (capture dirs, keychain test
dirs, etc.) and are out of scope here — see `turbo.json` and each crate._
