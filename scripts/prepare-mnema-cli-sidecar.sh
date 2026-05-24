#!/usr/bin/env bash
set -euo pipefail

profile="${1:-debug}"
if [[ $# -gt 0 ]]; then
  shift
fi
case "$profile" in
  debug | release) ;;
  *)
    echo "usage: $0 [debug|release] [--locked]" >&2
    exit 2
    ;;
esac

cargo_locked=false
for arg in "$@"; do
  case "$arg" in
    --locked) cargo_locked=true ;;
    *)
      echo "usage: $0 [debug|release] [--locked]" >&2
      exit 2
      ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_triple="${CARGO_BUILD_TARGET:-${TAURI_ENV_TARGET_TRIPLE:-${TARGET:-}}}"
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

output_dir="$repo_root/apps/desktop/src-tauri/binaries"
output_path="$output_dir/mnema-cli-$target_triple$exe_suffix"

mkdir -p "$output_dir"

sidecar_output_path() {
  local rust_target="$1"
  echo "$output_dir/mnema-cli-$rust_target$exe_suffix"
}

build_target() {
  local rust_target="$1"
  local cargo_args=(
    build
    --manifest-path "$repo_root/Cargo.toml"
    -p cli
    --bin mnema-cli
    --target "$rust_target"
  )
  if [[ "$cargo_locked" == true ]]; then
    cargo_args+=(--locked)
  fi
  if [[ "$profile" == "release" ]]; then
    cargo_args+=(--release)
  fi

  cargo "${cargo_args[@]}"
}

if [[ "$target_triple" == "universal-apple-darwin" ]]; then
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "universal-apple-darwin sidecar builds require macOS" >&2
    exit 1
  fi
  if ! command -v lipo >/dev/null 2>&1; then
    echo "universal-apple-darwin sidecar builds require lipo in PATH" >&2
    exit 1
  fi

  build_target "aarch64-apple-darwin"
  build_target "x86_64-apple-darwin"

  arm_source_path="$repo_root/target/aarch64-apple-darwin/$profile/mnema-cli"
  intel_source_path="$repo_root/target/x86_64-apple-darwin/$profile/mnema-cli"
  arm_output_path="$(sidecar_output_path "aarch64-apple-darwin")"
  intel_output_path="$(sidecar_output_path "x86_64-apple-darwin")"

  cp "$arm_source_path" "$arm_output_path"
  cp "$intel_source_path" "$intel_output_path"
  lipo -create -output "$output_path" "$arm_source_path" "$intel_source_path"
  chmod 755 "$arm_output_path" "$intel_output_path"
else
  build_target "$target_triple"

  source_path="$repo_root/target/$target_triple/$profile/mnema-cli$exe_suffix"
  cp "$source_path" "$output_path"
fi
chmod 755 "$output_path"

echo "prepared $output_path"
