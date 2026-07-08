#!/usr/bin/env bash
set -euo pipefail

# Launch the mnema desktop app in a "dev profile" sandbox that is fully
# isolated from an installed production build. Lets dev + prod run together.
#
#   - Separate bundle identifier (com.shaikzeeshan.mnema.dev) and product name
#   - Separate data root:   ~/.mnema-dev            (DB, recordings, OCR models)
#   - Separate config root: ~/Library/Application Support/com.shaikzeeshan.mnema.dev
#   - Separate deep-link scheme: mnema-dev://
#   - API keys are intentionally shared with prod via the OS keychain.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export MNEMA_SAVE_DIRECTORY="${HOME}/.mnema-dev"
export MNEMA_APP_CONFIG_DIR="${HOME}/Library/Application Support/com.shaikzeeshan.mnema.dev"

mkdir -p "${MNEMA_SAVE_DIRECTORY}" "${MNEMA_APP_CONFIG_DIR}"

echo "mnema dev sandbox"
echo "  save dir:   ${MNEMA_SAVE_DIRECTORY}"
echo "  config dir: ${MNEMA_APP_CONFIG_DIR}"
if [[ -n "${MNEMA_LICENSE_ENFORCE:-}" ]]; then
  echo "  license:    enforced (real trial/read-only gate)"
else
  echo "  license:    bypassed (dev build) — set MNEMA_LICENSE_ENFORCE=1 to test gating"
fi

# Per-env keypair split (ADR 0045): bake the DEV licensing public key so this
# build verifies keys minted by the dev Fulfillment worker (dev seed) and NOT
# production keys. Drop the base64 dev public key (from `bun scripts/gen-keypair.ts`)
# at the path below; without it the build falls back to the production key and
# dev-minted keys won't verify. Explicit env wins.
dev_pubkey_file="${MNEMA_DEV_PUBLIC_KEY_FILE:-${HOME}/.mnema-licensing-keys/dev_public_key.b64}"
if [[ -z "${MNEMA_LICENSE_PUBLIC_KEY:-}" && -f "${dev_pubkey_file}" ]]; then
  export MNEMA_LICENSE_PUBLIC_KEY="$(tr -d '[:space:]' < "${dev_pubkey_file}")"
fi
if [[ -n "${MNEMA_LICENSE_PUBLIC_KEY:-}" ]]; then
  echo "  signing key: DEV public key baked (dev-minted keys verify)"
else
  echo "  signing key: production (no dev key at ${dev_pubkey_file})"
fi

# speakrs builds OpenBLAS from source; this puts the gcc lib dir on the linker
# search path so the from-source build links (see scripts/openblas-build-env.sh).
. "${repo_root}/scripts/openblas-build-env.sh"

cd "${repo_root}"
exec bun run tauri -- dev -c src-tauri/tauri.dev.conf.json "$@"
