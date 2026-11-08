#!/usr/bin/env bash
# bash scripts/bundle-gpui-app.sh --dev [--fresh]
#
# Builds the GPUI app (`mnema-gpui`) and assembles an ad-hoc-signed dev bundle:
#   ${CARGO_TARGET_DIR:-target}/gpui-bundle/Mnema GPUI Dev.app
# identity day.mnema.gpui-dev, URL scheme mnema-gpui-dev, LSMinimumSystemVersion 15.
# Launch it with `open` (LaunchServices), never by exec'ing the binary.
#
# The bundle runs on a COPY of the Tauri dev sandbox, never ~/.mnema-dev itself
# (startup retention deletes data), with file-backed keys and no keychain:
#   data   ~/.mnema-dev                                -> ~/.mnema-gpui-dev
#   config ~/Library/Application Support/day.mnema.dev -> .../day.mnema.gpui-dev
# Those paths ride in LSEnvironment because `open` passes no env. --fresh re-copies.
#
# An early cut of S16: no --release, no mnema-cli, no entitlements or profile, no
# notarization. TCC flows (screen, mic, system audio) need the founder's signed
# build; an ad-hoc signature changes every rebuild.
set -euo pipefail

mode=""
fresh=0
for arg in "$@"; do
  case "$arg" in
    --dev) mode=dev ;;
    --fresh) fresh=1 ;;
    *) echo "bundle-gpui-app: unknown argument $arg" >&2; exit 2 ;;
  esac
done
[[ "$mode" == dev ]] || { echo "usage: bundle-gpui-app.sh --dev [--fresh] (--release is S16)" >&2; exit 2; }

repo="$(cd "$(dirname "$0")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"
bundle_id="day.mnema.gpui-dev"
scheme="mnema-gpui-dev"
app="$target/gpui-bundle/Mnema GPUI Dev.app"

src_data="${HOME}/.mnema-dev"
src_config="${HOME}/Library/Application Support/day.mnema.dev"
data="${HOME}/.mnema-gpui-dev"
config="${HOME}/Library/Application Support/${bundle_id}"

copy() { # copy <from> <to>: clone `from` into `to` unless `to` exists (or --fresh)
  local from="$1" to="$2"
  [[ "$to" != "$from" && "$to" != "$HOME/.mnema" && "$to" != "$HOME/.mnema-dev" ]] ||
    { echo "bundle-gpui-app: refusing to use the original $to" >&2; exit 1; }
  if [[ $fresh -eq 1 && -e "$to" ]]; then rm -rf "$to"; fi
  if [[ ! -e "$to" ]]; then
    if [[ -d "$from" ]]; then
      cp -Rc "$from" "$to" 2>/dev/null || cp -R "$from" "$to"
      echo "bundle-gpui-app: copied $from -> $to"
    else
      mkdir -p "$to"
      echo "bundle-gpui-app: no $from; starting $to empty"
    fi
  fi
}
copy "$src_data" "$data"
copy "$src_config" "$config"
mkdir -p "$config/capture-index-keys" "$config/license-tokens"
# The copied settings still name the source save directory, and app-infra opens
# the persisted `saveDirectory` over MNEMA_SAVE_DIRECTORY: point it at the copy.
python3 - "$config/recording-settings.json" "$data" <<'PY'
import json, sys
path, data = sys.argv[1], sys.argv[2]
try:
    with open(path) as f:
        settings = json.load(f)
except FileNotFoundError:
    sys.exit(0)
if settings.get("saveDirectory") != data:
    settings["saveDirectory"] = data
    with open(path, "w") as f:
        json.dump(settings, f, indent=2)
    print(f"bundle-gpui-app: saveDirectory -> {data}")
PY

cd "$repo"
unset RUSTUP_TOOLCHAIN # rust-toolchain.toml pins the toolchain
# shellcheck source=/dev/null
. "$repo/scripts/openblas-build-env.sh"
# Debug: MNEMA_DEV_MASTER_KEY_FILE is compiled out of release builds.
cargo build -p mnema-gpui --bin mnema-gpui

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/debug/mnema-gpui" "$app/Contents/MacOS/mnema-gpui"
cp "$repo/apps/desktop/src-tauri/icons/icon.icns" "$app/Contents/Resources/icon.icns"

plist="$app/Contents/Info.plist"
cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>${bundle_id}</string>
  <key>CFBundleName</key><string>Mnema GPUI Dev</string>
  <key>CFBundleDisplayName</key><string>Mnema GPUI Dev</string>
  <key>CFBundleExecutable</key><string>mnema-gpui</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>LSArchitecturePriority</key><array><string>arm64</string></array>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>${bundle_id}</string>
      <key>CFBundleURLSchemes</key><array><string>${scheme}</string></array>
    </dict>
  </array>
  <key>NSMicrophoneUsageDescription</key>
  <string>Microphone access is required to capture audio input during recording.</string>
  <key>NSAudioCaptureUsageDescription</key>
  <string>System audio access is required to capture sound from meetings, videos, and apps during recording.</string>
  <key>NSSpeechRecognitionUsageDescription</key>
  <string>Speech recognition access is required to transcribe microphone audio with Apple Speech.</string>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Local network access is required to connect to AI model servers running on your local network.</string>
  <key>LSEnvironment</key>
  <dict>
    <key>MNEMA_SAVE_DIRECTORY</key><string>${data}</string>
    <key>MNEMA_APP_CONFIG_DIR</key><string>${config}</string>
    <key>MNEMA_DEV_MASTER_KEY_FILE</key><string>${data}/dev-master-key</string>
    <key>MNEMA_CAPTURE_INDEX_KEY_DIR</key><string>${config}/capture-index-keys</string>
    <key>MNEMA_LICENSE_TOKEN_DIR</key><string>${config}/license-tokens</string>
    <key>MNEMA_TRIPWIRE_NO_KEYCHAIN</key><string>1</string>
  </dict>
</dict>
</plist>
PLIST
plutil -lint "$plist" >/dev/null

# Ad-hoc: no identity, no keychain access.
codesign --force --sign - "$app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
echo "bundle-gpui-app: built $app"
echo "bundle-gpui-app: data $data"
echo "bundle-gpui-app: config $config"
echo "bundle-gpui-app: launch with: open \"$app\""
