#!/usr/bin/env bash
set -euo pipefail

# Launch the mnema desktop app in a "dev profile" sandbox that is fully
# isolated from an installed production build. Lets dev + prod run together.
#
#   - Separate bundle identifier (com.shaikzeeshan.mnema.dev) and product name
#   - Separate data root:   ~/.mnema-dev            (DB, recordings, OCR models)
#   - Separate config root: ~/Library/Application Support/com.shaikzeeshan.mnema.dev
#   - Separate deep-link scheme: mnema-dev://
#   - Separate secret vault (secrets.vault lives in the dev save dir), with a
#     file-based master key so ad-hoc dev builds never hit the keychain.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Repo-root .env (gitignored): licensing build env (MNEMA_LICENSE_* etc).
# Sourced with allexport so the values reach cargo through turbo; values in
# .env override the inherited shell env.
if [[ -f "${repo_root}/.env" ]]; then
  set -a
  . "${repo_root}/.env"
  set +a
  echo "loaded ${repo_root}/.env"
fi

export MNEMA_SAVE_DIRECTORY="${HOME}/.mnema-dev"
export MNEMA_APP_CONFIG_DIR="${HOME}/Library/Application Support/com.shaikzeeshan.mnema.dev"
export MNEMA_DEV_MASTER_KEY_FILE="${MNEMA_SAVE_DIRECTORY}/dev-master-key"
# Ad-hoc dev builds can't claim the shared keychain access group (ADR 0057),
# so the capture index key lives in a file store outside the save dir.
export MNEMA_CAPTURE_INDEX_KEY_DIR="${MNEMA_APP_CONFIG_DIR}/capture-index-keys"

mkdir -p "${MNEMA_SAVE_DIRECTORY}" "${MNEMA_APP_CONFIG_DIR}" "${MNEMA_CAPTURE_INDEX_KEY_DIR}"

echo "mnema dev sandbox"
echo "  save dir:   ${MNEMA_SAVE_DIRECTORY}"
echo "  config dir: ${MNEMA_APP_CONFIG_DIR}"
if [[ -n "${MNEMA_LICENSE_ENFORCE:-}" ]]; then
  echo "  license:    enforced (real trial/read-only gate)"
else
  echo "  license:    bypassed (dev build) — set MNEMA_LICENSE_ENFORCE=1 to test gating"
fi

# Per-product keypair split (ADR 0054): bake the mnema-dev verifying key so
# this build verifies keys minted by the licensegate sandbox product and NOT
# production keys. Drop the sandbox public key (64-char hex, from the licensegate
# admin dashboard) at the path below; without it the build keeps the
# reject-everything placeholder and no key verifies. Explicit env wins. The kid,
# publishable token, and base URL ride the same override family
# (MNEMA_LICENSE_KID / MNEMA_LICENSE_PK_TOKEN / MNEMA_LICENSE_BASE_URL) — see
# docs/licensing/ENV.md.
dev_pubkey_file="${MNEMA_DEV_PUBLIC_KEY_FILE:-${HOME}/.mnema-licensing-keys/dev_public_key.hex}"
if [[ -z "${MNEMA_LICENSE_PUBLIC_KEY:-}" && -f "${dev_pubkey_file}" ]]; then
  export MNEMA_LICENSE_PUBLIC_KEY="$(tr -d '[:space:]' < "${dev_pubkey_file}")"
fi
if [[ -n "${MNEMA_LICENSE_PUBLIC_KEY:-}" ]]; then
  echo "  signing key: sandbox public key baked (sandbox-minted keys verify)"
else
  echo "  signing key: placeholder — no key verifies (no key at ${dev_pubkey_file})"
fi

# speakrs builds OpenBLAS from source; this puts the gcc lib dir on the linker
# search path so the from-source build links (see scripts/openblas-build-env.sh).
. "${repo_root}/scripts/openblas-build-env.sh"

cd "${repo_root}"
exec bun run tauri -- dev -c src-tauri/tauri.dev.conf.json "$@"
