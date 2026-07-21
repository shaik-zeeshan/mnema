#!/usr/bin/env bash
set -euo pipefail

# Embed the Developer ID provisioning profile and repackage mnema-cli as an
# entitled helper bundle, then re-sign (helper first, app last, so the outer
# signature seals the nested code). Shared by build-macos-local-sign.sh and
# macos-release.yml.
#
# Why (ADR 0057, verified findings): the app-groups entitlement is
# validation-required — without a profile authorising it, secd clears the
# entitlements-validated flag and the shared keychain group fails with -34018
# even under Developer ID. And a profile only validates a bundle's MAIN
# executable, so the bare sidecar binary must become a helper bundle; the
# documented sidecar path stays as a symlink (validation follows the resolved
# executable path).

app="${1:?usage: entitle-macos-app.sh <mnema.app> <signing-identity>}"
identity="${2:?usage: entitle-macos-app.sh <mnema.app> <signing-identity>}"
tauri_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../apps/desktop/src-tauri" && pwd)"

cp "${tauri_dir}/mnema.provisionprofile" "${app}/Contents/embedded.provisionprofile"

helper="${app}/Contents/Helpers/mnema-cli.app"
mkdir -p "${helper}/Contents/MacOS"
if [[ -f "${app}/Contents/MacOS/mnema-cli" && ! -L "${app}/Contents/MacOS/mnema-cli" ]]; then
  mv "${app}/Contents/MacOS/mnema-cli" "${helper}/Contents/MacOS/mnema-cli"
fi
cp "${tauri_dir}/Info.mnema-cli.plist" "${helper}/Contents/Info.plist"
cp "${tauri_dir}/mnema.provisionprofile" "${helper}/Contents/embedded.provisionprofile"
ln -sfh "../Helpers/mnema-cli.app/Contents/MacOS/mnema-cli" \
  "${app}/Contents/MacOS/mnema-cli"

# --timestamp: notarization rejects signatures without a secure timestamp.
codesign --force --options runtime --timestamp --sign "${identity}" \
  --entitlements "${tauri_dir}/Entitlements.mnema-cli.plist" \
  "${helper}"
codesign --force --options runtime --timestamp --sign "${identity}" \
  --entitlements "${tauri_dir}/Entitlements.plist" \
  "${app}"
codesign --verify --deep --strict "${app}"
