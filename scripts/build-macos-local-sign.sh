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
dmg_dir="${repo_root}/target/release/bundle/dmg"

# speakrs links OpenBLAS via the `openblas-static` feature, built from source.
# Two build-time requirements (see AGENTS.md):
#   - a Fortran toolchain must be present, and the gcc lib dir on the linker
#     search path (LIBRARY_PATH) so OpenBLAS's own test programs link;
#   - OPENBLAS_DYNAMIC_ARCH=1 so this signed build (which you hand to others)
#     runs on every Apple Silicon generation, not just this machine's core.
if ! command -v gfortran >/dev/null 2>&1; then
  print -u2 "gfortran not found — install the Fortran toolchain: brew install gcc"
  exit 1
fi
gfortran_libdir="$(dirname "$(gfortran -print-file-name=libgfortran.dylib)")"
export LIBRARY_PATH="${gfortran_libdir}${LIBRARY_PATH:+:${LIBRARY_PATH}}"
export OPENBLAS_DYNAMIC_ARCH=1
print "OpenBLAS: static + DYNAMIC_ARCH; LIBRARY_PATH includes ${gfortran_libdir}"

cd "${repo_root}/apps/desktop"
CI=true APPLE_SIGNING_IDENTITY="${identity}" bun run tauri -- build

dmg_path="$(ls -t "${dmg_dir}"/*.dmg 2>/dev/null | head -n 1 || true)"

if [[ -n "${dmg_path}" ]]; then
  print "Opening DMG: ${dmg_path}"
  open "${dmg_path}"
else
  print -u2 "Build succeeded, but no DMG was found in ${dmg_dir}."
  exit 1
fi
