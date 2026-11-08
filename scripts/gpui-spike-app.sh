#!/usr/bin/env bash
# S0 spike only: wrap the built `spike` binary in a scratch, ad-hoc-signed .app that
# registers the `mnema-gpui-dev://` URL scheme. Not a release bundle (S16 builds that).
# Usage: bash scripts/gpui-spike-app.sh [SPIKE_LOG path baked into LSEnvironment]
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
bin="$target_dir/debug/spike"
app="$repo_root/target/gpui-spike/MnemaSpike.app"
log_path="${1:-${SPIKE_LOG:-}}"

[ -x "$bin" ] || { echo "missing $bin: run cargo build -p mnema-gpui --bin spike" >&2; exit 1; }

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$bin" "$app/Contents/MacOS/MnemaSpike"

env_block=""
if [ -n "$log_path" ]; then
  env_block="<key>LSEnvironment</key><dict><key>SPIKE_LOG</key><string>$log_path</string><key>SPIKE_SHARING</key><string>readonly</string></dict>"
fi

cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>MnemaSpike</string>
  <key>CFBundleIdentifier</key><string>day.mnema.gpui-spike</string>
  <key>CFBundleName</key><string>MnemaSpike</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.0</string>
  <key>CFBundleVersion</key><string>0</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>day.mnema.gpui-spike</string>
      <key>CFBundleURLSchemes</key><array><string>mnema-gpui-dev</string></array>
    </dict>
  </array>
  $env_block
</dict>
</plist>
PLIST

codesign --force --sign - "$app"
# Register with LaunchServices so `open mnema-gpui-dev://...` resolves to this bundle.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
echo "$app"
