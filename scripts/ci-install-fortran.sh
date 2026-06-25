#!/usr/bin/env bash
# CI-only: install Homebrew's gcc and make its Fortran toolchain usable for the
# from-source OpenBLAS build (speakrs' `openblas-static` feature) and the desktop
# build.rs static-Fortran force-link.
#
# Why this isn't just `brew install gcc` + call `gfortran`: on the hosted macOS
# runners the gcc keg can expose ONLY a versioned binary (e.g. `gfortran-15`)
# with no unversioned `gfortran` on PATH (an "already installed and up-to-date"
# keg is not re-linked). Every consumer that hardcodes the name then dies with
# `gfortran: command not found`:
#   - OpenBLAS's Makefile invokes the literal `gfortran` for the native build,
#   - apps/desktop/src-tauri/build.rs defaults to `gfortran` to force-link the
#     static Fortran runtime,
#   - the LIBRARY_PATH probe below.
# The previous inline step also swallowed the failure: `gfortran` failing inside
# a command substitution left `dirname` with empty input, so it silently wrote
# `LIBRARY_PATH=.` and the build only blew up later in build.rs.
#
# This script discovers a real binary, shims an unversioned `gfortran` onto PATH,
# points FC/OPENBLAS_FC at it, and adds its lib dir to LIBRARY_PATH (OpenBLAS's
# `make all` links its test programs with `-lgfortran`).
#
# It persists to $GITHUB_ENV / $GITHUB_PATH, so run it as a workflow step (NOT
# sourced). See AGENTS.md / CLAUDE.md for the broader OpenBLAS build story.
set -euo pipefail

brew install gcc

# Prefer the gcc keg's own bin (reliable even when the keg isn't linked into
# /opt/homebrew/bin), then any versioned gfortran there, then PATH.
gcc_bin="$(brew --prefix gcc)/bin"
fc=""
for cand in "$gcc_bin/gfortran" "$gcc_bin"/gfortran-* gfortran; do
  if resolved="$(command -v "$cand" 2>/dev/null)"; then
    fc="$resolved"
    break
  fi
done
if [ -z "$fc" ]; then
  echo "::error::gfortran not found after 'brew install gcc' (searched ${gcc_bin} and PATH)" >&2
  exit 1
fi
echo "Fortran compiler: ${fc}"

# 1) Unversioned `gfortran` on PATH for tools that invoke it by literal name
#    (OpenBLAS's native make, build.rs default).
shim_dir="${RUNNER_TEMP:-/tmp}/fortran-shim"
mkdir -p "${shim_dir}"
ln -sf "${fc}" "${shim_dir}/gfortran"
echo "${shim_dir}" >> "${GITHUB_PATH}"

# 2) Point this repo's build.rs and openblas-src at the resolved compiler
#    explicitly, independent of PATH resolution.
{
  echo "FC=${fc}"
  echo "OPENBLAS_FC=${fc}"
} >> "${GITHUB_ENV}"

# 3) gfortran's lib dir on the linker search path so OpenBLAS's from-source
#    `make all` can link its `-lgfortran` test programs.
gfdir="$(dirname "$("${fc}" -print-file-name=libgfortran.dylib)")"
echo "LIBRARY_PATH=${gfdir}${LIBRARY_PATH:+:${LIBRARY_PATH}}" >> "${GITHUB_ENV}"
