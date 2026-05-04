#!/bin/zsh

set -euo pipefail

if [[ "${OSTYPE}" != darwin* ]]; then
  print -u2 "This script only runs on macOS."
  exit 1
fi

identity="${APPLE_SIGNING_IDENTITY:-}"

if [[ -z "${identity}" ]]; then
  identity="$(security find-identity -v -p codesigning | grep 'Apple Development' | head -n 1 | sed -E 's/.*\"(.*)\"/\1/' || true)"
fi

if [[ -z "${identity}" ]]; then
  print -u2 "No Apple Development signing identity found."
  print -u2 "Create one in Xcode: Settings > Accounts > Apple ID > Manage Certificates > + > Apple Development."
  print -u2 "Then re-run this script, or set APPLE_SIGNING_IDENTITY explicitly."
  exit 1
fi

print "Using signing identity: ${identity}"
script_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

cd "${repo_root}/apps/desktop"
APPLE_SIGNING_IDENTITY="${identity}" bun run tauri -- build
