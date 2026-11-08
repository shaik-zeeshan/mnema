#!/usr/bin/env bash
# bash scripts/dev-gpui.sh [--fresh] [extra cargo run args]
#
# Runs the GPUI app (`mnema-gpui`, identity day.mnema.gpui-dev) on a COPY of the
# Tauri dev sandbox, never on ~/.mnema-dev itself: startup retention deletes data,
# and an S1 smoke run against the real sandbox lost 37 segments that way.
#
#   data:   ~/.mnema-dev                                   -> ~/.mnema-gpui-dev
#   config: ~/Library/Application Support/day.mnema.dev    -> .../day.mnema.gpui-dev
#
# The copies are reused on later runs; --fresh re-copies them (APFS clones, so cheap).
# Built with `feel-metrics`: startup milestones and frame times go to
# ~/Library/Logs/day.mnema.gpui-dev/rust.log; scripts/gpui-feel-report.sh reads them.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
src_data="${HOME}/.mnema-dev"
src_config="${HOME}/Library/Application Support/day.mnema.dev"
data="${HOME}/.mnema-gpui-dev"
config="${HOME}/Library/Application Support/day.mnema.gpui-dev"

fresh=0
if [[ "${1:-}" == "--fresh" ]]; then
  fresh=1
  shift
fi

copy() { # copy <from> <to>: clone `from` into `to` unless `to` exists (or --fresh)
  local from="$1" to="$2"
  [[ "$(cd "$(dirname "$to")" 2>/dev/null && pwd)/$(basename "$to")" != "$from" ]] ||
    { echo "dev-gpui: refusing to run on the original $from" >&2; exit 1; }
  if [[ $fresh -eq 1 && -e "$to" ]]; then rm -rf "$to"; fi
  if [[ ! -e "$to" ]]; then
    if [[ -d "$from" ]]; then
      cp -Rc "$from" "$to" 2>/dev/null || cp -R "$from" "$to"
      echo "dev-gpui: copied $from -> $to"
    else
      mkdir -p "$to"
      echo "dev-gpui: no $from; starting $to empty"
    fi
  fi
}
copy "$src_data" "$data"
copy "$src_config" "$config"

# Same layout dev-app.sh gives the Tauri sandbox, pointed at the copies.
export MNEMA_SAVE_DIRECTORY="$data"
export MNEMA_APP_CONFIG_DIR="$config"
export MNEMA_DEV_MASTER_KEY_FILE="$data/dev-master-key"
export MNEMA_LICENSE_TOKEN_DIR="$config/license-tokens"
export MNEMA_CAPTURE_INDEX_KEY_DIR="$config/capture-index-keys"
mkdir -p "$MNEMA_CAPTURE_INDEX_KEY_DIR"
echo "dev-gpui: data $MNEMA_SAVE_DIRECTORY"
echo "dev-gpui: config $MNEMA_APP_CONFIG_DIR"

cd "$repo"
unset RUSTUP_TOOLCHAIN # rust-toolchain.toml pins the toolchain
# shellcheck source=/dev/null
. "$repo/scripts/openblas-build-env.sh"
exec cargo run -p mnema-gpui --features feel-metrics "$@"
