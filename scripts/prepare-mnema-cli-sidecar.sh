#!/usr/bin/env bash
set -euo pipefail

profile="${1:-debug}"
case "$profile" in
  debug | release) ;;
  *)
    echo "usage: $0 [debug|release]" >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_triple="${CARGO_BUILD_TARGET:-${TAURI_TARGET_TRIPLE:-${TARGET:-}}}"
if [[ -z "$target_triple" ]]; then
  target_triple="$(rustc -vV | awk '/^host:/ { print $2 }')"
fi
if [[ -z "$target_triple" ]]; then
  echo "failed to resolve Rust target triple" >&2
  exit 1
fi

case "$target_triple" in
  *windows*) exe_suffix=".exe" ;;
  *) exe_suffix="" ;;
esac

cargo_args=(
  build
  --manifest-path "$repo_root/Cargo.toml"
  -p app-infra
  --bin mnema-cli
  --target "$target_triple"
)
if [[ "$profile" == "release" ]]; then
  cargo_args+=(--release)
fi

cargo "${cargo_args[@]}"

source_path="$repo_root/target/$target_triple/$profile/mnema-cli$exe_suffix"
output_dir="$repo_root/apps/desktop/src-tauri/binaries"
output_path="$output_dir/mnema-cli-$target_triple$exe_suffix"

mkdir -p "$output_dir"
cp "$source_path" "$output_path"
chmod 755 "$output_path"

echo "prepared $output_path"
