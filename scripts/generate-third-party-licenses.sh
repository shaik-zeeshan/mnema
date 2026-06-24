#!/usr/bin/env bash
# Regenerate apps/desktop/THIRD_PARTY_LICENSES.md from scratch.
#
# Merges three sources into the shipped attribution file:
#   1. cargo-about    — Rust crates compiled into the desktop binary
#   2. license-checker-rseidelsohn — production JS/frontend dependency closure
#   3. assemble-third-party-licenses.py — merges (1) + (2) into the final doc
#
# Neither cargo-about nor license-checker compiles anything or needs the
# mnema-cli sidecar / OpenBLAS toolchain — both read manifests + Cargo.lock only.
# Run from anywhere; repo root is resolved from this script's location.

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
src_tauri="${repo_root}/apps/desktop/src-tauri"
desktop="${repo_root}/apps/desktop"
out="${desktop}/THIRD_PARTY_LICENSES.md"

# Work in a private temp dir that is always cleaned up.
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/mnema-licenses.XXXXXX")"
cleanup() { rm -rf "${tmp_dir}"; }
trap cleanup EXIT

# 1. Ensure cargo-about is available (guarded so re-runs are fast).
if ! cargo about --version >/dev/null 2>&1; then
  echo "==> cargo-about not found; installing (cargo install cargo-about --locked)…"
  cargo install cargo-about --locked
fi

# 2. Rust dependency licenses.
echo "==> Generating Rust licenses with cargo-about…"
( cd "${src_tauri}" && cargo about generate about.hbs --frozen ) > "${tmp_dir}/rust-licenses.txt"

# 3. JavaScript / frontend production dependency licenses.
echo "==> Generating JS licenses with license-checker-rseidelsohn…"
( cd "${desktop}" && bunx license-checker-rseidelsohn --production --json ) > "${tmp_dir}/js-licenses.json"

# 4. Merge into the shipped attribution file.
echo "==> Assembling ${out}…"
python3 "${script_dir}/assemble-third-party-licenses.py" \
  --rust-in "${tmp_dir}/rust-licenses.txt" \
  --js-in "${tmp_dir}/js-licenses.json" \
  --out "${out}"

lines="$(wc -l < "${out}" | tr -d ' ')"
echo "==> Wrote ${out} (${lines} lines)"
