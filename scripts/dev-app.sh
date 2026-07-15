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

export MNEMA_SAVE_DIRECTORY="${HOME}/.mnema-dev"
export MNEMA_APP_CONFIG_DIR="${HOME}/Library/Application Support/com.shaikzeeshan.mnema.dev"
export MNEMA_DEV_MASTER_KEY_FILE="${MNEMA_SAVE_DIRECTORY}/dev-master-key"

mkdir -p "${MNEMA_SAVE_DIRECTORY}" "${MNEMA_APP_CONFIG_DIR}"

echo "mnema dev sandbox"
echo "  save dir:   ${MNEMA_SAVE_DIRECTORY}"
echo "  config dir: ${MNEMA_APP_CONFIG_DIR}"

# speakrs builds OpenBLAS from source; this puts the gcc lib dir on the linker
# search path so the from-source build links (see scripts/openblas-build-env.sh).
. "${repo_root}/scripts/openblas-build-env.sh"

cd "${repo_root}"
exec bun run tauri -- dev -c src-tauri/tauri.dev.conf.json "$@"
