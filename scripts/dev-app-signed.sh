#!/usr/bin/env bash
set -euo pipefail

# Signed dev sandbox: same isolation as dev-app.sh (day.mnema.dev bundle id,
# ~/.mnema-dev data root, mnema-dev:// scheme) but built as a real signed .app
# so TCC grants (mic / system audio / screen) attach to a stable Team-ID
# identity and persist across rebuilds. Debug profile.
#
# Unlike dev-app.sh, this build uses the KEYCHAIN for the vault master key and
# capture-index DB key, exactly like prod — a signed build has a stable
# signature, so one "Always Allow" sticks. The file knobs
# (MNEMA_DEV_MASTER_KEY_FILE / MNEMA_CAPTURE_INDEX_KEY_DIR) exist only for
# ad-hoc `tauri dev` builds and are deliberately NOT set here. If you switch
# back to plain `bun run dev:sandbox` afterwards, delete
# ~/.mnema-dev/secrets.vault first — the file key can't unlock a
# keychain-keyed vault.
#
# Tradeoff vs dev-app.sh: no vite hot reload — full rebuild per change.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Repo-root .env (gitignored): licensing build env, updater signing key.
if [[ -f "${repo_root}/.env" ]]; then
  set -a
  . "${repo_root}/.env"
  set +a
  echo "loaded ${repo_root}/.env"
fi

export MNEMA_SAVE_DIRECTORY="${HOME}/.mnema-dev"
export MNEMA_APP_CONFIG_DIR="${HOME}/Library/Application Support/day.mnema.dev"
mkdir -p "${MNEMA_SAVE_DIRECTORY}" "${MNEMA_APP_CONFIG_DIR}"

echo "mnema signed dev sandbox"
echo "  save dir:   ${MNEMA_SAVE_DIRECTORY}"
echo "  config dir: ${MNEMA_APP_CONFIG_DIR}"
echo "  keys:       keychain (day.mnema.vault / day.mnema.capture-index)"

# Sandbox licensing keypair (ADR 0054) — same resolution as dev-app.sh.
dev_pubkey_file="${MNEMA_DEV_PUBLIC_KEY_FILE:-${HOME}/.mnema-licensing-keys/dev_public_key.hex}"
if [[ -z "${MNEMA_LICENSE_PUBLIC_KEY:-}" && -f "${dev_pubkey_file}" ]]; then
  export MNEMA_LICENSE_PUBLIC_KEY="$(tr -d '[:space:]' < "${dev_pubkey_file}")"
fi

# Signing identity — same discovery as build-macos-local-sign.sh.
identity="${APPLE_SIGNING_IDENTITY:-}"
if [[ -z "${identity}" ]]; then
  identity="$(security find-identity -v -p codesigning | grep 'Apple Development' | head -n 1 | sed -E 's/.*"(.*)"/\1/' || true)"
fi
if [[ -z "${identity}" ]]; then
  echo "No Apple Development signing identity found (see build-macos-local-sign.sh)." >&2
  exit 1
fi
echo "  identity:   ${identity}"

# speakrs/OpenBLAS from-source link path (local machine only, no DYNAMIC_ARCH).
. "${repo_root}/scripts/openblas-build-env.sh"

cd "${repo_root}/apps/desktop"
CI=true APPLE_SIGNING_IDENTITY="${identity}" \
  bun run tauri -- build --debug --bundles app -c src-tauri/tauri.dev.conf.json

# Launch through LaunchServices, not a shell exec: TCC attributes permission
# requests to the *responsible process*, and a shell-exec'd binary inherits the
# terminal as responsible — prompts never fire. `open` makes launchd the parent
# so the day.mnema.dev bundle owns its TCC identity. Runtime env rides in
# LSEnvironment (open does not pass env), then the outer signature is redone
# so the edited Info.plist stays sealed.
app="${repo_root}/target/debug/bundle/macos/mnema-dev.app"
plist="${app}/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Delete :LSEnvironment" "${plist}" 2>/dev/null || true
/usr/libexec/PlistBuddy \
  -c "Add :LSEnvironment dict" \
  -c "Add :LSEnvironment:MNEMA_SAVE_DIRECTORY string ${MNEMA_SAVE_DIRECTORY}" \
  -c "Add :LSEnvironment:MNEMA_APP_CONFIG_DIR string ${MNEMA_APP_CONFIG_DIR}" \
  "${plist}"
codesign --force --sign "${identity}" --options runtime \
  --entitlements "${repo_root}/apps/desktop/src-tauri/Entitlements.plist" "${app}"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "${app}"
echo "launching ${app} via open"
open "${app}"
