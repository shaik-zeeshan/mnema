#!/usr/bin/env bash
# bash scripts/bundle-gpui-app.sh --dev [--fresh]
# bash scripts/bundle-gpui-app.sh --release [--notarize] [--version X.Y.Z]
#
# Builds the GPUI app (`mnema-gpui`) and assembles a bundle under
# ${CARGO_TARGET_DIR:-target}/gpui-bundle/. Launch it with `open` (LaunchServices),
# never by exec'ing the binary: a shell-exec'd app makes the terminal the TCC
# responsible process.
#
#   --dev      debug build, "Mnema GPUI Dev.app", day.mnema.gpui-dev, scheme
#              mnema-gpui-dev, on a COPY of the Tauri dev sandbox (below).
#   --release  release build (OPENBLAS_DYNAMIC_ARCH=1), "mnema.app", day.mnema,
#              scheme mnema: the shipped identity, so it runs on ~/.mnema (the
#              same data, keychain and /Applications slot as the Tauri prod app),
#              plus mnema_<version>_aarch64.dmg to install from.
#              A signed bundle (either mode) also gets <name>.app.tar.gz, the
#              updater archive.
#   --notarize notarytool submit + staple (release only): APPLE_ID, APPLE_PASSWORD
#              (app-specific), team from APPLE_TEAM_ID or the identity's "(TEAMID)".
#   --version  CFBundleShortVersionString; default tauri.conf.json's version. A
#              higher one builds a test update.
#
# Signing identity: APPLE_SIGNING_IDENTITY (never hard-coded). Set, the bundle goes
# through scripts/entitle-macos-app.sh: Helpers/mnema-cli.app + the
# Contents/MacOS/mnema-cli symlink, the provisioning profile, entitlements,
# hardened runtime. Unset, --dev signs ad-hoc (no keychain access) and --release
# refuses. TCC flows need a signed build; an ad-hoc signature changes every rebuild.
#
# A test update feed: build with MNEMA_UPDATE_FEED_URL=http://127.0.0.1:8000/latest.json
# (baked into app-core at compile time; release CI never sets it).
#
# Dev data: the bundle runs on a copy, never ~/.mnema-dev itself (startup
# retention deletes data), with file-backed keys and no keychain:
#   data   ~/.mnema-dev                                -> ~/.mnema-gpui-dev
#   config ~/Library/Application Support/day.mnema.dev -> .../day.mnema.gpui-dev
# Those paths ride in LSEnvironment because `open` passes no env. --fresh re-copies.
set -euo pipefail

mode=""
fresh=0
notarize=0
version=""
while (($#)); do
  case "$1" in
    --dev) mode=dev ;;
    --release) mode=release ;;
    --fresh) fresh=1 ;;
    --notarize) notarize=1 ;;
    --version) version="${2:?--version needs a value}"; shift ;;
    *) echo "bundle-gpui-app: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
usage="usage: bundle-gpui-app.sh --dev [--fresh] | --release [--notarize] [--version X.Y.Z]"
[[ -n "$mode" ]] || { echo "$usage" >&2; exit 2; }
[[ $notarize -eq 0 || "$mode" == release ]] || { echo "bundle-gpui-app: --notarize needs --release" >&2; exit 2; }

identity="${APPLE_SIGNING_IDENTITY:-}"
if [[ "$mode" == release && -z "$identity" ]]; then
  echo "bundle-gpui-app: --release needs a signing identity. Set APPLE_SIGNING_IDENTITY to one from" >&2
  echo "  security find-identity -v -p codesigning   (Developer ID Application for a build you hand out)" >&2
  exit 1
fi

repo="$(cd "$(dirname "$0")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"
tauri_dir="$repo/apps/desktop/src-tauri"
[[ -n "$version" ]] || version="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$tauri_dir/tauri.conf.json")"

if [[ "$mode" == dev ]]; then
  bundle_id="day.mnema.gpui-dev"; scheme="mnema-gpui-dev"; name="Mnema GPUI Dev"; profile=debug
else
  # "mnema", lowercase: the shipped Tauri app's bundle and display name (productName),
  # so the Dock tile, the /Applications slot and the release asset names stay the same.
  bundle_id="day.mnema"; scheme="mnema"; name="mnema"; profile=release
fi
app="$target/gpui-bundle/$name.app"

env_plist=""
if [[ "$mode" == dev ]]; then
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
  env_plist="  <key>LSEnvironment</key>
  <dict>
    <key>MNEMA_SAVE_DIRECTORY</key><string>${data}</string>
    <key>MNEMA_APP_CONFIG_DIR</key><string>${config}</string>
    <key>MNEMA_DEV_MASTER_KEY_FILE</key><string>${data}/dev-master-key</string>
    <key>MNEMA_CAPTURE_INDEX_KEY_DIR</key><string>${config}/capture-index-keys</string>
    <key>MNEMA_LICENSE_TOKEN_DIR</key><string>${config}/license-tokens</string>
    <key>MNEMA_TRIPWIRE_NO_KEYCHAIN</key><string>1</string>
  </dict>"
fi

cd "$repo"
unset RUSTUP_TOOLCHAIN # rust-toolchain.toml pins the toolchain
# shellcheck source=/dev/null
. "$repo/scripts/openblas-build-env.sh"
if [[ "$mode" == dev ]]; then
  # Debug: MNEMA_DEV_MASTER_KEY_FILE is compiled out of release builds. Optimised, or
  # most screens drop frames (see scripts/dev-gpui.sh).
  cargo build --config 'profile.dev.opt-level=3' -p mnema-gpui --bin mnema-gpui
else
  # A build you hand out runs on every Apple Silicon generation (AGENTS.md).
  export OPENBLAS_DYNAMIC_ARCH=1
  cargo build --release -p mnema-gpui --bin mnema-gpui
  "$repo/scripts/generate-third-party-licenses.sh"
fi
bash "$repo/scripts/prepare-mnema-cli-sidecar.sh" "$profile"
host_triple="$(rustc -vV | awk '/^host:/ { print $2 }')"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/$profile/mnema-gpui" "$app/Contents/MacOS/mnema-gpui"
cp "$tauri_dir/binaries/mnema-cli-$host_triple" "$app/Contents/MacOS/mnema-cli"
cp "$tauri_dir/icons/icon.icns" "$app/Contents/Resources/icon.icns"
licenses="$repo/apps/desktop/THIRD_PARTY_LICENSES.md"
if [[ -f "$licenses" ]]; then
  cp "$licenses" "$app/Contents/Resources/THIRD_PARTY_LICENSES.md"
else
  echo "bundle-gpui-app: no $licenses (scripts/generate-third-party-licenses.sh makes it); skipped" >&2
fi

plist="$app/Contents/Info.plist"
cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>${bundle_id}</string>
  <key>CFBundleName</key><string>${name}</string>
  <key>CFBundleDisplayName</key><string>${name}</string>
  <key>CFBundleExecutable</key><string>mnema-gpui</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
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
${env_plist}
</dict>
</plist>
PLIST
plutil -lint "$plist" >/dev/null

if [[ -n "$identity" ]]; then
  bash "$repo/scripts/entitle-macos-app.sh" "$app" "$identity"
else
  # Ad-hoc, dev only: no identity, no keychain access, no install of updates.
  codesign --force --deep --sign - "$app"
fi

if [[ $notarize -eq 1 ]]; then
  : "${APPLE_ID:?--notarize needs APPLE_ID}" "${APPLE_PASSWORD:?--notarize needs APPLE_PASSWORD (app-specific)}"
  team="${APPLE_TEAM_ID:-$(sed -nE 's/.*\(([A-Z0-9]{10})\)$/\1/p' <<<"$identity")}"
  [[ -n "$team" ]] || { echo "bundle-gpui-app: set APPLE_TEAM_ID (the identity names no team)" >&2; exit 1; }
  creds=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$team")
  zip="$target/gpui-bundle/$name-notarize.zip"
  ditto -c -k --keepParent "$app" "$zip"
  id="$(xcrun notarytool submit "$zip" "${creds[@]}" --output-format json |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
  # Poll, not `submit --wait`: one network blip in the long wait killed CI runs.
  status=""
  for _ in $(seq 1 180); do
    status="$(xcrun notarytool info "$id" "${creds[@]}" --output-format json 2>/dev/null |
      python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' 2>/dev/null || true)"
    case "$status" in
      Accepted) break ;;
      Invalid | Rejected) xcrun notarytool log "$id" "${creds[@]}" || true; exit 1 ;;
      *) sleep 60 ;;
    esac
  done
  [[ "$status" == Accepted ]] || { echo "bundle-gpui-app: notarization not Accepted in 3 h (last: '$status')" >&2; exit 1; }
  xcrun stapler staple "$app"
  rm -f "$zip"
fi

if [[ -n "$identity" ]]; then
  # The updater archive (only a signed bundle can pass the install gate). No
  # AppleDouble `._*` files or xattr headers: written into the bundle on unpack
  # they'd break its seal.
  COPYFILE_DISABLE=1 tar --no-mac-metadata --no-xattrs -czf "$target/gpui-bundle/$name.app.tar.gz" \
    -C "$target/gpui-bundle" "$name.app"
  echo "bundle-gpui-app: archive $target/gpui-bundle/$name.app.tar.gz"
fi

if [[ "$mode" == release ]]; then # --release always has an identity (checked above)
  # The install disk image: the app (ditto keeps its signature and symlinks) plus an
  # Applications link to drag it onto.
  dmg="$target/gpui-bundle/${name}_${version}_aarch64.dmg"
  staging="$(mktemp -d "$target/gpui-bundle/dmg.XXXXXX")"
  trap 'rm -rf "$staging"' EXIT
  ditto "$app" "$staging/$name.app"
  ln -s /Applications "$staging/Applications"
  hdiutil create -volname "$name" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null
  rm -rf "$staging"
  trap - EXIT
  codesign --force --sign "$identity" "$dmg"
  echo "bundle-gpui-app: dmg $dmg"
fi

/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
echo "bundle-gpui-app: built $app ($bundle_id $version, $([[ -n "$identity" ]] && echo signed || echo ad-hoc))"
if [[ "$mode" == dev ]]; then
  echo "bundle-gpui-app: data $data"
  echo "bundle-gpui-app: config $config"
fi
echo "bundle-gpui-app: launch with: open \"$app\""
